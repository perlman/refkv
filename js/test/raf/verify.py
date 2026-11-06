"""Checks the browser RAF zarrifier's output against rawpy (LibRaw).

Each fixtures/raf/*.raf is served over local HTTP and zarrified by js/conformance/zarrify.ts (the
browser code, run under Node). The array is read through the reference reader and zarr-python and
must equal rawpy's `raw_image`, sample for sample; a compressed file's codec configuration must
carry rawpy's pattern (spec/zarrify/raf.md §3
<https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#3-the-sensor-data>); the file must be
rebuilt byte for byte from the archive's IR mirror (refkv_source, §5.2
<https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#52-the-ir-mirror>). `raf_reject_*`
files must be rejected.

A directory of real RAF files can be given instead, such as raw.pixls.us's (spec/zarrify/raf.md
§8 <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#8-raf-profile>): those the
profile reads are checked the same way.

Usage: uv run python js/test/raf/verify.py [<dir of .raf files>]
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
import rawpy
import zarr
from zarr.core.metadata import ArrayV3Metadata

HERE = Path(__file__).parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT / "conformance" / "vzip"))

import refkv.zarr  # noqa: E402  (a vzip archive as a zarr store)
from refkv._narrow import at, ndarray  # noqa: E402

sys.path.insert(0, str(HERE.parent))
from ir_mirror import mirror_problems  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))  # js/test, for verify_common
from verify_common import LOOPBACK_SOURCES, run, serve  # noqa: E402


def rawpy_pattern(raw) -> list[list[int]]:
    """rawpy's pattern from the raw image's first row and column (its own is from the visible
    area's), the second green (3) written as 1."""
    pattern, top, left = raw.raw_pattern, raw.sizes.top_margin, raw.sizes.left_margin
    p = pattern.shape[0]
    colors = [[int(pattern[(y - top) % p][(x - left) % p]) for x in range(p)] for y in range(p)]
    return [[1 if c == 3 else c for c in row] for row in colors]


def problems_of(path: Path, out: Path) -> list[str]:
    with rawpy.imread(str(path)) as raw:
        want = raw.raw_image.copy()
        pattern = rawpy_pattern(raw)
    root = zarr.open_group(refkv.zarr.open(str(out), policy=LOOPBACK_SOURCES), mode="r", zarr_format=3)
    arr = root.get_array("0")
    got = ndarray(arr[...])
    problems = []
    if got.dtype != np.uint16:
        problems.append(f"dtype {got.dtype} != uint16")
    if got.shape != want.shape or not np.array_equal(got, want):
        problems.append(f"samples differ from rawpy's ({got.shape} vs {want.shape})")
    assert isinstance(arr.metadata, ArrayV3Metadata)
    codec = arr.metadata.codecs[0].to_dict()
    if codec["name"] == "refkv.fuji_compressed" and at(codec, "configuration", "cfa") != pattern:
        problems.append(f"pattern {at(codec, 'configuration', 'cfa')} != rawpy's {pattern}")
    return problems + mirror_problems(out, path)


def main(fixtures: Path) -> int:
    paths = sorted(p for p in fixtures.iterdir() if p.suffix.lower() == ".raf")
    return run(paths, lambda path, out, _: problems_of(path, out), url=serve(paths), width=40)


if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "fixtures" / "raf"))
