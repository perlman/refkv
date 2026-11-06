//! The RAF parser (src/raf.rs; spec/zarrify/raf.md): one test per rejected fixture.

mod common;

use refkv_ir::raf::Raf;

// one test per case of conformance/zarrify/rejects/raf.json (build.rs), each the message its check gives
macro_rules! reject {
    ($name:ident, $input:literal, $msg:literal) => {
        common::reject!($name, Raf, concat!("raf/", $input), $msg);
    };
}
include!(concat!(env!("OUT_DIR"), "/rejects_raf.rs"));
