"""The `refkv.fuji_compressed` codec (python/src/refkv/zarr/codecs/fuji_compressed.py;
spec/conventions.md §13
<https://github.com/d-v-b/refkv/blob/main/spec/conventions.md#13-fujifilm-compressed-stripes>) as
zarr reads it: the first row groups of real stripes (fixtures/raf/README.md) decode to rawpy's
samples; and one test per error a reader meets."""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

import numpy as np
import pytest
import zarr
from zarr.storage import LocalStore

from refkv import _core
from refkv._narrow import ndarray
from refkv.zarr.codecs.fuji_compressed import FujiCompressedCodec

FIXTURES = Path(__file__).resolve().parents[2] / "fixtures" / "raf"
BAYER = [[0, 1], [1, 2]]
LOSSLESS = {"bits": 14, "lossless": True, "cfa": BAYER}
PREFIX = "refkv.fuji_compressed: "


def read(chunk: bytes, config: Any, shape: tuple[int, ...], root: Path, dtype: str = "uint16") -> np.ndarray:
    """`chunk` as the one chunk of a zarr array of `shape` whose codec is `refkv.fuji_compressed`
    with `config`."""
    codec = FujiCompressedCodec.from_dict({"name": "refkv.fuji_compressed", "configuration": config})
    zarr.create_array(LocalStore(root), shape=shape, chunks=shape, dtype=dtype, fill_value=0, compressors=None,
                      serializer=codec, overwrite=True)
    key = root / "c" / "/".join("0" * len(shape))
    key.parent.mkdir(parents=True, exist_ok=True)
    key.write_bytes(chunk)
    return ndarray(zarr.open_array(LocalStore(root, read_only=True), mode="r")[...])


def rejects(message: str, chunk: bytes, config: Any, shape: tuple[int, ...], root: Path) -> None:
    with pytest.raises(ValueError, match=f"^{re.escape(PREFIX + message)}$"):
        read(chunk, config, shape, root)


def test_every_vector_decodes_to_rawpys_samples(tmp_path):
    vectors = json.loads((FIXTURES / "vectors.json").read_text())
    for v in vectors:
        name, shape = v["name"], tuple(v["shape"])
        chunk = (FIXTURES / f"{name}.chunk").read_bytes()
        want = np.frombuffer((FIXTURES / f"{name}.expected").read_bytes(), "<u2").reshape(shape)
        got = read(chunk, v["configuration"], shape, tmp_path / name)
        np.testing.assert_array_equal(got, want, err_msg=name, strict=True)
        # the chunk of a 3-dimensional array, its first dimension of length 1
        got = read(chunk, v["configuration"], (1, *shape), tmp_path / f"{name}.3")
        np.testing.assert_array_equal(got[0], want, err_msg=name, strict=True)
    assert len(vectors) == 3


def test_rejects_a_configuration_that_is_not_an_object():
    # zarr refuses an array document whose configuration is not an object: the core, directly
    with pytest.raises(ValueError, match=f"^{re.escape(PREFIX)}the configuration is not an object$"):
        _core.fuji_compressed_decode(b"", "[14]", [6, 768])


def test_rejects_an_unknown_member(tmp_path):
    rejects('the configuration has an unknown member "width"', b"", {**LOSSLESS, "width": 768}, (6, 768), tmp_path)


def test_rejects_a_configuration_without_bits(tmp_path):
    rejects("the configuration has no bits", b"", {"lossless": True, "cfa": BAYER}, (6, 768), tmp_path)


def test_rejects_bits_other_than_12_14_or_16(tmp_path):
    rejects("the configuration's bits is 10, not 12, 14 or 16", b"", {**LOSSLESS, "bits": 10}, (6, 768), tmp_path)


def test_rejects_a_configuration_without_lossless(tmp_path):
    rejects("the configuration has no lossless", b"", {"bits": 14, "cfa": BAYER}, (6, 768), tmp_path)


def test_rejects_a_lossless_that_is_not_a_boolean(tmp_path):
    rejects("the configuration's lossless is 1, not a boolean", b"", {**LOSSLESS, "lossless": 1}, (6, 768), tmp_path)


def test_rejects_a_configuration_without_cfa(tmp_path):
    rejects("the configuration has no cfa", b"", {"bits": 14, "lossless": True}, (6, 768), tmp_path)


def test_rejects_a_cfa_not_of_2_rows_of_2_or_6_of_6_colors(tmp_path):
    rejects("the configuration's cfa is [[0,1],[1,3]], not 2 rows of 2 or 6 rows of 6 colors 0, 1 or 2", b"",
            {**LOSSLESS, "cfa": [[0, 1], [1, 3]]}, (6, 768), tmp_path)


def test_rejects_a_chunk_shape_not_of_a_stripe(tmp_path):
    for shape in [(6, 512), (8, 768), (2, 6, 768)]:
        dims = ", ".join(map(str, shape))
        rejects(f"the chunk shape [{dims}] is not 1 along all but its last two dimensions, a multiple of 6 from 6 to "
                "16386 and 768", b"", LOSSLESS, shape, tmp_path / str(shape))


def test_rejects_a_lossy_chunk_shorter_than_its_q_base(tmp_path):
    rejects("the chunk is 31 bytes, fewer than the 32 of its 17 row groups' q_base", bytes(31),
            {**LOSSLESS, "lossless": False}, (102, 768), tmp_path)


def test_rejects_coded_bytes_that_end_before_the_last_sample(tmp_path):
    # the X-E3 vector decodes to its block's first 24 rows, and has too few bytes for 48
    v = json.loads((FIXTURES / "vectors.json").read_text())[0]
    chunk = (FIXTURES / f"{v['name']}.chunk").read_bytes()
    rejects("the coded bytes end before the stripe's last sample", chunk, v["configuration"], (48, 768), tmp_path)


def test_rejects_a_code_of_its_tables_values_or_more(tmp_path):
    # the first sample: 41 zeros, then read(14) + 1 = 16384, not less than T = 16384
    rejects("a code of 16384 is not less than its table's 16384 values", bytes([0, 0, 0, 0, 0, 0x7F, 0xFF]),
            LOSSLESS, (6, 768), tmp_path)


def test_rejects_an_array_that_is_not_uint16(tmp_path):
    with pytest.raises(ValueError, match=r"^refkv\.fuji_compressed: an array of int32 is not decoded \(uint16\)$"):
        read(b"", LOSSLESS, (6, 768), tmp_path, "int32")
