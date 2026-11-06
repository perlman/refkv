"""The RAF IR: the Rust parser and its schema, the image projection (spec/zarrify/raf.md) of every
synthetic file against the samples it was written with, and the mirror's rebuild. The compressed
files' stripes are the codec's test vectors, whose samples rawpy gave (fixtures/raf/README.md);
js/test/raf/verify.py compares every file with rawpy itself. RAF has no frozen reference."""

import base64
import json
import struct

import numpy as np
import pytest
from ir_helpers import FIXTURES
from ir_helpers import zarrified as shared_zarrified
from zarr.core.metadata import ArrayV3Metadata

from refkv._narrow import at, integer, mapping, ndarray, sequence

RAF = FIXTURES / "raf"
VECTORS = {v["name"]: v for v in json.loads((RAF / "vectors.json").read_text())}
# each accepted file: its stripes' vector (None: uncompressed), and its stripes
ACCEPTED = {
    "raf_uncompressed": (None, 0),
    "raf_lossless_xtrans": ("xe3_lossless_xtrans_b0_h24", 1),
    "raf_lossy_bayer": ("gfx50sii_lossy_bayer_b0_h24", 1),
    "raf_lossy_bayer16": ("gfx100s_lossy_bayer16_b0_h24", 1),
    "raf_lossless_xtrans_narrow_last": ("xe3_lossless_xtrans_b0_h24", 2),
}


def zarrified(path, tmp_path):
    """(the output, the hierarchy's root group); the archive rebuilds the file
    (ir_helpers.zarrified)."""
    out, root, _ = shared_zarrified(path, tmp_path, "raf")
    return out, root


def expected(name: str, width: int) -> np.ndarray:
    """The samples a file was written with: a vector's rows (each stripe the same), or the
    uncompressed ramp of fixtures/generators/raf/write_fixtures.py."""
    vector, _ = ACCEPTED[name]
    if vector is None:
        return ((np.arange(48 * 64) * 37) % 16384).astype(np.uint16).reshape(48, 64)
    rows = np.frombuffer((RAF / f"{vector}.expected").read_bytes(), "<u2").reshape(24, 768)
    return np.tile(rows, (1, -(-width // 768)))[:, :width]


@pytest.mark.parametrize("name", sorted(ACCEPTED))
def test_samples_and_source_metadata(name, tmp_path):
    """The array is the samples the file holds, with the codec and chunks of spec/zarrify/raf.md
    §4.1 <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#41-the-array>; the source
    metadata holds the header, every record and the TIFF; the file is rebuilt from the mirror; and
    the run reads only the header, the records, the CFA region's start and nothing else."""
    path = RAF / f"{name}.raf"
    raw = path.read_bytes()
    out, root = zarrified(path, tmp_path)
    arr = root.get_array("0")
    vector, blocks = ACCEPTED[name]
    want = expected(name, arr.shape[1])
    got = ndarray(arr[...])
    assert got.dtype == np.uint16
    assert np.array_equal(got, want)
    meta = mapping(at(root.attrs, "refkv_zarrified", "raf"))
    words = struct.unpack(">12I", raw[84:132])
    assert at(meta, "header", "directory") == list(words)
    assert at(meta, "header", "magic") == "FUJIFILMCCD-RAW "
    assert list(mapping(meta["directory"]).values()) == list(words)
    # every record, as written
    records, at_ = [], words[2] + 4
    for _ in range(struct.unpack(">I", raw[words[2]:words[2] + 4])[0]):
        tag, length = struct.unpack(">HH", raw[at_:at_ + 4])
        records.append({"tag": tag, "value": base64.b64encode(raw[at_ + 4:at_ + 4 + length]).decode()})
        at_ += 4 + length
    assert meta["records"] == records
    tiff = mapping(meta["cfa_tiff"])
    assert tiff["byte_order"] == "little"
    raw_ifd = {integer(at(e, "tag")): at(e, "value") for e in sequence(at(tiff, "ifds", 1, "entries"))}
    assert (raw_ifd[0xF001], raw_ifd[0xF002]) == (arr.shape[1], arr.shape[0])
    assert isinstance(arr.metadata, ArrayV3Metadata)
    codec = arr.metadata.codecs[0].to_dict()
    if vector is None:
        assert meta["encoding"] == "uncompressed"
        assert meta["byte_order"] == "little"
        assert codec == {"name": "bytes", "configuration": {"endian": "little"}}
        assert out.summary["sensor"] is None
    else:
        config = VECTORS[vector]["configuration"]
        assert meta["encoding"] == "compressed"
        assert codec == {"name": "refkv.fuji_compressed", "configuration": config}
        assert arr.chunks == (24, 768)
        assert at(meta, "stripes", "blocks") == blocks
        assert at(meta, "cfa", "pattern") == config["cfa"]
        assert at(meta, "cfa", "from") == ("file" if len(config["cfa"]) == 6 else "readers")
        assert out.summary["chunks"] == blocks
    bits = integer(at(meta, "stripes", "bits")) if vector else 14
    window = at(root.attrs, "ome", "omero", "channels", 0, "window")
    assert window == {"min": 0, "max": 2**bits - 1, "start": 0, "end": 2**bits - 1}
    # the header, then the records and the CFA region's start in one round
    assert at(out.summary, "planner", "batches") == 2


def test_a_lossy_chunk_is_its_q_base_run_then_its_stripe(tmp_path):
    """A lossy stripe's chunk references its run of q_base bytes, then its coded bytes, both
    ranges of the file (spec/zarrify/raf.md §8.3
    <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#83-chunks>)."""
    path = RAF / "raf_lossy_bayer.raf"
    raw = path.read_bytes()
    out, _ = zarrified(path, tmp_path)
    parts = out.refs["0/c/0/0"]
    chunk = (RAF / "gfx50sii_lossy_bayer_b0_h24.chunk").read_bytes()
    assert len(parts) == 2
    ranges = [p for p in parts if isinstance(p, tuple) and len(p) == 2]
    assert len(ranges) == 2
    assert b"".join(raw[o:o + n] for o, n in ranges) == chunk
