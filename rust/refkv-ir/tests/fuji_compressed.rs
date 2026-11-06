//! The `refkv.fuji_compressed` codec (src/fuji_compressed.rs; spec/conventions.md §13
//! <https://github.com/d-v-b/refkv/blob/main/spec/conventions.md#13-fujifilm-compressed-stripes>):
//! the first row groups of real stripes (fixtures/raf/README.md) decode to rawpy's samples, and
//! one test per error.

mod common;

use refkv_ir::fuji_compressed::{Config, decode};
use serde_json::{Value as J, json};

/// Each test vector of fixtures/raf/vectors.json: its name, configuration and chunk shape.
fn vectors() -> Vec<(String, J, Vec<u64>)> {
    let dir = common::fixtures().join("raf");
    let v: J = serde_json::from_slice(&std::fs::read(dir.join("vectors.json")).unwrap()).unwrap();
    v.as_array().unwrap().iter().map(|e| {
        let shape = e["shape"].as_array().unwrap().iter().map(|n| n.as_u64().unwrap()).collect();
        (e["name"].as_str().unwrap().to_string(), e["configuration"].clone(), shape)
    }).collect()
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(common::fixtures().join("raf").join(name)).unwrap()
}

fn config(v: J) -> Config {
    Config::from_json(&v).unwrap()
}

const BAYER: [[u8; 2]; 2] = [[0, 1], [1, 2]];

fn bayer(lossless: bool) -> Config {
    config(json!({"bits": 14, "lossless": lossless, "cfa": BAYER}))
}

#[test]
fn every_vector_decodes_to_rawpys_samples() {
    let mut checked = 0;
    for (name, cfg, shape) in vectors() {
        let chunk = read(&format!("{name}.chunk"));
        let want: Vec<u16> = read(&format!("{name}.expected")).chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        let got = decode(&chunk, &config(cfg.clone()), &shape).unwrap();
        assert_eq!(got.len(), want.len(), "{name}");
        if let Some(i) = (0..got.len()).find(|&i| got[i] != want[i]) {
            panic!("{name}: row {} column {}: {} and not {}", i / 768, i % 768, got[i], want[i]);
        }
        // the chunk of a 3-dimensional array, its first dimension of length 1
        assert_eq!(decode(&chunk, &config(cfg), &[1, shape[0], shape[1]]).unwrap(), want, "{name}");
        checked += 1;
    }
    assert_eq!(checked, 3);
}

#[test]
fn reads_the_configuration() {
    assert_eq!(bayer(true), Config { bits: 14, lossless: true, cfa: vec![vec![0, 1], vec![1, 2]] });
    let cfa = [[1u8; 6]; 6];
    let x = config(json!({"bits": 12, "lossless": false, "cfa": cfa}));
    assert_eq!((x.bits, x.cfa.len()), (12, 6));
}

fn rejects(v: J, message: &str) {
    assert_eq!(Config::from_json(&v), Err(format!("refkv.fuji_compressed: {message}")), "{v}");
}

#[test]
fn rejects_a_configuration_that_is_not_an_object() {
    rejects(json!([14]), "the configuration is not an object");
}

#[test]
fn rejects_an_unknown_member() {
    rejects(json!({"bits": 14, "lossless": true, "cfa": BAYER, "width": 768}), "the configuration has an unknown member \"width\"");
}

#[test]
fn rejects_a_configuration_without_bits() {
    rejects(json!({"lossless": true, "cfa": BAYER}), "the configuration has no bits");
}

#[test]
fn rejects_bits_other_than_12_14_or_16() {
    for b in [json!(10), json!(14.5), json!("14")] {
        rejects(json!({"bits": b, "lossless": true, "cfa": BAYER}), &format!("the configuration's bits is {b}, not 12, 14 or 16"));
    }
}

#[test]
fn rejects_a_configuration_without_lossless() {
    rejects(json!({"bits": 14, "cfa": BAYER}), "the configuration has no lossless");
}

#[test]
fn rejects_a_lossless_that_is_not_a_boolean() {
    rejects(json!({"bits": 14, "lossless": 1, "cfa": BAYER}), "the configuration's lossless is 1, not a boolean");
}

#[test]
fn rejects_a_configuration_without_cfa() {
    rejects(json!({"bits": 14, "lossless": true}), "the configuration has no cfa");
}

#[test]
fn rejects_a_cfa_not_of_2_rows_of_2_or_6_of_6_colors() {
    for c in [json!([[0, 1, 1], [1, 2, 1], [0, 1, 2]]), json!([[0, 1], [1]]), json!([[0, 1], [1, 3]]), json!([[0, 1], [1, -1]]), json!([0, 1, 1, 2]), json!("RGGB")] {
        rejects(json!({"bits": 14, "lossless": true, "cfa": c}), &format!("the configuration's cfa is {c}, not 2 rows of 2 or 6 rows of 6 colors 0, 1 or 2"));
    }
}

#[test]
fn rejects_a_chunk_shape_not_of_a_stripe() {
    for shape in [&[6, 767][..], &[1, 6, 768, 1], &[2, 6, 768], &[0, 768], &[8, 768], &[16392, 768], &[768]] {
        assert_eq!(
            decode(&[], &bayer(true), shape),
            Err(format!("refkv.fuji_compressed: the chunk shape {shape:?} is not 1 along all but its last two dimensions, a multiple of 6 from 6 to 16386 and 768")),
        );
    }
    // the largest height
    assert!(decode(&[], &bayer(true), &[16386, 768]).unwrap_err().contains("end before"));
}

#[test]
fn rejects_a_lossy_chunk_shorter_than_its_q_base() {
    // 17 row groups: 32 bytes of q_base
    assert_eq!(
        decode(&[0; 31], &bayer(false), &[102, 768]),
        Err("refkv.fuji_compressed: the chunk is 31 bytes, fewer than the 32 of its 17 row groups' q_base".into()),
    );
}

#[test]
fn rejects_coded_bytes_that_end_before_the_last_sample() {
    let message = Err("refkv.fuji_compressed: the coded bytes end before the stripe's last sample".to_string());
    for (name, cfg, shape) in vectors() {
        let mut chunk = read(&format!("{name}.chunk"));
        let want = decode(&chunk, &config(cfg.clone()), &shape).unwrap();
        // the vector is its block's shortest prefix that decodes to rawpy's samples: a row group
        // more needs more bits, and one byte fewer either ends too soon or decodes otherwise
        assert_eq!(decode(&chunk, &config(cfg.clone()), &[shape[0] + 6, 768]), message, "{name}");
        chunk.pop();
        assert_ne!(decode(&chunk, &config(cfg), &shape), Ok(want), "{name}");
    }
    assert_eq!(decode(&[], &bayer(true), &[6, 768]), message);
}

#[test]
fn rejects_a_code_of_its_tables_values_or_more() {
    // the first sample, at R2's position 0 (a coded sample in a Bayer stripe): 41 zeros, then
    // read(14) + 1 = 16384, which is not less than T = 16384 of 14 bits' main table of step 0
    assert_eq!(
        decode(&[0, 0, 0, 0, 0, 0x7f, 0xff], &bayer(true), &[6, 768]),
        Err("refkv.fuji_compressed: a code of 16384 is not less than its table's 16384 values".into()),
    );
}

#[test]
fn rejects_a_configuration_made_otherwise_that_it_does_not_decode() {
    for cfg in [Config { bits: 13, ..bayer(true) }, Config { cfa: vec![vec![0, 1], vec![1, 2], vec![0, 1]], ..bayer(true) }, Config { cfa: vec![vec![0, 1], vec![1, 3]], ..bayer(true) }] {
        assert_eq!(decode(&[], &cfg, &[6, 768]), Err("refkv.fuji_compressed: the configuration's bits or cfa is not one the codec decodes".into()));
    }
}
