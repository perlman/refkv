//! The RAF image projection (spec/zarrify/raf.md §4
//! <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#4-the-image>, and the root's
//! source metadata of §5
//! <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#5-source-metadata>) from a valid
//! IR: the header, the metadata records and the CFA region's TIFF are the IR's held values, and
//! the array's chunks are cut from its `samples` element, or are its `stripes` one each.

use super::{contiguous, header_json, n, Copied};
use crate::ir::Ir;
use crate::out::{array_json, bytes_codec, full_window, grid_chunks, image_ome, omero_channel, omero_json, root_json, Out, Part};
use crate::raf::{header, records, Tiff, STRIPE, WORDS};
use crate::refs::Refs;
use serde_json::{json, Map, Value as J};

/// The most bytes of a chunk of uncompressed samples (spec/zarrify/raf.md §4.1
/// <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#41-the-array>).
const CHUNK_BYTES: u64 = 1 << 20;
const AXES: [&str; 2] = ["y", "x"];

fn held<'a>(ir: &'a Ir, facts: &J, name: &str) -> Result<&'a [u8], String> {
    ir.value_bytes(n(&facts[name]) as u32).ok_or_else(|| format!("internal: the {name} is not held"))
}

/// The cut of uncompressed samples: the grid and chunks of contiguous `uint16` values.
fn cut(ir: &Ir, facts: &J) -> Result<(Vec<u64>, Vec<(Vec<u64>, crate::out::Chunk)>), String> {
    let data = n(&facts["data"]) as usize;
    let shape = [n(&facts["height"]), n(&facts["width"])];
    grid_chunks(ir.start[data], &shape, 2, CHUNK_BYTES, contiguous::threshold(facts)).ok_or_else(|| "internal: the samples cannot be cut into chunks".into())
}

/// The source's ranges of the edge chunks of uncompressed samples the projection copies.
pub fn reads(ir: &Ir, facts: &J) -> Result<Vec<(u64, u64)>, String> {
    if facts["encoding"] != "uncompressed" {
        return Ok(Vec::new());
    }
    let (_, chunks) = cut(ir, facts)?;
    Ok(contiguous::copy_reads(&chunks).collect())
}

/// A TIFF entry as the source metadata writes it.
fn entry_json(e: &crate::raf::Entry, little: bool) -> J {
    let value = match e.scalar(little) {
        Some(v) => json!(v),
        None => json!(crate::mirror::base64(&e.field)),
    };
    json!({"tag": e.tag, "type": e.kind, "count": e.count, "value": value})
}

/// The CFA region's TIFF as the source metadata writes it: its byte order and each IFD's
/// offset and entries.
fn tiff_json(raw: &[u8]) -> J {
    match crate::raf::tiff(raw) {
        Ok(Tiff { little, ifds }) => json!({
            "byte_order": if little { "little" } else { "big" },
            "ifds": ifds.iter().map(|(o, es)| json!({"offset": o, "entries": es.iter().map(|e| entry_json(e, little)).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        }),
        Err(_) => J::Null,
    }
}

pub fn project(ir: &Ir, facts: &J, url: &str, copied: &Copied) -> Result<Out, String> {
    let raw = held(ir, facts, "header")?;
    let h = header(raw);
    let meta = held(ir, facts, "metadata")?;
    let tiff = held(ir, facts, "cfa_tiff")?;
    let (height, width, bits) = (n(&facts["height"]), n(&facts["width"]), n(&facts["bits"]));
    let compressed = facts["encoding"] == "compressed";
    let stripes = &facts["stripes"];
    // the samples' bits: a compressed strip's header gives them, else the raw IFD (0xF003)
    let bits = if compressed { n(&stripes["bits"]) } else { bits };

    // the image (§4 <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#4-the-image>)
    let mut ome = image_ome(&AXES, &Map::new(), &[json!([1, 1])], None, None)?;
    if (1..=16).contains(&bits) {
        let window = full_window(json!(0), json!((1u64 << bits) - 1));
        ome["omero"] = omero_json(vec![omero_channel("raw", "FFFFFF", Some(&window))]);
    }

    // the source metadata (§5.1 <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#51-the-root>)
    let words: Vec<u64> = h.values("directory").iter().map(|v| v.i() as u64).collect();
    let directory: Map<String, J> = WORDS.iter().zip(&words).map(|(k, v)| (k.to_string(), json!(v))).collect();
    let mut own = Map::new();
    own.insert("header".into(), header_json(&h));
    own.insert("directory".into(), J::Object(directory));
    let list = records(meta).map(|r| r.1).unwrap_or_default();
    own.insert("records".into(), J::Array(list.iter().map(|(tag, b)| json!({"tag": tag, "value": crate::mirror::base64(b)})).collect()));
    own.insert("cfa_tiff".into(), tiff_json(tiff));
    own.insert("encoding".into(), facts["encoding"].clone());
    if compressed {
        own.insert("stripes".into(), stripes.clone());
        own.insert("cfa".into(), json!({"pattern": facts["config"]["cfa"], "from": facts["cfa_from"]}));
    } else {
        own.insert("byte_order".into(), json!(if facts["little"] == true { "little" } else { "big" }));
    }

    let mut out = Out::new(url);
    out.json("zarr.json", &root_json(ome, "raf", url, Some(J::Object(own))));
    let shape = [height, width];
    let (grid, count) = if compressed {
        let config = facts["config"].clone();
        let codecs = json!([{"name": "refkv.fuji_compressed", "configuration": config}]);
        let grid = vec![height, STRIPE];
        out.json("0/zarr.json", &array_json(&shape, "uint16", json!(grid), codecs, &AXES));
        let data = n(&facts["data"]) as usize;
        let (start, length, form) = (ir.start[data], ir.len[data], ir.form_id(data as u32).ok_or("internal: the stripes have no form")?);
        let blocks = facts["blocks"].as_array().cloned().unwrap_or_default();
        let runs = facts["q_runs"].as_array().cloned().unwrap_or_default();
        let mut refs = Refs::new(ir, out);
        for (k, b) in blocks.iter().enumerate() {
            // a lossy stripe's chunk is its q_base run, then its coded bytes (conventions §13
            // <https://github.com/d-v-b/refkv/blob/main/spec/conventions.md#13-fujifilm-compressed-stripes>)
            let mut parts = Vec::with_capacity(2);
            if let Some(r) = runs.get(k) {
                parts.push(Part::Src(n(&r[0]), n(&r[1])));
            }
            parts.push(Part::Src(n(&b[0]), n(&b[1])));
            refs.chunk(format!("0/c/0/{k}"), start, length, form, Some(parts), k as u64)?;
        }
        out = refs.out;
        (grid, blocks.len())
    } else {
        let endian = if facts["little"] == true { "little" } else { "big" };
        let (grid, chunks) = cut(ir, facts)?;
        out.json("0/zarr.json", &array_json(&shape, "uint16", json!(grid), json!([bytes_codec(2, endian)]), &AXES));
        let data = n(&facts["data"]) as usize;
        let (start, length, form) = (ir.start[data], ir.len[data], ir.form_id(data as u32).ok_or("internal: the samples have no form")?);
        let count = chunks.len();
        let mut refs = Refs::new(ir, out);
        let mut zeros = contiguous::Zeros::of(&chunks);
        contiguous::emit(&mut refs, chunks, (start, length, form), copied, &["0/c/"], |c| c, &mut zeros)?;
        out = refs.out;
        (grid, count)
    };
    out.summary = json!({
        "camera": super::json_text(h.bytes("camera")), "encoding": facts["encoding"],
        "sensor": if compressed { if n(&stripes["raw_type"]) == 16 { json!("x-trans") } else { json!("bayer") } } else { J::Null },
        "lossless": if compressed { json!(n(&stripes["lossless"]) == 1) } else { J::Null },
        "bits": bits, "sizes": {"y": height, "x": width}, "chunkShape": grid, "chunks": count,
    });
    Ok(out)
}
