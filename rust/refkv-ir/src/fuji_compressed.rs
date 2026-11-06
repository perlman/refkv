//! The `refkv.fuji_compressed` codec (spec/conventions.md §13
//! <https://github.com/d-v-b/refkv/blob/main/spec/conventions.md#13-fujifilm-compressed-stripes>):
//! decodes one stripe of a Fujifilm RAF file's compressed sensor data, 768 columns over the
//! chunk's height, lossless or lossy, Bayer or X-Trans, into its `uint16` samples. Written from
//! the section alone, step by step: its step numbers are the comments' "step n". One
//! implementation serves Python and the browser.

use serde_json::Value as J;

type Res<T> = Result<T, String>;

fn err<T>(m: impl std::fmt::Display) -> Res<T> {
    Err(format!("refkv.fuji_compressed: {m}"))
}

/// A stripe's columns.
pub const WIDTH: usize = 768;
/// The most rows a chunk may have.
pub const MAX_HEIGHT: u64 = 16_386;

/// The codec's configuration (the array's `codecs` entry).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// the samples' bits: 12, 14 or 16
    pub bits: u32,
    pub lossless: bool,
    /// the color of each position of the pattern, 0 red, 1 green, 2 blue: 2 × 2 (Bayer) or 6 × 6
    /// (X-Trans)
    pub cfa: Vec<Vec<u8>>,
}

impl Config {
    /// The configuration object: `bits`, `lossless` and `cfa`, all required, and no other member.
    pub fn from_json(v: &J) -> Res<Config> {
        let Some(o) = v.as_object() else {
            return err("the configuration is not an object");
        };
        if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "bits" | "lossless" | "cfa")) {
            return err(format!("the configuration has an unknown member {k:?}"));
        }
        let bits = match o.get("bits") {
            None => return err("the configuration has no bits"),
            Some(b) => match b.as_u64() {
                Some(b @ (12 | 14 | 16)) => b as u32,
                _ => return err(format!("the configuration's bits is {b}, not 12, 14 or 16")),
            },
        };
        let lossless = match o.get("lossless") {
            None => return err("the configuration has no lossless"),
            Some(l) => match l.as_bool() {
                Some(l) => l,
                None => return err(format!("the configuration's lossless is {l}, not a boolean")),
            },
        };
        let Some(c) = o.get("cfa") else {
            return err("the configuration has no cfa");
        };
        let bad = || err(format!("the configuration's cfa is {c}, not 2 rows of 2 or 6 rows of 6 colors 0, 1 or 2"));
        let Some(rows) = c.as_array() else {
            return bad();
        };
        if rows.len() != 2 && rows.len() != 6 {
            return bad();
        }
        let mut cfa = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(row) = row.as_array().filter(|r| r.len() == rows.len()) else {
                return bad();
            };
            let mut colors = Vec::with_capacity(row.len());
            for x in row {
                match x.as_u64() {
                    Some(x @ 0..=2) => colors.push(x as u8),
                    _ => return bad(),
                }
            }
            cfa.push(colors);
        }
        Ok(Config { bits, lossless, cfa })
    }

    fn xtrans(&self) -> bool {
        self.cfa.len() == 6
    }
}

// ---- step 2: bits

/// The coded bytes as bits, most significant first; the bits past the end read as 0, up to but
/// not including the last bit of the byte after the end.
struct Bits<'a> {
    b: &'a [u8],
    /// the next bit
    pos: usize,
}

impl Bits<'_> {
    /// The first bit a decoding may not read: the 8th of the byte after the end.
    fn limit(&self) -> usize {
        self.b.len() * 8 + 7
    }

    fn past() -> String {
        "the coded bytes end before the stripe's last sample".into()
    }

    /// `zeros()`: the 0s before the next 1, reading it too.
    fn zeros(&mut self) -> Res<u32> {
        let mut n = 0u32;
        // a whole byte at a time; past the end every bit is 0, so no 1 is found there
        while let Some(&byte) = self.b.get(self.pos >> 3) {
            let shift = (self.pos & 7) as u32;
            let rest = byte << shift;
            if rest == 0 {
                n += 8 - shift;
                self.pos = (self.pos | 7) + 1;
            } else {
                let lz = rest.leading_zeros();
                n += lz;
                self.pos += lz as usize + 1;
                return Ok(n);
            }
        }
        Err(Self::past())
    }

    /// `read(k)`, for `k` at most 16.
    fn read(&mut self, k: u32) -> Res<u32> {
        if k == 0 {
            return Ok(0);
        }
        if self.pos + k as usize > self.limit() {
            return Err(Self::past());
        }
        let mut v = 0u32;
        for _ in 0..k {
            let bit = self.b.get(self.pos >> 3).map_or(0, |&byte| (byte >> (7 - (self.pos & 7))) & 1);
            v = (v << 1) | bit as u32;
            self.pos += 1;
        }
        Ok(v)
    }
}

// ---- step 3: quantization tables

/// A quantization table.
#[derive(Clone, Copy, Debug)]
struct Table {
    q: i32,
    /// the thresholds p0–p3 (p4, the largest sample, is never compared)
    p: [i32; 4],
    m: i32,
    t: i32,
    /// raw bits, `L(T)`
    r: u32,
    /// the gradient limit of an extra table
    g: i32,
}

/// `L(x)`: the binary digits of `x − 1`, at least 1.
fn digits(x: i32) -> u32 {
    (32 - ((x - 1).max(0) as u32).leading_zeros()).max(1)
}

impl Table {
    fn new(q: i32, p: [i32; 4], m: i32, t: i32, g: i32) -> Table {
        Table { q, p, m, t, r: digits(t), g }
    }

    /// The main table of step `q`.
    fn main(q: i32, max: i32) -> Table {
        Table::new(q, [q, 3 * q + 18, 5 * q + 67, 7 * q + 276], 9, (max + 2 * q) / (2 * q + 1) + 1, 0)
    }

    /// The extra table `k`, 1 to 3, of a lossy stripe.
    fn extra(k: i32, max: i32) -> Table {
        let q = k - 1;
        Table::new(q, [q, 18 + 3 * q, 67 + 5 * q, 276 + 7 * q], 3, (max + 2 * q) / (2 * q + 1) + 1, 4 + k)
    }

    /// `Q(v)`.
    fn quantize(&self, v: i32) -> i32 {
        let [p0, p1, p2, p3] = self.p;
        if v <= -p3 {
            -4
        } else if v <= -p2 {
            -3
        } else if v <= -p1 {
            -2
        } else if v < -p0 {
            -1
        } else if v <= p0 {
            0
        } else if v < p1 {
            1
        } else if v < p2 {
            2
        } else if v < p3 {
            3
        } else {
            4
        }
    }

    /// A context's start (step 4).
    fn context(&self) -> Ctx {
        Ctx { a: ((self.t + 32) / 64).max(2), n: 1 }
    }
}

// ---- step 4: contexts

#[derive(Clone, Copy, Debug)]
struct Ctx {
    a: i32,
    n: i32,
}

/// A set of contexts: 41 of the main table and 5 of each extra table.
#[derive(Clone, Debug)]
struct Set {
    main: [Ctx; 41],
    extra: [[Ctx; 5]; 3],
}

// ---- step 1: lines

/// A line's color: its first line's index among the 18 and its last number.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    R,
    G,
    B,
}

impl Color {
    fn base(self) -> usize {
        match self {
            Color::R => 0,
            Color::G => 5,
            Color::B => 13,
        }
    }

    fn last(self) -> usize {
        match self {
            Color::R | Color::B => 4,
            Color::G => 7,
        }
    }
}

/// The even positions of a line a pass interpolates (step 7).
#[derive(Clone, Copy)]
enum Interp {
    None,
    All,
    /// those with `x mod 4` equal to it
    Mod4(usize),
}

impl Interp {
    fn holds(self, x: usize) -> bool {
        match self {
            Interp::None => false,
            Interp::All => true,
            Interp::Mod4(r) => x % 4 == r,
        }
    }
}

/// The six passes of a row group (step 7): first line, second line, group, and the X-Trans
/// interpolation of each line.
const PASSES: [(Color, usize, Color, usize, usize, Interp, Interp); 6] = [
    (Color::R, 2, Color::G, 2, 0, Interp::All, Interp::None),
    (Color::G, 3, Color::B, 2, 1, Interp::None, Interp::All),
    (Color::R, 3, Color::G, 4, 2, Interp::Mod4(0), Interp::All),
    (Color::G, 5, Color::B, 3, 0, Interp::None, Interp::Mod4(2)),
    (Color::R, 4, Color::G, 6, 1, Interp::Mod4(2), Interp::None),
    (Color::G, 7, Color::B, 4, 2, Interp::All, Interp::Mod4(0)),
];

/// The decoder's state over a chunk.
struct Stripe<'a> {
    bits: Bits<'a>,
    /// the largest sample, `M`
    max: i32,
    /// the bits, `b`
    b: i32,
    lossless: bool,
    xtrans: bool,
    /// a line's samples, `W`
    w: usize,
    /// the 18 lines of `W + 2` entries: R0–R4, G0–G7, B0–B4
    lines: Vec<i32>,
    main: Table,
    extra: [Table; 3],
    /// the sets of contexts, by group then parity (even, odd)
    sets: [[Set; 2]; 3],
}

impl Stripe<'_> {
    fn at(&self, c: Color, i: usize) -> usize {
        (c.base() + i) * (self.w + 2)
    }

    /// Starts the main table's contexts of every set with the table of step `q`.
    fn start_main(&mut self, q: i32) {
        self.main = Table::main(q, self.max);
        let ctx = self.main.context();
        for group in &mut self.sets {
            for set in group {
                set.main = [ctx; 41];
            }
        }
    }

    /// The sample at position `x` of the line `c i` (step 5, or step 6 when `interp`).
    fn sample(&mut self, c: Color, i: usize, x: usize, odd: bool, group: usize, interp: bool) -> Res<()> {
        let e = x + 1;
        let line = self.at(c, i);
        let up = self.at(c, i - 1);
        let up2 = self.at(c, i - 2);
        let l = &self.lines;
        let (b, cc, d) = (l[up + e], l[up + e - 1], l[up + e + 1]);
        let (f, a, g2) = if odd { (0, l[line + e - 1], l[line + e + 1]) } else { (l[up2 + e], 0, 0) };
        // the prediction (5.3)
        let p = if odd {
            if (b > cc && b > d) || (b < cc && b < d) { (g2 + a + 2 * b) >> 2 } else { (a + g2) >> 1 }
        } else {
            let (dc, df, dd) = ((cc - b).abs(), (f - b).abs(), (d - b).abs());
            if dc > df && dc > dd {
                (f + d + 2 * b) >> 2
            } else if dd > dc && dd > df {
                (f + cc + 2 * b) >> 2
            } else {
                (d + cc + 2 * b) >> 2
            }
        };
        if interp {
            self.lines[line + e] = p;
            return Ok(());
        }
        // the table (5.1) and the gradient (5.2)
        let (v1, v2) = if odd { (b - cc, cc - a) } else { (b - f, cc - b) };
        let set = &mut self.sets[group][odd as usize];
        let mut table = self.main;
        let mut extra = None;
        if !self.lossless {
            let sum = v1.abs() + v2.abs();
            extra = (1..=self.main.q.min(3) as usize).find(|&k| self.extra[k - 1].g >= sum);
            if let Some(k) = extra {
                table = self.extra[k - 1];
            }
        }
        let grad = table.m * table.quantize(v1) + table.quantize(v2);
        let at = grad.unsigned_abs() as usize;
        let ctx = match extra {
            None => set.main.get_mut(at),
            Some(k) => set.extra[k - 1].get_mut(at),
        };
        let Some(ctx) = ctx else {
            return err(format!("a gradient of {grad} has no context"));
        };
        // the code (5.4)
        let s = self.bits.zeros().or_else(err)?;
        let code = if (s as i32) < 4 * self.b - table.r as i32 - 1 {
            let k = if ctx.n >= ctx.a { 0 } else { (1..=15).find(|&k| ctx.n << k >= ctx.a).unwrap_or(15) };
            ((s << k) | self.bits.read(k).or_else(err)?) as i32
        } else {
            self.bits.read(table.r).or_else(err)? as i32 + 1
        };
        if code >= table.t {
            return err(format!("a code of {code} is not less than its table's {} values", table.t));
        }
        // the residual (5.5)
        let res = if code % 2 == 0 { code / 2 } else { -(code + 1) / 2 };
        ctx.a += res.abs();
        if ctx.n == 64 {
            ctx.a /= 2;
            ctx.n = 32;
        }
        ctx.n += 1;
        // the sample (5.6)
        let step = 2 * table.q + 1;
        let mut v = if grad >= 0 { p + res * step } else { p - res * step };
        if v < -table.q {
            v += table.t * step;
        } else if v > table.q + self.max {
            v -= table.t * step;
        }
        self.lines[line + e] = v.clamp(0, self.max);
        Ok(())
    }

    /// Each line of `c` from number 2 to its last takes its edges from the line above (step 7).
    fn extend(&mut self, c: Color) {
        let w = self.w;
        for i in 2..=c.last() {
            let (line, up) = (self.at(c, i), self.at(c, i - 1));
            self.lines[line] = self.lines[up + 1];
            self.lines[line + w + 1] = self.lines[up + w];
        }
    }

    /// A row group's six passes (step 7) and its line copies (step 8).
    fn group(&mut self) -> Res<()> {
        let half = self.w / 2;
        for &(c1, i1, c2, i2, group, x1, x2) in &PASSES {
            let (x1, x2) = if self.xtrans { (x1, x2) } else { (Interp::None, Interp::None) };
            let (mut even, mut odd) = (0, 0);
            while even < half || odd < half {
                if even < half {
                    let x = 2 * even;
                    self.sample(c1, i1, x, false, group, x1.holds(x))?;
                    self.sample(c2, i2, x, false, group, x2.holds(x))?;
                    even += 1;
                }
                if even >= 5 && odd < half {
                    let x = 2 * odd + 1;
                    self.sample(c1, i1, x, true, group, false)?;
                    self.sample(c2, i2, x, true, group, false)?;
                    odd += 1;
                }
            }
            self.extend(c1);
            if c2 != c1 {
                self.extend(c2);
            }
        }
        let n = self.w + 2;
        for (to, from) in [(Color::R, 0, 3), (Color::R, 1, 4), (Color::G, 0, 6), (Color::G, 1, 7), (Color::B, 0, 3), (Color::B, 1, 4)].map(|(c, i, j)| (self.at(c, i), self.at(c, j))) {
            self.lines.copy_within(from..from + n, to);
        }
        Ok(())
    }

    /// Lines R2–R4, G2–G7 and B2–B4 become zero, and R2, G2 and B2 take their edges (step 9).
    fn next_group(&mut self) {
        let (w, n) = (self.w, self.w + 2);
        for c in [Color::R, Color::G, Color::B] {
            let from = self.at(c, 2);
            self.lines[from..from + n * (c.last() - 1)].fill(0);
            let up = self.at(c, 1);
            self.lines[from] = self.lines[up + 1];
            self.lines[from + w + 1] = self.lines[up + w];
        }
    }
}

/// The chunk's height `h`, from its shape: 1 along every dimension but the last two, which are
/// `h`, a multiple of 6 from 6 to 16,386, and 768.
fn height(shape: &[u64]) -> Res<usize> {
    let bad = || err(format!("the chunk shape {shape:?} is not 1 along all but its last two dimensions, a multiple of 6 from 6 to {MAX_HEIGHT} and {WIDTH}"));
    let [lead @ .., h, w] = shape else {
        return bad();
    };
    if lead.iter().any(|&d| d != 1) || *w != WIDTH as u64 || *h % 6 != 0 || !(6..=MAX_HEIGHT).contains(h) {
        return bad();
    }
    Ok(*h as usize)
}

/// Decodes a chunk: its samples in C order over `shape`.
pub fn decode(chunk: &[u8], cfg: &Config, shape: &[u64]) -> Res<Vec<u16>> {
    let h = height(shape)?;
    // a configuration made otherwise than by `from_json`
    let p = cfg.cfa.len();
    if !matches!(p, 2 | 6) || cfg.cfa.iter().any(|r| r.len() != p || r.iter().any(|&c| c > 2)) || !matches!(cfg.bits, 12 | 14 | 16) {
        return err("the configuration's bits or cfa is not one the codec decodes");
    }
    let groups = h / 6;
    let (q_base, coded) = if cfg.lossless {
        (&[][..], chunk)
    } else {
        let len = groups.div_ceil(16) * 16;
        if chunk.len() < len {
            return err(format!("the chunk is {} bytes, fewer than the {len} of its {groups} row groups' q_base", chunk.len()));
        }
        chunk.split_at(len)
    };
    let xtrans = cfg.xtrans();
    let w = if xtrans { 512 } else { 384 };
    let max = (1i32 << cfg.bits) - 1;
    let main = Table::main(0, max);
    let extra = [1, 2, 3].map(|k| Table::extra(k, max));
    let set = Set { main: [main.context(); 41], extra: extra.map(|t| [t.context(); 5]) };
    let mut s = Stripe {
        bits: Bits { b: coded, pos: 0 },
        max,
        b: cfg.bits as i32,
        lossless: cfg.lossless,
        xtrans,
        w,
        lines: vec![0; 18 * (w + 2)],
        main,
        extra,
        sets: std::array::from_fn(|_| std::array::from_fn(|_| set.clone())),
    };
    let mut out = vec![0u16; h * WIDTH];
    for j in 0..groups {
        if j > 0 {
            s.next_group();
        }
        if !cfg.lossless {
            let q = q_base.get(j).copied().unwrap_or(0) as i32;
            if j == 0 || q_base.get(j - 1).copied() != Some(q as u8) {
                s.start_main(q);
            }
        }
        s.group()?;
        // the rows (step 8)
        for t in 0..6 {
            let row = &mut out[(6 * j + t) * WIDTH..(6 * j + t + 1) * WIDTH];
            for (x, v) in row.iter_mut().enumerate() {
                let (color, i) = if xtrans {
                    (cfg.cfa[t][x % 6], 2 * (x / 3) + usize::from(x % 3 != 0))
                } else {
                    (cfg.cfa[t % 2][x % 2], x / 2)
                };
                let line = match color {
                    0 => s.at(Color::R, 2 + t / 2),
                    1 => s.at(Color::G, 2 + t),
                    _ => s.at(Color::B, 2 + t / 2),
                };
                *v = s.lines[line + i + 1] as u16;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_read_past_the_end_up_to_the_last_bit_of_the_next_byte() {
        let mut b = Bits { b: &[0b1010_0000], pos: 0 };
        assert_eq!(b.zeros(), Ok(0));
        assert_eq!(b.read(7), Ok(0b010_0000));
        assert_eq!(b.read(7), Ok(0));
        assert!(b.read(1).is_err());
        let mut b = Bits { b: &[0], pos: 0 };
        assert!(b.zeros().is_err());
    }

    #[test]
    fn digits_are_those_of_x_minus_1_at_least_1() {
        assert_eq!([1, 2, 3, 4, 5, 16384, 16385].map(digits), [1, 1, 2, 2, 3, 14, 15]);
    }

    #[test]
    fn quantizes_at_the_thresholds() {
        let t = Table::main(0, 16383);
        let q: Vec<i32> = [-276, -275, -67, -66, -18, -17, -1, 0, 1, 17, 18, 66, 67, 275, 276].iter().map(|&v| t.quantize(v)).collect();
        assert_eq!(q, [-4, -3, -3, -2, -2, -1, -1, 0, 1, 1, 2, 2, 3, 3, 4]);
    }
}
