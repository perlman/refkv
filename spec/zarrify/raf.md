# RAF

The Zarr layout of a Fujifilm RAF file (`.raf`), a camera's raw photo, and the translation of its
header and metadata records into JSON. What all of refkv's conventions share is in
[spec/conventions.md](../conventions.md), cited here as "conventions §n". How refkv produces this
layout as a reference store, and which files it accepts, is the RAF profile,
[Part 2](#part-2-the-profile) below.

Convention version: 0 (until release, [conventions §1](../conventions.md#1-conventions)) · UUID: `53faefeb-140e-454f-b960-72ecae5170c0` ·
Schema: [schema.json](raf/schema.json)

Changes: [changes/raf/](changes/raf/), one file per change.

Added 2026-11-06.

This document has two parts ([conventions §11.1](../conventions.md#111-parts)). It covers the
files whose sensor data is compressed, lossless or lossy, or uncompressed in 16 bits: every X-series
camera from the X-E2 and X-T1 on, and every GFX. The design, and the research on 119 public files of
82 models behind it, are in [design/plans/raf.md](../../design/plans/raf.md).

# Part 1. The convention

## Layout

The hierarchy's entries ([conventions §11.5](../conventions.md#115-layout-tables)):

| prefix | type | structure | description |
|---|---|---|---|
| `zarr.json` | group | [`ImageGroup`](../conventions.md#type-imagegroup) | the root: the image ([§4](#4-the-image)) |
| `zarr.json#/attributes/ome` | attribute | [`OmeImage`](../conventions.md#type-omeimage) | one level, the array `0`, of scale 1 along `y` and `x`, and `omero` as [§4.2](#42-the-channel) says |
| `zarr.json#/attributes/zarr_conventions` | attribute | array of [`ConventionMetadata`](../conventions.md#type-conventionmetadata) | the CMO of [§1](#1-declaration) |
| `zarr.json#/attributes/refkv_zarrified` | attribute | [`RootProperty`](../conventions.md#type-rootproperty) | `profile` `"raf"`, `version` `0`, and `source.url` the file's URL |
| `zarr.json#/attributes/refkv_zarrified/raf` | attribute | [`RafRoot`](#type-rafroot) | the root's source metadata ([§5.1](#51-the-root)) |
| `0/zarr.json` | array | [`RafArray`](#type-rafarray) | the sensor's samples ([§4.1](#41-the-array)) |
| `0/c/<y>/<x>` | chunk | [`RafChunk`](#type-rafchunk) or [`RafStripeChunk`](#type-rafstripechunk) | every chunk of the array ([§8.3](#83-chunks)) |
| `refkv_source/**` | subtree | [the mirror's entries](../conventions.md#table-mirror) | the IR mirror ([§5.2](#52-the-ir-mirror)): its elements are [§5.3](#53-elements)'s |

| placeholder | value |
|---|---|
| `<y>`, `<x>` | the chunk's coordinates along `y` and `x`, in decimal |

## 1. Declaration

The root and `refkv_source` declare the convention
([conventions §11.2](../conventions.md#112-declaration)), the root with its source metadata of
[§5.1](#51-the-root) as the member `"raf"`. Its CMO is:

```json
{
  "uuid": "53faefeb-140e-454f-b960-72ecae5170c0",
  "schema_url": "https://raw.githubusercontent.com/d-v-b/refkv/refs/heads/main/spec/zarrify/raf/schema.json",
  "spec_url": "https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md",
  "name": "refkv_zarrified",
  "description": "The Zarr layout of a Fujifilm RAF source zarrified by refkv, and the source's metadata"
}
```

The array has no attributes.

## 2. The file

A RAF file starts with a **header** of 132 bytes, big-endian:

| field | offset | type | meaning |
|---|---|---|---|
| `magic` | 0 | `char[16]` | `FUJIFILMCCD-RAW ` |
| `format_version` | 16 | `char[4]` | `0201` (`0200` in the S2Pro's) |
| `camera_id` | 20 | `char[8]` | the camera's identifier |
| `camera` | 28 | `char[32]` | the camera's name, padded with NULs |
| `directory_version` | 60 | `char[4]` | |
| `reserved` | 64 | `bytes[20]` | |
| `directory` | 84 | `u32[12]` | the **directory**, below |

| word | name | meaning |
|---|---|---|
| 0, 1 | `jpeg_offset`, `jpeg_length` | the embedded JPEG, a preview with the photo's EXIF |
| 2, 3 | `metadata_offset`, `metadata_length` | the **metadata records** |
| 4, 5 | `cfa_offset`, `cfa_length` | the **CFA region**, which holds the sensor data |
| 6 | `compression` | 0 uncompressed, 2 lossless, 3 lossy, in every file surveyed |
| 7 | `cfa_length_2` | the CFA region's length again, or 0 |
| 8 | `word_8` | |
| 9, 10, 11 | `frame_2_metadata_offset`, `frame_2_metadata_length`, `frame_2_cfa_offset` | a second frame's, in EXR and SuperCCD SR cameras' files |

The **metadata records** are a big-endian `u32` count, then each record: its tag (`u16`), its length
`l` (`u16`) and `l` bytes, which carry no type. Among them: `0x0100`, the raw size (height and
width, `u16` each); `0x0110` and `0x0111`, the crop's top left and size; `0x0130`, the sensor's
layout; `0x0131`, the X-Trans pattern ([§3](#3-the-sensor-data)); `0x2ff0`, the white balance; and
`0x4000`, the black levels.

The **CFA region** is a TIFF (TIFF 6.0), its offsets from the region's start. Its IFD 0 has the
tag `0xF000`, the offset of the **raw IFD**, whose one-count SHORT or LONG tags give the sensor
data's width `w` (`0xF001`), height `h` (`0xF002`) and bits (`0xF003`), and the **strip**'s offset
`s` (`0xF007`) and byte count (`0xF008`), from the region's start.

## 3. The sensor data

The strip holds the sensor's samples, one color each through the sensor's color filter array, in
one of two encodings this convention presents:

- **Uncompressed**, when its byte count is `2 w h`: the samples row by row, `uint16`, in the TIFF's
  byte order.
- **Compressed**, when it starts with `IS`: a 16-byte header, a table of block sizes, for lossy
  data a run of `q_base` bytes per block, then the blocks, each a **stripe** of 768 columns over the
  full height that decodes on its own
  ([conventions §13](../conventions.md#13-fujifilm-compressed-stripes)). The header gives
  whether the data is lossless (byte 2), the raw type (byte 3: 0 Bayer, 16 X-Trans), the bits
  (byte 4), and, big-endian `u16`, the height (bytes 5–6), the width rounded to whole stripes (7–8),
  the width (9–10), the stripes' width (11–12), the number of stripes (byte 13) and of row groups
  (14–15). The blocks' sizes are big-endian `u32`, the table padded to 16 bytes; each `q_base` run
  is `16 × ⌈n / 16⌉` bytes, `n` the row groups.

A stripe codes each color in lines of its own, and decoding it places them by the **pattern** of
the color filter array, from the array's first row and column:

- An X-Trans stripe's is the 6 × 6 pattern of metadata record `0x0131`, which holds its 36 colors
  (0 red, 1 green, 2 blue) last first: entry `(y, x)` is the record's byte `35 − 6y − x`.
- A Bayer stripe's is red, green / green, blue: `[[0, 1], [1, 2]]`. **No file states it.** It is
  the pattern LibRaw and rawpy give every GFX model, the only cameras whose compressed data is Bayer,
  and matches their samples on every file checked.

## 4. The image

The hierarchy is one image at the root ([conventions §4](../conventions.md#4-images)), without a
name, with one array at path `"0"`: the root `zarr.json` is
`{"zarr_format": 3, "node_type": "group", "attributes": A}`, where `A` has `ome`,
`zarr_conventions` and `refkv_zarrified` ([§1](#1-declaration)). Its axes are `y` and `x`, of type
`space`, with scale 1 and no unit: the file states no pixel size.

### 4.1 The array

The array is the **raw image**: the sensor's `h × w` samples, margins included, as LibRaw's and
rawpy's `raw_image` gives them. It is not demosaiced: each sample is the one color the pattern
gives its position.

<a id="type-rafarray"></a>**`RafArray`**, the array's `zarr.json`: an
[`ArrayDocument`](../conventions.md#type-arraydocument) with these members:

| member | type | presence | value |
|---|---|---|---|
| `shape` | array of integer | required | `[h, w]` |
| `data_type` | string | required | `"uint16"` |
| `chunk_grid` | object | required | regular; uncompressed, the chunk shape [conventions §7](../conventions.md#7-source-values-as-arrays) gives contiguous values with the chunk limit 2^20 bytes; compressed, `[h, 768]`, one chunk a stripe |
| `fill_value` | number | required | `0` |
| `codecs` | array of object | required | uncompressed, `[{"name": "bytes", "configuration": {"endian": E}}]`, `E` the TIFF's byte order; compressed, `[{"name": "refkv.fuji_compressed", "configuration": {"bits": b, "lossless": L, "cfa": P}}]`, `b` and `L` from the stripes' header and `P` the pattern of [§3](#3-the-sensor-data) |
| `dimension_names` | array of string | required | `["y", "x"]` |
| `attributes` | object | required | `{}` ([§1](#1-declaration)) |

The last stripe of a file whose width is not a multiple of 768 decodes to 768 columns, and the
array's shape leaves out those past `w`.

### 4.2 The channel

The image has an `omero` member with one channel, `{"label": "raw", "color": "FFFFFF", "active":
true, "window": {"min": 0, "max": M, "start": 0, "end": M}}`, `M = 2^b − 1`, `b` the stripes'
bits for compressed data and the raw IFD's (`0xF003`) for uncompressed; and no `omero` when that
`b` is not from 1 to 16.

## 5. Source metadata

The root's source metadata `S` ([conventions §2](../conventions.md#2-attributes)) is an object with
the members of [§5.1](#51-the-root), in that order, translated by
[conventions §6](../conventions.md#6-source-metadata-as-json). Everything the file holds is on
`refkv_source`, the file's **IR mirror** ([§5.2](#52-the-ir-mirror)).

### 5.1 The root

<a id="type-rafroot"></a>**`RafRoot`**, the root's `S`:

| member | type | presence | value |
|---|---|---|---|
| `header` | object | required | every field of the header ([§2](#2-the-file)), in its order and named as there: a `char[k]` field as a [`Text`](../conventions.md#type-text), `reserved` as a [`Base64`](../conventions.md#type-base64), and `directory` as an array of 12 integers |
| `directory` | object | required | the directory's words, by the names of [§2](#2-the-file), in order |
| `records` | array of [`RafRecord`](#type-rafrecord) | required | every metadata record, in the file's order |
| `cfa_tiff` | [`RafTiff`](#type-raftiff) | required | the CFA region's TIFF |
| `encoding` | string | required | `"compressed"` or `"uncompressed"` |
| `byte_order` | string | optional | uncompressed: the samples' byte order, `"little"` or `"big"` |
| `stripes` | [`RafStripes`](#type-rafstripes) | optional | compressed: the stripes' header and sizes |
| `cfa` | [`RafCfa`](#type-rafcfa) | optional | compressed: the pattern the codec places the samples by |

<a id="type-rafrecord"></a>**`RafRecord`**:

| member | type | presence | value |
|---|---|---|---|
| `tag` | integer | required | the record's tag |
| `value` | [`Base64`](../conventions.md#type-base64) | required | its bytes |

<a id="type-raftiff"></a>**`RafTiff`**:

| member | type | presence | value |
|---|---|---|---|
| `byte_order` | string | required | `"little"` (`II`) or `"big"` (`MM`) |
| `ifds` | array of object | required | IFD 0, then the raw IFD: each `{"offset": o, "entries": [...]}`, `o` from the region's start, and each entry `{"tag", "type", "count", "value"}`: `value` the integer of a one-count SHORT, LONG or IFD entry, else the [`Base64`](../conventions.md#type-base64) of its 4 value bytes |

<a id="type-rafstripes"></a>**`RafStripes`**: the stripes' header ([§3](#3-the-sensor-data)) as
integers, `lossless`, `raw_type`, `bits`, `height`, `rounded_width`, `width`, `block_size`,
`blocks` and `row_groups`, then `block_sizes`, an array of each block's bytes.

<a id="type-rafcfa"></a>**`RafCfa`**:

| member | type | presence | value |
|---|---|---|---|
| `pattern` | array of array of integer | required | the codec's `cfa` ([§3](#3-the-sensor-data)) |
| `from` | string | required | `"file"` for an X-Trans pattern, from record `0x0131`; `"readers"` for the Bayer pattern no file states |

### 5.2 The IR mirror

`refkv_source` is the file's **IR mirror** ([conventions §11.3](../conventions.md#113-the-ir-mirror)).
Its elements (the RAF **source model**) are below; nothing is left out.

### 5.3 Elements

Paths are relative to the IR's root ([conventions §8.1](../conventions.md#81-elements)); types are
those of [conventions §8.6](../conventions.md#86-types). `J`, `M` and `C` are the embedded JPEG's,
the metadata records' and the CFA region's offsets, `s` the strip's offset in the CFA region.

| path | kind | type | extent: what |
|---|---|---|---|
| `""` | struct | | the root: the whole file |
| `header` | value | record | `(0, 132)`: the header, `magic:cstr[16]`, `format_version:cstr[4]`, `camera_id:cstr[8]`, `camera:cstr[32]`, `directory_version:cstr[4]`, `reserved:bytes[20]`, `directory:>u4[12]` |
| `jpeg` | value | `bytes[n]` | `(J, n)`, when `n`, the JPEG's length, is not 0: the embedded JPEG, never read |
| `metadata` | value | `bytes[n]` | `(M, n)`: the metadata records |
| `cfa_tiff` | value | `bytes[s]` | `(C, s)`: the CFA region up to its strip |
| `samples` | data | | uncompressed, `(C + s, 2 w h)`: its form's geometry `{"data_type": "uint16", "samples": 1}`, codec `{"endian": E}`, recipe `[["src", 0, null]]` |
| `stripe_header` | value | `bytes[16]` | compressed, `(C + s, 16)` |
| `stripe_tables` | value | `bytes[t]` | compressed, `(C + s + 16, t)`: the blocks' sizes, and the `q_base` runs of lossy data |
| `stripes` | data | | compressed, the blocks, from `C + s + 16 + t`: its form's geometry `{"data_type": "uint16", "shape": [h, w]}`, codec `{"refkv.fuji_compressed": configuration, "sizes": [...]}`, recipe `[["src", 0, null]]` |
| `gaps/<offset>` | gap | | bytes no other element claims |

### 5.4 Equivalence

Hierarchies of one file are compared entry for entry, `refkv_source` included
([conventions §11.4](../conventions.md#114-equivalence)).

## 6. Example

A lossless X-Trans file of an X-E3 has `h = 4038`, `w = 6048`, 14-bit samples, and 8 stripes. Its
array is `[4038, 6048]` of `uint16`, chunk shape `[4038, 768]`, with the codec
`{"name": "refkv.fuji_compressed", "configuration": {"bits": 14, "lossless": true, "cfa": [[1, 1, 0, 1, 1, 2], [1, 1, 2, 1, 1, 0], [2, 0, 1, 0, 2, 1], [1, 1, 2, 1, 1, 0], [1, 1, 0, 1, 1, 2], [0, 2, 1, 2, 0, 1]]}}`.
The chunk `0/c/0/k` is block `k`'s range of the file, and its `omero` window is 0 to 16383.

## 7. What the layout does not express

This section is informative.

- **The colors.** The array is the mosaic, not an RGB image: demosaicing, white balance, black
  levels and the color matrix are a raw developer's. The pattern is in the codec's configuration
  for compressed data, and the records hold the white balance and black levels; for uncompressed
  data no structure states the pattern, and the layout gives none.
- **The crop.** Records `0x0110` and `0x0111` give the area a developer shows; the array keeps the
  whole raw image, margins included. Readers' own crops are model tables.
- **The embedded JPEG** and its EXIF, kept in the mirror, are not decoded.
- **The meaning of most records.** They are kept as bytes; only the X-Trans pattern is read.

# Part 2. The profile

The RAF profile of [spec/zarrify.md](../zarrify.md) is [§8](#8-raf-profile) of this document. What
every profile's Part 2 shares is in [spec/zarrify.md, Profiles](../zarrify.md#profiles).

## 8. RAF profile

### 8.1 The header

The input is a RAF file, which its **signature** ([spec/zarrify.md §1.2](../zarrify.md#12-input),
tested at the precedence of [profiles/raf.json](profiles/raf.json)) chose: its first 16 bytes are
`FUJIFILMCCD-RAW `.

**Reads.** First the header, the file's first 132 bytes. Then, in one round, the metadata records
and the CFA region's first 65,536 bytes. Then, when that did not reach them, the strip's first 16
bytes and a compressed strip's tables. No other byte is read.

**Checks.** The input is rejected unless all of these hold, checked in this order:

1. **File:** the file has at least 132 bytes: `file too short for a 132-byte RAF header`.
2. **Regions:** the embedded JPEG, the metadata records and the CFA region, each of its directory's
   length when not 0, lie within the file after the header: `the <region> of <length> bytes at
   <offset> is outside the <size>-byte file past its header`.
3. **Overlap:** no two of them share a byte: `the <a> at <offset> and the <b> at <offset> overlap`.
4. **Second frame:** word 11 is 0: `a second frame, its CFA region at <offset> (an EXR or SuperCCD
   camera's), which the profile does not read`.
5. **Records:** their length is from 4 to 1,048,576 bytes (`metadata records of <length> bytes, not
   from 4 to 1048576`), their count at most 255 (`<count> metadata records, more than 255`), and
   every record within them (`the <count> metadata records run past their <length> bytes`).
6. **TIFF:** the CFA region starts with a TIFF header and IFD 0 (`the CFA region at <offset> is not
   a TIFF (an older FinePix camera's bare samples), which the profile does not read`), whose tag
   `0xF000` gives a raw IFD within the region's first 65,536 bytes (`the CFA region's TIFF has no
   raw IFD (tag 0xF000) within its first 65536 bytes`).
7. **Raw tags:** the raw IFD has each of `0xF001`, `0xF002`, `0xF003`, `0xF007` and `0xF008`, one
   SHORT or LONG: `the raw IFD has no <meaning> (tag <tag>, one SHORT or LONG)`.
8. **Strip:** the strip's offset `s` is at least 8, at most 65,536, and `s + 16` at most the CFA
   region's length: `the strip at <s> of the <length>-byte CFA region is not within its first 65536
   bytes, after its TIFF header, with 16 bytes`.
9. **Encoding:** the strip starts with `IS`, or its byte count is `2 w h`, with `w h` from 1 to
   2^53 − 1: `a <bytes>-byte strip of <w> × <h> samples, <bytes / (w h)> bytes a sample: packed
   samples, which the profile does not read`. (12- and 14-bit packed files are those of the X-Pro1,
   X-E1, X-A and X-T100/X-T200 lines, the X10, X-S1, X100 and FinePix models.)

Then, for uncompressed samples:

10. **Samples:** `C + s + 2 w h` is at most the CFA region's end: `the uncompressed samples end at
    <end>, past the CFA region's end at <end>`.

Or, for compressed ones, from the stripes' header ([§3](#3-the-sensor-data)):

10. **Stripes:** the lossless flag is 0 or 1; the raw type 0 or 16; the bits 12, 14 or 16; the
    stripes' width 768; the height a multiple of 6 from 6 to 16,386, with as many row groups as
    sixths; at least one stripe, the rounded width the stripes' count times 768, and the width from
    1 to the rounded width, less than 768 short of it; and the width and height the raw IFD's. Each
    has its message in the schema.
11. **Pattern:** an X-Trans strip has metadata record `0x0131` of 36 bytes, each 0, 1 or 2: `an
    X-Trans strip without its pattern (metadata record 0x0131 of 36 bytes, each 0, 1 or 2)`.
12. **Stripes' data:** the tables, then the blocks of the sizes they give, end within the CFA
    region: `a compressed strip's <t> bytes of tables run past the CFA region's end at <end>`, or
    `the compressed stripes end at <end>, past the CFA region's end at <end>`.

No other field is checked. A file is not rejected for its records' contents, and the records LibRaw
reads by model are kept, not applied.

### 8.2 The strip

The strip starts at `C + s`. Uncompressed, the samples are its first `2 w h` bytes. Compressed, the
stripes' header is its first 16 bytes, then `t` bytes of tables: `16 × ⌈B / 4⌉` of block sizes (`B`
blocks; `4B` rounded up to 16), then, lossy, `B` runs of `16 × ⌈n / 16⌉` `q_base` bytes. Block `k`
starts after the tables and the blocks before it.

### 8.3 Chunks

**Uncompressed:** the samples are the row-major array of `[h, w]` `uint16` values at `C + s`, cut
as contiguous values with the chunk limit 2^20 bytes and referenced as
[mrc.md §8.2](mrc.md#82-chunks) references a chunk: one range of source 0, followed for an edge chunk
by its padding, a range of the output's zero data source, or copied where [conventions §7](../conventions.md#7-source-values-as-arrays) copies.

<a id="type-rafchunk"></a>**`RafChunk`**, an uncompressed chunk:

| part | range | presence | value |
|---|---|---|---|
| the samples | source range `(0, o, l)` | required | the chunk's bytes in the file |
| the padding | source range `(1, 0, p)` | optional | an edge chunk's zero padding |

**Compressed:** the chunk `0/c/0/k` is stripe `k`:

<a id="type-rafstripechunk"></a>**`RafStripeChunk`**:

| part | range | presence | value |
|---|---|---|---|
| the `q_base` run | source range `(0, q_k, 16 × ⌈n / 16⌉)` | lossy | block `k`'s run of `q_base` bytes |
| the stripe | source range `(0, b_k, z_k)` | required | block `k`'s bytes |

The output's source table is source 0, and the zero data source when an uncompressed reference is
padded.

### 8.4 Summary

This section is informative. Both implementations print a one-line JSON summary, with these
members:

| member | value |
|---|---|
| `camera` | the header's `camera` |
| `encoding` | `"compressed"` or `"uncompressed"` |
| `sensor` | `"bayer"` or `"x-trans"`, compressed; `null` otherwise |
| `lossless` | whether compressed data is lossless; `null` uncompressed |
| `bits` | the samples' bits |
| `sizes` | `{"y": h, "x": w}` |
| `chunkShape` | the array's chunk shape |
| `chunks` | the number of the array's chunks |
