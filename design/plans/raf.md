# RAF: implementation plan

The plan for a profile of Fujifilm's raw format, RAF. Not normative: the convention and the profile,
in `spec/zarrify/raf.md`, will state the decisions, and this plan is amended where the code decides
otherwise ([adding a format §2](../../docs/adding-a-format.md#2-research-the-format-and-its-files)).

## 1. Research summary

Claims marked **[inspected]** were checked against real bytes in November 2026: 119 files, up to
three a model, from all 82 Fujifilm models on [raw.pixls.us](https://raw.pixls.us/data/Fujifilm/)
(CC0), read over range requests with a header-only survey script. Two files were also decoded with
rawpy 0.22 (LibRaw 0.22). The reference sources read were LibRaw (`src/metadata/fuji.cpp`,
`src/metadata/identify.cpp`, `src/utils/open.cpp`, `src/decoders/fuji_compressed.cpp`) and rawspeed
(`parsers/FiffParser.cpp`, `decoders/RafDecoder.cpp`, `decompressors/FujiDecompressor.cpp`,
`data/cameras.xml`).

### 1.1 The container

- **Header [inspected].** Every file starts with the 16 bytes `FUJIFILMCCD-RAW `, then a 4-byte
  format version (`0201` in all but the S2Pro's `0200`), an 8-byte camera ID, the camera name in 32
  NUL-padded bytes, and a 4-byte directory version. At offset 84 is a directory of big-endian
  `uint32` words:

  | offset | word | meaning |
  |---|---|---|
  | 84 | 0 | embedded JPEG offset |
  | 88 | 1 | embedded JPEG length |
  | 92 | 2 | metadata records offset |
  | 96 | 3 | metadata records length |
  | 100 | 4 | CFA region offset |
  | 104 | 5 | CFA region length |
  | 108 | 6 | compression: 0 none, 2 lossless, 3 lossy **[inspected]**, matched by the CFA stream's own header in all 37 compressed files |
  | 112 | 7 | the CFA region's length again, in all but 10 files, where it is 0 |
  | 120, 124, 128 | 9–11 | a second frame's metadata offset, length and CFA offset (EXR models, S3Pro, S5Pro) |

  LibRaw reads word 9 as a second frame's metadata (its `shot_select`), and word 11 as its CFA
  region (`100 + 28`).
- **Metadata records [inspected].** A big-endian count, then records of `tag: u16`, `length: u16`
  and `length` bytes, which are in place and carry no type. 159 distinct tags in the survey. The ones
  the readers use: `0x0100` raw size (height, width), `0x0110` crop top left, `0x0111` cropped
  size, `0x0115` aspect ratio, `0x0121` image size (older models), `0x0130` layout, `0x0131` X-Trans
  pattern (36 bytes, stored reversed: LibRaw writes byte `c` to position `35 − c`), `0x2ff0` white
  balance, `0x4000` black levels, `0x9650` exposure bias, `0xc000` a model-specific table.
- **The CFA region** takes two forms:
  - **A TIFF [inspected]** (`II*\0`) in 101 of the files: every X-series and GFX model, and the
    FinePix EXR, HS and X10/X100 models. IFD 0 holds tag `0xF000`, a sub-IFD with `0xF001` width,
    `0xF002` height, `0xF003` bits per sample, `0xF007` strip offset and `0xF008` strip byte count
    (both relative to the region), plus `0xF009`–`0xF015`. One strip, at offset 2048, or 1024 for
    the X-A3 family. A compressed file's CFA region ends at the end of the file.
  - **Bare samples [inspected]** in the 18 other files: older FinePix models, from the S1 to the
    S200EXR.
- **The embedded JPEG** holds the EXIF, with the Fujifilm maker note.

### 1.2 The sensor data, in five encodings

Each was told apart by the strip's byte count against `0xF001 × 0xF002`, as both readers do:

| encoding | test | surveyed | models |
|---|---|---|---|
| 16 bits, little endian | count = 2 w h | 43 | X-E2 on, X-T1 on, GFX uncompressed |
| 12 bits packed, least significant bit first | count = 1.5 w h | 23 | X-Pro1, X-E1, X-A1, X-A2, X-M1, X10, X-S1, X100, FinePix EXR and HS |
| 12 or 14 bits packed, in rawspeed's `MSB32` bit order (not yet checked against data) | count = 1.5 w h or 1.75 w h | 10 | X-A3, X-A5, X-A7, X-A10, X-T100, X-T200, XF10, FinePix S1, HS10, SL1000 |
| Fujifilm compressed, lossless or lossy | strip starts `IS` (`0x4953`) | 37 | X-T2 on, GFX 50S on |
| bare samples, with SuperCCD layouts | not a TIFF | 18 | older FinePix |

- **16 bits [inspected]:** the strip is the raw image row by row.
- **12 bits, least significant bit first [inspected]:** element `i` is bits `[12 i, 12 i + 12)` of
  the strip. On an X-A1 file this unpacking equals rawpy's `raw_image` value for value. It is
  zarr-extensions' `packbits` on `uint16` with `last_bit: 11`.
- **32-bit word order:** both readers can't tell this from the bytes. rawspeed applies its
  `jpeg32_bitorder` hint by camera model in `cameras.xml`. LibRaw takes 1.75 bytes a pixel as its
  `fuji_14bit_load_raw`. No existing codec decodes it.
- **Compressed [inspected]:** a 16-byte header (`0x4953`, lossless flag, raw type 0 Bayer or 16
  X-Trans, bits 12, 14 or 16, height, rounded width, width, block size, blocks in a row, line
  groups), then one big-endian `uint32` size per block, padded to 16 bytes, then for lossy a `q_base`
  byte per line group per block (each block's run padded to 16), then the blocks in order.
  - The block size is 768 in every file, and LibRaw rejects any other. `rounded width = blocks ×
    768`, and the height is a multiple of 6.
  - **Each block is a vertical stripe of 768 columns over the full height, decoded on its own.**
    LibRaw decodes them in parallel (`fuji_decode_loop`), each from its own offset and size, sharing
    only the header's parameters and its own run of `q_base` bytes. The decoder writes all 768
    columns of the last stripe, and copies only those within `width`.
  - On an X-E3 file the header's (height, width) equals rawpy's `raw_image` shape.
  - Lossy and lossless share the decoder. Lossy files turned up for 12 models, from the X-T4 and
    GFX 50S II on **[inspected]**.
- **SuperCCD [inspected]:** layout record `0x0130`, byte 1, bit 3 clear means LibRaw's `fuji_width`,
  a sensor sampled on a 45° grid that readers rotate. This holds for the S2Pro, S3Pro, S5Pro, S100FS,
  S6000fd, S6500fd, F700, E550, E900, S5000, S5200, S5600, S7000, S9500, S9600 and DBP for GX680.

### 1.3 Findings that shaped the design

1. Compressed RAF fits: each stripe is a unit that decodes on its own, on a regular grid, and the
   last one decodes to the full chunk shape, so it needs no padding
   ([TECHNIQUES §2](../../spec/TECHNIQUES.md#2-chunks-from-the-sources-own-units)).
2. Its chunks are large: height × 768 × 2 bytes, 6.2 MB on an X-T3, 13.4 MB on a GFX 100. Stripes
   can't be cut further, since each line group depends on the one before it.
3. Uncompressed 16-bit and 12-bit least-significant-first files need no new codec.
4. The 32-bit word order is chosen by camera model in both readers, not by the structure. A profile
   would have to carry a model table. These ten models are left for later (open question 3).
5. **The Bayer pattern is not in the file.** LibRaw takes it from a table keyed by raw width, and
   rawspeed from `cameras.xml`. The X-Trans pattern is in record `0x0131`. A pattern the file
   doesn't state is not written ([TECHNIQUES §3](../../spec/TECHNIQUES.md#3-axes-grids-and-coordinates)).
6. The default crop of both readers is model-specific too (LibRaw's `raw_width` cases). The file
   states a crop in `0x0110`/`0x0111`, which is recorded, not applied.
7. EXR and SuperCCD SR files hold two frames. Both are kept (record, don't reject).
8. raw.pixls.us serves ranges with ETags, but no CORS headers, so the demo needs another host.

## 2. Decisions

- **One input is one file.** Profile `raf`, signature the 16 bytes at offset 0, within the first 552
  bytes ([zarrify §1.2](../../spec/zarrify.md#12-input)).
- **The hierarchy** (to settle in the document):
  - the root group, with the source metadata: header, directory, every metadata record as written
    (hex), the decoded records above, and the CFA TIFF's tags;
  - `raw/0`: the sensor data as `[y, x]` `uint16`, the full raw area as both readers' `raw_image`
    gives it, margins included, with the CFA pattern where the file states it;
  - `raw/1`: the second frame, when there is one;
  - `preview`: the embedded JPEG, one chunk with `imagecodecs_jpeg`, `[y, x, c]`. Its size comes
    from the JPEG's SOF.
  - A frame the profile can't present (open question 3) is listed with its reason, and the preview
    and metadata are still written.
- **Chunks:**
  - 16 bits: whole rows up to 2^20 bytes, `bytes` little endian.
  - 12 bits: whole rows up to 2^20 bytes, `packbits` (`uint16`, `last_bit: 11`). refkv's Python
    `packbits` supports only data types up to 8 bits, so it must accept `uint16`. Check the
    Neuroglancer fork's `packbits` too.
  - Compressed: one chunk a stripe, regular grid `[height, 768]` over `[height, width]`, with the new
    codec `refkv.fuji_compressed`, specified in
    [conventions §13](../../spec/conventions.md#13-fujifilm-compressed-stripes). Its configuration
    holds the bits, lossless or lossy, and the CFA pattern. A lossless chunk is the stripe's range.
    A lossy one is two ranges: the stripe's run of `q_base` bytes, then its coded bytes. No literal
    is needed, and the decoder's memory is bounded by the chunk.
    - §13 was checked before any implementation: a throwaway Python decoder written from its text
      alone matched rawpy's `raw_image` value for value on lossless X-Trans (X-E3) and lossy Bayer
      (GFX 50S II) stripes, the first and the last.
- **Rejected:** a CFA region that is not a TIFF (the bare-sample FinePix files), a TIFF without
  `0xF000`, a strip count that fits no encoding, a compressed header that fails LibRaw's validation
  (block size 768, width a multiple of 24, height of 6, sizes within the region), and every range
  outside the file.
- **The IR mirror** holds every byte but the strips and the JPEG: the header, the metadata records,
  the CFA TIFF up to its strip, and the compressed header, block table and `q_base` runs.
- **The oracle is rawpy (LibRaw) `raw_image`**, value for value. rawspeed is the second opinion where
  they differ. Known LibRaw special cases to record if real files show them: an X-H2S that states the
  wrong strip size (`open.cpp`), and record `0x0121`'s width 4284 read as 4287.
- **OME-NGFF:** `raw/0` as a one-level image with axes `y`, `x`, no units (the file states no pixel
  size), and an `omero` window of 0 to `2^bits − 1` from `0xF003`.

## 3. Order of work

Each step leaves a working subset.

1. **The profile, uncompressed** (one PR): the spec document, the registry entry, the parser, the
   projection, the 16-bit and 12-bit encodings, the preview, the source metadata and mirror, and
   compressed and 32-bit-order frames listed as not presented. Hand-built fixtures (struct writer,
   small sizes), the rejects table, `js/test/raf/verify.py` against rawpy, and corpus entries.
2. **`refkv.fuji_compressed`, lossless** (one PR, plus one in the Neuroglancer fork): the Rust
   decoder, the Python codec and its entry point, the wasm export, its section in
   `spec/conventions.md` and rows in the codec tables. The profile then presents compressed frames.
3. **Lossy** in the same codec, with the `q_base` prefix.
4. **Later, if wanted:** the 32-bit word order (a codec, and a model table), second frames on real
   EXR files, and bare-sample FinePix files that aren't SuperCCD.

## 4. Open questions

1. **License of the compressed decoder.** LibRaw is LGPL 2.1 or CDDL 1.0, and rawspeed LGPL 2.1.
   refkv is MIT or Apache 2.0, and a line-by-line port would be a derived work. *Recommended:* write
   the algorithm into the codec's section of `spec/conventions.md` in refkv's own words. Then
   implement it in a session that reads only that section and has never read either library's
   source. Check the result against rawpy. This session read LibRaw's `fuji_compressed.cpp`, so it
   should write the spec section, not the decoder.
2. **Fixtures for the codec.** No public tool writes a small compressed RAF, and real files are 15
   to 130 MB. *Recommended:* an encoder in the fixture generator, written from the same spec section,
   for a few small stripes (Bayer, X-Trans, 12/14/16 bits, lossy). Decode them with rawpy as well as
   refkv, so the encoder can't hide a shared misreading. Real files go in the corpus.
3. **The 32-bit word order.** *Recommended:* leave it out of the first steps and list those frames
   as not presented, naming the camera. Adding it later needs a codec and a model table, which would
   be the first model table in a refkv profile.
4. **The Bayer pattern.** For uncompressed frames, *recommended:* write none, as no structure states
   it, and say so in "What the layout does not express". The alternative is LibRaw's width table,
   which is a default, not data. **Compressed Bayer frames (every GFX) can't avoid it:** a stripe
   codes each color in lines of its own, and placing them needs the pattern
   ([conventions §13](../../spec/conventions.md#13-fujifilm-compressed-stripes), step 8). Neither
   the metadata records nor the EXIF (no `CFAPattern`) of the GFX 50S II file checked states one.
   LibRaw and rawpy give RGGB from LibRaw's tables. **Decided 2026-11-06:** compressed Bayer frames
   are presented with RGGB, the pattern both readers give every GFX model, and the document says
   it is the readers' and not the file's.
5. **The preview.** *Recommended:* keep it as an array. It costs one reference and lets a viewer show
   the photo without decoding raw data. Its EXIF stays inside its bytes and is not parsed into source
   metadata in step 1.
6. **Demo host.** raw.pixls.us has no CORS. *Recommended:* a small CC0 file mirrored on a host that
   sends CORS headers, as other examples do.
