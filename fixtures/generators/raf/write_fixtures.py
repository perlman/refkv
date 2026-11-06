"""Writes synthetic Fujifilm RAF files to fixtures/raf/, covering the rules of the RAF profile
(spec/zarrify/raf.md §8
<https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#8-raf-profile>): uncompressed
16-bit samples, compressed stripes (lossless X-Trans, lossy Bayer of 14 and 16 bits, and a
narrow last stripe), and the inputs the profile rejects (`raf_reject_*`).

Every file is packed field by field here. A compressed file's stripes are the codec's test
vectors (fixtures/raf/README.md): the first bytes of real stripes of public-domain files, each
decoding to its stripe's first rows, so that the file is small and rawpy can check it.
js/test/raf/verify.py compares each accepted file's zarrification with rawpy's reading of it.

Usage: uv run python fixtures/generators/raf/write_fixtures.py
"""

from __future__ import annotations

import json
import struct
from pathlib import Path

OUT = Path(__file__).parents[2] / "raf"
VECTORS = {v["name"]: v for v in json.loads((OUT / "vectors.json").read_text())}

JPEG_AT = 148
STRIP_AT = 256
STRIPE = 768


def jpeg(make: bytes, model: bytes) -> bytes:
    """A JPEG of no image whose APP1 holds the EXIF a reader identifies the camera by: Make and
    Model in IFD 0 of a little-endian TIFF."""
    entries = [(0x010F, make + b"\0"), (0x0110, model + b"\0")]
    ifd_at = 8
    data_at = ifd_at + 2 + 12 * len(entries) + 4
    ifd, data = struct.pack("<H", len(entries)), b""
    for tag, value in entries:
        ifd += struct.pack("<HHII", tag, 2, len(value), data_at + len(data))
        data += value
    tiff = b"II*\0" + struct.pack("<I", ifd_at) + ifd + struct.pack("<I", 0) + data
    app1 = b"Exif\0\0" + tiff
    return b"\xff\xd8\xff\xe1" + struct.pack(">H", len(app1) + 2) + app1 + b"\xff\xd9"


def records(items: list[tuple[int, bytes]], count: int | None = None) -> bytes:
    """Metadata records: a big-endian count, then each tag, length and bytes."""
    out = struct.pack(">I", len(items) if count is None else count)
    for tag, value in items:
        out += struct.pack(">HH", tag, len(value)) + value
    return out


def cfa_tiff(width: int, height: int, bits: int, strip: int, strip_bytes: int, *, omit: int | None = None,
             raw_ifd: bool = True) -> bytes:
    """The CFA region's TIFF, up to its strip at `strip`: IFD 0 with tag 0xF000, and the raw IFD
    with the width, height, bits, strip offset and byte count (less `omit`)."""
    tags = [(0xF001, width), (0xF002, height), (0xF003, bits), (0xF007, strip), (0xF008, strip_bytes)]
    tags = [t for t in tags if t[0] != omit]
    zero_at, raw_at = 8, 8 + 2 + 12 + 4
    entry = struct.pack("<HHII", 0xF000 if raw_ifd else 0x0100, 13 if raw_ifd else 4, 1, raw_at)
    zero = struct.pack("<H", 1) + entry + struct.pack("<I", 0)
    entries = b"".join(struct.pack("<HHII", t, 4, 1, v) for t, v in tags)
    raw = struct.pack("<H", len(tags)) + entries + struct.pack("<I", 0)
    head = b"II*\0" + struct.pack("<I", zero_at) + zero + raw
    return head.ljust(strip, b"\0")


def raf(name: str, *, camera: bytes, cfa: bytes, recs: bytes, words: dict[int, int] | None = None,
        shrink: int = 0) -> None:
    """A RAF file: the header and directory, the embedded JPEG, the metadata records, then the
    CFA region; `words` overrides directory words, and `shrink` makes the directory's CFA region
    that many bytes shorter than the file's."""
    preview = jpeg(b"FUJIFILM", camera)
    meta_at = JPEG_AT + len(preview)
    cfa_at = meta_at + len(recs)
    directory = [JPEG_AT, len(preview), meta_at, len(recs), cfa_at, len(cfa) - shrink, 0, len(cfa) - shrink, 0, 0, 0, 0]
    for k, v in (words or {}).items():
        directory[k] = v
    head = (b"FUJIFILMCCD-RAW " + b"0201" + b"FF129502" + camera.ljust(32, b"\0") + b"0100" + bytes(20)
            + struct.pack(">12I", *directory))
    data = head.ljust(JPEG_AT, b"\0") + preview + recs + cfa
    (OUT / f"{name}.raf").write_bytes(data)


def uncompressed(width: int, height: int, seed: int = 0) -> bytes:
    """Distinct 14-bit samples, row by row, little-endian uint16."""
    return b"".join(struct.pack("<H", (i * 37 + seed) % 16384) for i in range(width * height))


def stripes(vector: str, blocks: int = 1) -> tuple[dict, bytes]:
    """A compressed strip of `blocks` copies of a test vector's stripe: its configuration and
    bytes (the stripes' header, the tables, the stripes)."""
    v = VECTORS[vector]
    chunk = (OUT / f"{vector}.chunk").read_bytes()
    config, (height, _) = v["configuration"], v["shape"]
    lossless = config["lossless"]
    groups = height // 6
    step = -(-groups // 16) * 16
    q, stripe = (b"", chunk) if lossless else (chunk[:step], chunk[step:])
    return v, b"".join([
        b"IS", bytes([1 if lossless else 0, 16 if len(config["cfa"]) == 6 else 0, config["bits"]]),
        struct.pack(">HHHH", height, blocks * STRIPE, blocks * STRIPE, STRIPE),
        bytes([blocks]), struct.pack(">H", groups),
        struct.pack(f">{blocks}I", *[len(stripe)] * blocks).ljust(-(-4 * blocks // 16) * 16, b"\0"),
        q * blocks, stripe * blocks,
    ])


def xtrans_record(cfa: list[list[int]]) -> bytes:
    """Metadata record 0x0131: the X-Trans pattern's 36 colors, last first."""
    return bytes(cfa[(35 - c) // 6][(35 - c) % 6] for c in range(36))


def compressed(name: str, vector: str, *, camera: bytes, blocks: int = 1, width: int | None = None,
               patch: dict[int, int] | None = None, pattern: bool = True, tiff_width: int | None = None, **kw) -> None:
    v, strip = stripes(vector, blocks)
    height = v["shape"][0]
    if width is not None:
        strip = strip[:9] + struct.pack(">H", width) + strip[11:]
    for at, byte in (patch or {}).items():
        strip = strip[:at] + bytes([byte]) + strip[at + 1:]
    w = width or blocks * STRIPE
    recs = [(0x0100, struct.pack(">HH", height, w)), (0x0130, b"\x0c\x0c\x0c\x0c")]
    if pattern and len(v["configuration"]["cfa"]) == 6:
        recs.append((0x0131, xtrans_record(v["configuration"]["cfa"])))
    tiff_kw = {k: kw.pop(k) for k in ("omit", "raw_ifd") if k in kw}
    region = cfa_tiff(tiff_width or w, height, v["configuration"]["bits"], STRIP_AT, len(strip), **tiff_kw) + strip
    raf(name, camera=camera, cfa=region, recs=records(recs), **kw)


def plain(name: str, width: int = 64, height: int = 48, *, camera: bytes = b"X-E2", strip_bytes: int | None = None,
          strip_at: int = STRIP_AT, cut: int = 0, **kw) -> None:
    samples = uncompressed(width, height)
    count = 2 * width * height if strip_bytes is None else strip_bytes
    tiff_kw = {k: kw.pop(k) for k in ("omit", "raw_ifd") if k in kw}
    region = cfa_tiff(width, height, 14, strip_at, count, **tiff_kw) + samples[:count]
    region = region[:len(region) - cut] if cut else region
    recs = kw.pop("recs", records([(0x0100, struct.pack(">HH", height, width)), (0x0130, b"\x0c\x0c\x0c\x0c")]))
    raf(name, camera=camera, cfa=region, recs=recs, **kw)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for p in OUT.glob("raf_*.raf"):
        p.unlink()

    # accepted
    plain("raf_uncompressed")
    compressed("raf_lossless_xtrans", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3")
    compressed("raf_lossy_bayer", "gfx50sii_lossy_bayer_b0_h24", camera=b"GFX50S II")
    compressed("raf_lossy_bayer16", "gfx100s_lossy_bayer16_b0_h24", camera=b"GFX100S")
    # two stripes, the last 24 columns inside the image (a width a multiple of 24, as LibRaw requires)
    compressed("raf_lossless_xtrans_narrow_last", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3",
               blocks=2, width=STRIPE + 24)

    # rejected, one per check of spec/zarrify/raf.md §8.1
    # <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#81-the-header>, in its order
    (OUT / "raf_reject_short.raf").write_bytes(b"FUJIFILMCCD-RAW " + bytes(100))
    plain("raf_reject_region_outside", words={3: 1 << 20})
    plain("raf_reject_overlap", words={1: 1000})
    plain("raf_reject_second_frame", words={11: 4096})
    plain("raf_reject_records_length", recs=b"\0\0")
    plain("raf_reject_records_count", recs=records([(0x0100, b"\0\0\0\0")], count=300))
    plain("raf_reject_records_past", recs=records([(0x0100, b"\0\0\0\0")], count=2))
    raf("raf_reject_not_tiff", camera=b"FinePix S5600", cfa=bytes(64), recs=records([(0x0100, b"\0\x08\0\x08")]))
    plain("raf_reject_no_raw_ifd", raw_ifd=False)
    plain("raf_reject_raw_tag", omit=0xF003)
    plain("raf_reject_strip", strip_at=4)
    plain("raf_reject_packed", strip_bytes=64 * 48 * 3 // 2)
    plain("raf_reject_samples_past", cut=2)
    compressed("raf_reject_lossless_flag", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3", patch={2: 2})
    compressed("raf_reject_raw_type", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3", patch={3: 1})
    compressed("raf_reject_bits", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3", patch={4: 10})
    compressed("raf_reject_block", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3", patch={11: 2, 12: 0})
    compressed("raf_reject_height", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3", patch={6: 7})
    compressed("raf_reject_width", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3", width=STRIPE + 1)
    compressed("raf_reject_size_mismatch", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3", tiff_width=STRIPE - 24)
    compressed("raf_reject_xtrans_pattern", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3", pattern=False)
    # the CFA region ends 8 bytes into the strip's 16 bytes of tables
    xe3 = len((OUT / "xe3_lossless_xtrans_b0_h24.chunk").read_bytes())
    compressed("raf_reject_tables_past", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3", shrink=16 + xe3 - 8)
    compressed("raf_reject_stripes_past", "xe3_lossless_xtrans_b0_h24", camera=b"X-E3", shrink=1)


if __name__ == "__main__":
    main()
