//! A sans-IO parser of Fujifilm RAF files into the IR (spec/zarrify/raf.md §8
//! <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#8-raf-profile>, and the source
//! model of §5.3 <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#53-elements>).
//!
//! Rounds: the header and its directory; then the metadata records and the start of the CFA
//! region, a TIFF whose raw IFD (tag 0xF000) places the strip; then, when that read did not
//! reach them, the strip's first bytes, and a compressed strip's tables. The schema
//! (`schema/raf.json`) holds every check, in the profile's order. The IR is the header, the
//! embedded JPEG (described, never read), the metadata records, the CFA region's TIFF, and the
//! strip: its samples, or a compressed strip's header, tables and stripes.

use crate::fed::Fed;
use crate::ir::*;
use crate::nd2::Step;
use crate::record::{Field, Header};
use crate::rules::{self, Rules, flag, num, text};
use crate::sans;
use serde_json::{Value as J, json};

/// The signature, the file's first 16 bytes.
pub const SIGNATURE: &[u8; 16] = b"FUJIFILMCCD-RAW ";
/// The header's length: its fields through the directory's twelve words.
pub const HEADER: u64 = 132;
/// The bytes of the CFA region's start the second round reads, which hold its TIFF's IFDs
/// and, in every file surveyed, the strip's tables.
pub const CFA_HEAD: u64 = 1 << 16;
/// A compressed strip's header, and its stripes' width (conventions §13
/// <https://github.com/d-v-b/refkv/blob/main/spec/conventions.md#13-fujifilm-compressed-stripes>).
pub const STRIPE_HEADER: u64 = 16;
pub const STRIPE: u64 = 768;
/// The raw IFD's tag in the CFA region's IFD 0, and its tags the profile reads.
pub const RAW_IFD: u16 = 0xF000;
pub const RAW_TAGS: [(u16, &str); 5] = [(0xF001, "width"), (0xF002, "height"), (0xF003, "bits"), (0xF007, "strip_offset"), (0xF008, "strip_bytes")];
/// The metadata record of the X-Trans pattern.
pub const XTRANS_RECORD: u16 = 0x0131;
/// The pattern of a compressed Bayer strip, which no file states: the one LibRaw and rawpy
/// give every GFX model (spec/zarrify/raf.md §3
/// <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#3-the-sensor-data>).
pub const BAYER: [[u8; 2]; 2] = [[0, 1], [1, 2]];
/// The most entries an IFD may have.
const MAX_ENTRIES: usize = 512;

/// The header's fields, then the directory's twelve big-endian words
/// (spec/zarrify/raf.md §2 <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#2-the-file>).
pub const FIELDS: [Field; 7] = [
    ("magic", 0, "c", 16), ("format_version", 16, "c", 4), ("camera_id", 20, "c", 8), ("camera", 28, "c", 32),
    ("directory_version", 60, "c", 4), ("reserved", 64, "b", 20), ("directory", 84, "u4", 12),
];
/// The names of the directory's words, by index.
pub const WORDS: [&str; 12] = [
    "jpeg_offset", "jpeg_length", "metadata_offset", "metadata_length", "cfa_offset", "cfa_length",
    "compression", "cfa_length_2", "word_8", "frame_2_metadata_offset", "frame_2_metadata_length", "frame_2_cfa_offset",
];

/// Whether a file's first bytes are a RAF file's: the signature.
pub fn detect(head: &[u8]) -> bool {
    head.starts_with(SIGNATURE)
}

/// The header, at the start of `raw` (at least `HEADER` bytes).
pub fn header(raw: &[u8]) -> Header<'_> {
    Header { raw: &raw[..HEADER as usize], little: false, fields: &FIELDS }
}

fn be(b: &[u8], at: usize, n: usize) -> Option<u64> {
    crate::fed::try_uint(b, at, n, false)
}

/// The metadata records: a big-endian count, then each record's tag, length and bytes. `None`
/// when a record runs past the bytes; the count is the caller's to bound.
pub fn records(raw: &[u8]) -> Option<(u64, Vec<(u16, &[u8])>)> {
    let count = be(raw, 0, 4)?;
    let mut out = Vec::new();
    let mut at = 4usize;
    for _ in 0..count.min(256) {
        let tag = be(raw, at, 2)? as u16;
        let length = be(raw, at + 2, 2)? as usize;
        let bytes = raw.get(at + 4..at + 4 + length)?;
        out.push((tag, bytes));
        at += 4 + length;
    }
    Some((count, out))
}

/// One TIFF entry: tag, type, count, and the 4 bytes of its value or offset.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub tag: u16,
    pub kind: u16,
    pub count: u32,
    pub field: [u8; 4],
}

impl Entry {
    /// The value of a one-count SHORT or LONG (or IFD) entry.
    pub fn scalar(&self, little: bool) -> Option<u64> {
        match (self.kind, self.count) {
            (3, 1) => crate::fed::try_uint(&self.field, 0, 2, little),
            (4 | 13, 1) => crate::fed::try_uint(&self.field, 0, 4, little),
            _ => None,
        }
    }
}

/// The CFA region's TIFF: its byte order, and the IFD at each of its offsets, in the order read
/// (IFD 0, then the raw IFD).
pub struct Tiff {
    pub little: bool,
    pub ifds: Vec<(u64, Vec<Entry>)>,
}

/// The IFD at `at` of `raw`: `None` when it runs past `raw` or has more than `MAX_ENTRIES`.
fn ifd(raw: &[u8], at: u64, little: bool) -> Option<Vec<Entry>> {
    let at = usize::try_from(at).ok()?;
    let n = crate::fed::try_uint(raw, at, 2, little)? as usize;
    if n > MAX_ENTRIES {
        return None;
    }
    (0..n).map(|k| {
        let e = raw.get(at + 2 + 12 * k..at + 14 + 12 * k)?;
        Some(Entry {
            tag: crate::fed::try_uint(e, 0, 2, little)? as u16,
            kind: crate::fed::try_uint(e, 2, 2, little)? as u16,
            count: crate::fed::try_uint(e, 4, 4, little)? as u32,
            field: [e[8], e[9], e[10], e[11]],
        })
    }).collect()
}

/// The CFA region's TIFF from its first bytes `raw`: `Err` with the reason it is not one, or
/// has no raw IFD within `raw`.
pub fn tiff(raw: &[u8]) -> Result<Tiff, &'static str> {
    let little = match raw.get(..4) {
        Some(b"II*\0") => true,
        Some(b"MM\0*") => false,
        _ => return Err("not a TIFF"),
    };
    let first = crate::fed::try_uint(raw, 4, 4, little).ok_or("not a TIFF")?;
    let zero = ifd(raw, first, little).ok_or("no IFD 0")?;
    let sub = zero.iter().find(|e| e.tag == RAW_IFD).and_then(|e| e.scalar(little)).ok_or("no raw IFD")?;
    let raw_ifd = ifd(raw, sub, little).ok_or("no raw IFD")?;
    Ok(Tiff { little, ifds: vec![(first, zero), (sub, raw_ifd)] })
}

/// A compressed strip's header (conventions §13
/// <https://github.com/d-v-b/refkv/blob/main/spec/conventions.md#13-fujifilm-compressed-stripes>).
#[derive(Clone, Copy, Debug)]
pub struct Stripes {
    pub lossless: u8,
    pub raw_type: u8,
    pub bits: u8,
    pub height: u64,
    pub rounded_width: u64,
    pub width: u64,
    pub block: u64,
    pub blocks: u64,
    pub groups: u64,
}

impl Stripes {
    pub fn of(h: &[u8]) -> Option<Stripes> {
        Some(Stripes {
            lossless: *h.get(2)?,
            raw_type: *h.get(3)?,
            bits: *h.get(4)?,
            height: be(h, 5, 2)?,
            rounded_width: be(h, 7, 2)?,
            width: be(h, 9, 2)?,
            block: be(h, 11, 2)?,
            blocks: *h.get(13)? as u64,
            groups: be(h, 14, 2)?,
        })
    }
    /// The table of the blocks' sizes, padded to 16 bytes.
    pub fn table(&self) -> u64 {
        (4 * self.blocks).next_multiple_of(16)
    }
    /// A block's run of `q_base` bytes, of a lossy strip.
    pub fn step(&self) -> u64 {
        self.groups.next_multiple_of(16)
    }
    /// The `q_base` runs, of a lossy strip: none for a lossless one.
    pub fn q_area(&self) -> u64 {
        if self.lossless == 1 { 0 } else { self.blocks * self.step() }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Start,
    Head,
    Region,
    Strip,
    Done,
}

/// What the second round found: the regions, the records' bytes, the TIFF's.
struct Found {
    words: [u64; 12],
    tiff: Tiff,
    /// the strip's offset in the file, and its bytes
    strip: (u64, u64),
    width: u64,
    height: u64,
    bits: u64,
}

pub struct Raf {
    size: u64,
    phase: Phase,
    fed: Fed,
    ir: Ir,
    rules: &'static Rules,
    found: Option<Found>,
    facts: J,
}

impl Raf {
    pub fn new(size: u64) -> Self {
        Raf { size, phase: Phase::Start, fed: Fed::default(), ir: Ir::new(size), rules: rules::of("raf"), found: None, facts: J::Null }
    }

    pub fn feed(&mut self, offset: u64, data: Vec<u8>) {
        self.fed.feed(offset, data);
    }

    pub fn step(&mut self) -> Result<Step, String> {
        match self.phase {
            Phase::Start => {
                self.phase = Phase::Head;
                Ok(Step::Read(vec![(0, self.size.min(HEADER))]))
            }
            Phase::Head => {
                let got = self.fed.get(0, self.size.min(HEADER)).ok_or("internal: no header")?.to_vec();
                let words = self.head(&got)?;
                self.phase = Phase::Region;
                let cfa = (words[4], words[5].min(CFA_HEAD));
                Ok(Step::Read(vec![(words[2], words[3]), cfa]))
            }
            Phase::Region => {
                let words = self.words()?;
                self.region(words)?;
                self.phase = Phase::Strip;
                self.step()
            }
            Phase::Strip => {
                let reads = self.strip_reads()?;
                if let Step::Read(_) = reads {
                    return Ok(reads);
                }
                self.strip()?;
                self.phase = Phase::Done;
                Ok(Step::Done)
            }
            Phase::Done => Ok(Step::Done),
        }
    }

    fn words(&self) -> Res<[u64; 12]> {
        let got = self.fed.get(0, HEADER).ok_or("internal: no header")?;
        let h = header(got);
        let mut words = [0u64; 12];
        for (k, v) in h.values("directory").iter().enumerate() {
            words[k] = v.i() as u64;
        }
        Ok(words)
    }

    /// The header and its directory (spec/zarrify/raf.md §8.1
    /// <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#81-the-header>).
    fn head(&mut self, got: &[u8]) -> Res<[u64; 12]> {
        if !detect(got) {
            return Err("not a RAF file".into());
        }
        self.rules.check("file", rules::env([("size", num(self.size as f64))]))?;
        let words = self.words()?;
        let regions = [("embedded JPEG", words[0], words[1]), ("metadata records", words[2], words[3]), ("CFA region", words[4], words[5])];
        for (name, offset, length) in regions {
            self.rules.check("region", rules::env([
                ("name", text(name)), ("offset", num(offset as f64)), ("length", num(length as f64)), ("size", num(self.size as f64)),
            ]))?;
        }
        for (i, a) in regions.iter().enumerate() {
            for b in &regions[i + 1..] {
                let apart = a.1 + a.2 <= b.1 || b.1 + b.2 <= a.1;
                self.rules.check("overlap", rules::env([
                    ("a", text(a.0)), ("a_offset", num(a.1 as f64)), ("b", text(b.0)), ("b_offset", num(b.1 as f64)), ("apart", flag(apart)),
                ]))?;
            }
        }
        self.rules.check("second_frame", rules::env([("offset", num(words[11] as f64))]))?;
        self.rules.check("records_length", rules::env([("length", num(words[3] as f64))]))?;
        Ok(words)
    }

    /// The metadata records and the CFA region's TIFF (§8.1
    /// <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#81-the-header>).
    fn region(&mut self, words: [u64; 12]) -> Res<()> {
        let meta = self.fed.get(words[2], words[3]).ok_or("internal: no metadata records")?;
        let (count, inside) = match records(meta) {
            Some((count, list)) => (count, list.len() as u64 == count),
            None => (be(meta, 0, 4).unwrap_or(0), false),
        };
        self.rules.check("records", rules::env([("count", num(count as f64)), ("inside", flag(inside)), ("length", num(words[3] as f64))]))?;
        let head = self.fed.get(words[4], words[5].min(CFA_HEAD)).ok_or("internal: no CFA region")?;
        let parsed = tiff(head);
        let reason = match &parsed {
            Ok(_) => "",
            Err(r) => r,
        };
        self.rules.check("cfa_tiff", rules::env([
            ("offset", num(words[4] as f64)), ("tiff", flag(reason != "not a TIFF")), ("raw_ifd", flag(parsed.is_ok())),
        ]))?;
        let tiff = parsed.map_err(|e| format!("internal: {e}"))?;
        let little = tiff.little;
        let raw_ifd = &tiff.ifds[1].1;
        let mut tags = [0u64; 5];
        for (k, (tag, name)) in RAW_TAGS.iter().enumerate() {
            let value = raw_ifd.iter().find(|e| e.tag == *tag).and_then(|e| e.scalar(little));
            self.rules.check("raw_tag", rules::env([("name", text(name)), ("tag", text(&format!("0x{tag:04X}"))), ("present", flag(value.is_some()))]))?;
            tags[k] = value.unwrap_or(0);
        }
        let [width, height, bits, strip_offset, strip_bytes] = tags;
        self.rules.check("strip", rules::env([
            ("offset", num(strip_offset as f64)), ("bytes", num(strip_bytes as f64)), ("region", num(words[5] as f64)),
            ("head", num(CFA_HEAD as f64)),
        ]))?;
        let strip = (words[4] + strip_offset, strip_bytes);
        self.found = Some(Found { words, tiff, strip, width, height, bits });
        Ok(())
    }

    /// The reads the strip's checks need that the earlier rounds did not make: its first 16
    /// bytes, then a compressed strip's tables. `Done` when all are fed.
    fn strip_reads(&self) -> Result<Step, String> {
        let f = self.found.as_ref().ok_or("internal: no CFA region")?;
        let (start, bytes) = f.strip;
        let first = bytes.min(STRIPE_HEADER);
        if !self.fed.has(start, first) {
            return Ok(Step::Read(vec![(start, first)]));
        }
        let head = self.fed.get(start, first).unwrap_or_default();
        if head.starts_with(b"IS") && head.len() as u64 == STRIPE_HEADER {
            if let Some(s) = Stripes::of(head) {
                // within the CFA region, which the second round checked lies in the file
                let end = f.words[4] + f.words[5];
                let tables = (s.table() + s.q_area()).min(end.saturating_sub(start + STRIPE_HEADER));
                if !self.fed.has(start + STRIPE_HEADER, tables) {
                    return Ok(Step::Read(vec![(start + STRIPE_HEADER, tables)]));
                }
            }
        }
        Ok(Step::Done)
    }

    /// The strip: its encoding and checks, then the IR's elements (§8.2
    /// <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#82-the-strip>).
    fn strip(&mut self) -> Res<()> {
        let f = self.found.take().ok_or("internal: no CFA region")?;
        let (start, bytes) = f.strip;
        let head = self.fed.get(start, bytes.min(STRIPE_HEADER)).ok_or("internal: no strip")?.to_vec();
        let compressed = head.starts_with(b"IS") && head.len() as u64 == STRIPE_HEADER;
        let samples = (f.width as u128) * (f.height as u128);
        self.rules.check("encoding", rules::env([
            ("compressed", flag(compressed)), ("bytes", num(bytes as f64)), ("samples", num(samples as f64)),
            ("width", num(f.width as f64)), ("height", num(f.height as f64)),
            ("per", text(&if samples > 0 { format!("{:.2}", bytes as f64 / samples as f64) } else { "∞".into() })),
        ]))?;
        let records = self.fed.get(f.words[2], f.words[3]).and_then(records).map(|r| r.1).unwrap_or_default();
        let header = self.ir.value(0, "header", NO_INDEX, &header(self.fed.get(0, HEADER).ok_or("internal: no header")?).record_type(), 0, (0, HEADER),
                                   self.fed.get(0, HEADER))?;
        if f.words[1] > 0 {
            self.ir.value(0, "jpeg", NO_INDEX, &format!("bytes[{}]", f.words[1]), 0, (f.words[0], f.words[1]), None)?;
        }
        let meta = self.fed.get(f.words[2], f.words[3]).ok_or("internal: no metadata records")?.to_vec();
        let metadata = self.ir.value(0, "metadata", NO_INDEX, &format!("bytes[{}]", f.words[3]), 0, (f.words[2], f.words[3]), Some(&meta))?;
        let strip_offset = start - f.words[4];
        let tiff_raw = self.fed.get(f.words[4], strip_offset).ok_or("the CFA region's TIFF ends past the bytes the profile reads")?.to_vec();
        let tiff = self.ir.value(0, "cfa_tiff", NO_INDEX, &format!("bytes[{strip_offset}]"), 0, (f.words[4], strip_offset), Some(&tiff_raw))?;
        let mut facts = json!({
            "header": header, "metadata": metadata, "cfa_tiff": tiff, "width": f.width, "height": f.height, "bits": f.bits,
            "little": f.tiff.little, "strip": [start, bytes],
            "ifds": f.tiff.ifds.iter().map(|(o, _)| o).collect::<Vec<_>>(),
        });
        let region_end = f.words[4] + f.words[5];
        if !compressed {
            let end = start as u128 + 2 * samples;
            self.rules.check("samples", rules::env([
                ("end", num(end as f64)), ("end_shown", text(&end.to_string())), ("region_end", num(region_end as f64)),
            ]))?;
            let endian = if f.tiff.little { "little" } else { "big" };
            let form = json!({"geometry": {"data_type": "uint16", "samples": 1}, "codec": {"endian": endian}, "recipe": [["src", 0, null]]});
            let data = self.ir.data(0, "samples", NO_INDEX, 0, (start, 2 * f.width * f.height), &form.to_string())?;
            facts["encoding"] = json!("uncompressed");
            facts["data"] = json!(data);
            facts["copy_threshold"] = json!(crate::out::copy_threshold());
            self.facts = facts;
            return Ok(());
        }
        let s = Stripes::of(&head).ok_or("internal: no stripe header")?;
        self.rules.check("stripes", rules::env([
            ("lossless", num(s.lossless)), ("raw_type", num(s.raw_type)), ("bits", num(s.bits)), ("height", num(s.height as f64)),
            ("rounded_width", num(s.rounded_width as f64)), ("width", num(s.width as f64)), ("block", num(s.block as f64)),
            ("blocks", num(s.blocks as f64)), ("groups", num(s.groups as f64)),
            ("tiff_width", num(f.width as f64)), ("tiff_height", num(f.height as f64)),
        ]))?;
        let cfa: Vec<Vec<u8>> = if s.raw_type == 16 {
            let pattern = records.iter().find(|r| r.0 == XTRANS_RECORD).map(|r| r.1);
            let valid = pattern.is_some_and(|p| p.len() == 36 && p.iter().all(|&c| c <= 2));
            self.rules.check("xtrans_pattern", rules::env([("valid", flag(valid))]))?;
            let p = pattern.unwrap_or_default();
            // the record holds the pattern's 36 colors last first
            (0..6).map(|y| (0..6).map(|x| p[35 - (6 * y + x)]).collect()).collect()
        } else {
            BAYER.iter().map(|r| r.to_vec()).collect()
        };
        let tables = s.table() + s.q_area();
        let fits = start + STRIPE_HEADER + tables <= region_end;
        let sizes_raw = if fits { self.fed.get(start + STRIPE_HEADER, s.table()).unwrap_or_default() } else { &[] };
        let sizes: Vec<u64> = (0..s.blocks as usize).map(|k| be(sizes_raw, 4 * k, 4).unwrap_or(0)).collect();
        let total: u128 = sizes.iter().map(|&v| v as u128).sum();
        let first = start + STRIPE_HEADER + tables;
        let end = first as u128 + total;
        self.rules.check("stripe_data", rules::env([
            ("fits", flag(fits)), ("tables", num(tables as f64)), ("end", num(end as f64)), ("end_shown", text(&end.to_string())),
            ("region_end", num(region_end as f64)),
        ]))?;
        let total = total as u64;
        self.ir.value(0, "stripe_header", NO_INDEX, "bytes[16]", 0, (start, STRIPE_HEADER), Some(&head))?;
        let table_raw = self.fed.get(start + STRIPE_HEADER, tables).ok_or("internal: no stripe tables")?.to_vec();
        self.ir.value(0, "stripe_tables", NO_INDEX, &format!("bytes[{tables}]"), 0, (start + STRIPE_HEADER, tables), Some(&table_raw))?;
        let config = json!({"bits": s.bits, "lossless": s.lossless == 1, "cfa": cfa});
        let form = json!({"geometry": {"data_type": "uint16", "shape": [s.height, s.width]}, "codec": {"refkv.fuji_compressed": config, "sizes": sizes},
                          "recipe": [["src", 0, null]]});
        let data = if total > 0 { Some(self.ir.data(0, "stripes", NO_INDEX, 0, (first, total), &form.to_string())?) } else { None };
        let mut offsets = Vec::with_capacity(sizes.len());
        let mut at = first;
        for &n in &sizes {
            offsets.push(json!([at, n]));
            at += n;
        }
        let q_runs: Vec<J> = (0..s.blocks).filter(|_| s.lossless != 1).map(|k| json!([start + STRIPE_HEADER + s.table() + k * s.step(), s.step()])).collect();
        facts["encoding"] = json!("compressed");
        facts["data"] = json!(data);
        facts["stripes"] = json!({
            "lossless": s.lossless, "raw_type": s.raw_type, "bits": s.bits, "height": s.height, "rounded_width": s.rounded_width,
            "width": s.width, "block_size": s.block, "blocks": s.blocks, "row_groups": s.groups, "block_sizes": sizes,
        });
        facts["config"] = config;
        facts["cfa_from"] = json!(if s.raw_type == 16 { "file" } else { "readers" });
        facts["blocks"] = J::Array(offsets);
        facts["q_runs"] = J::Array(q_runs);
        self.facts = facts;
        Ok(())
    }

    pub fn finish(self) -> Result<(Ir, J), String> {
        sans::finish(self.phase == Phase::Done, self.ir, self.facts)
    }
}

crate::parser!(Raf);

/// The RAF profile's file run (spec/zarrify/raf.md).
pub struct Profile;

impl crate::run::FileProfile for Profile {
    /// The 16 bytes `FUJIFILMCCD-RAW ` at the start (`detect`).
    fn detect(&self, head: &[u8]) -> bool {
        detect(head)
    }
    fn parser(&self, size: u64) -> crate::run::AnyParser {
        crate::run::AnyParser::of::<Raf>(size)
    }
    fn project(&self, ir: &crate::ir::Ir, facts: &serde_json::Value, url: &str, copied: &crate::project::Copied) -> Result<crate::out::Out, String> {
        crate::project::raf::project(ir, facts, url, copied)
    }
    fn reads(&self, ir: &crate::ir::Ir, facts: &serde_json::Value) -> Result<Vec<(u64, u64)>, String> {
        crate::project::raf::reads(ir, facts)
    }
}
