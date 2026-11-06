# RAF fixtures

Test vectors of the codec `refkv.fuji_compressed`
([spec/conventions.md §13](../../spec/conventions.md#13-fujifilm-compressed-stripes)), cut from
three real files of [raw.pixls.us](https://raw.pixls.us/), whose files are dedicated to the public
domain ([CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/)).

A vector is the first bytes of a stripe (a block of the file's compressed sensor data), enough
for its first 6 or 12 rows: the decoder reads a stripe in order, so a chunk of that height made
of them decodes to those rows. Each is the shortest such prefix: one byte fewer decodes otherwise.
`vectors.json` gives each vector's configuration and chunk shape; `<name>.chunk` is the chunk's
bytes, and `<name>.expected` the rows rawpy 0.28 (LibRaw 0.22) gives, `raw_image[:h, 768 k : 768 k
+ 768]` for block `k`, as little-endian `uint16`. The `cfa` is rawpy's `raw_pattern`, from the
stripe's first row and column, its second green 3 written as 1.

| vector | source | sha256 of the source | bytes of the source |
|---|---|---|---|
| `xe3_lossless_xtrans_b0_h6` | [`DSCF2176.RAF`](https://raw.pixls.us/download/data/Fujifilm/X-E3/DSCF2176.RAF), Fujifilm X-E3, lossless, X-Trans, 14 bits, 27,799,280 bytes | `03d6e79ae79107c51d0e1c1c9e488a0ab20268cd9fe43f47d8ea3ea310df35c6` | block 0: 868912–872287 |
| `gfx50sii_lossy_bayer_b0_h12` | [`Fuji_GFX50S_II_-_Compressed.RAF`](https://raw.pixls.us/download/data/Fujifilm/GFX50S%20II/Fuji_GFX50S_II_-_Compressed.RAF), Fujifilm GFX 50S II, lossy, Bayer, 14 bits, 34,522,544 bytes | `59699d3063a9900eb5f226da664473c6081ed2cfe4d4529b7cd45afa2f1cac62` | block 0: `q_base` 3291712–3291713, then 3304192–3311566 |
| `gfx100s_lossy_bayer16_b0_h6` | [`Fujifilm-GFX100S-16bits-compress-4_3.RAF`](https://raw.pixls.us/download/data/Fujifilm/GFX100S/Fujifilm-GFX100S-16bits-compress-4_3.RAF), Fujifilm GFX100S, lossy, Bayer, 16 bits, 73,013,616 bytes | `1bb4608b61861a3d9e85b0ca6c00689b8ba4620cbe8579374d958c21ed08edf0` | block 0: `q_base` 3449424, then 3472976–3478004 |

The byte ranges are inclusive. A lossy vector's chunk starts with 16 bytes: the `q_base` of its row
groups, then zeros.
