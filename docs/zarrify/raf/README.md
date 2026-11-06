# RAF

refkv turns an image file into an OME-Zarr dataset by writing a small archive of references to
the file's bytes, without copying the pixels ([overview](../README.md)). This page is about
Fujifilm's raw photos.

## Who this is for

A RAF file (`.raf`) is what a Fujifilm camera writes when set to raw: the sensor's samples before
demosaicing, with an embedded JPEG preview and the camera's metadata. Every X-series camera from
the X-E2 and X-T1 on, and every GFX, writes its samples compressed (lossless or lossy) or
uncompressed in 16 bits. refkv reads the file's header and tables, about 90 KB however large the
file, and writes an archive whose chunks are the file's own compressed stripes or sample rows.

## What you get

One OME-Zarr 0.5 image at the archive root, with one array, `0`.

- **The raw image:** the sensor's samples, `uint16`, `[y, x]`, margins included, as LibRaw's and
  rawpy's `raw_image` gives them. It is the mosaic, not a demosaiced RGB image: up close it is a
  fine checkerboard of the color filter array, Bayer (GFX) or X-Trans (X-series).
- **Chunks:** compressed data is one chunk per **stripe**, 768 columns over the full height, each
  the file's own bytes, decoded by refkv's codec `refkv.fuji_compressed`
  ([conventions §13](../../../spec/conventions.md#13-fujifilm-compressed-stripes)): 6 to 13 MB of
  samples a chunk. Uncompressed data is cut into runs of rows of at most 1 MiB.
- **The pattern** the codec places colors by: an X-Trans camera's from the file; a GFX's Bayer
  pattern, which no file states, is RGGB, the one LibRaw and rawpy give every GFX model.
- **omero:** one channel, its window from 0 to the largest sample of the file's bit depth.
- **Metadata:** the header and its directory, every metadata record (as bytes), and the sensor
  data's TIFF, under `refkv_zarrified.raf`; the whole file, preview included, on `refkv_source`.

## Try it

[example.py](example.py) zarrifies a public X-E3 file of raw.pixls.us (CC0) and plots a crop of
its mosaic with matplotlib. Its dependencies are in its header (PEP 723):

```bash
uv run docs/zarrify/raf/example.py
```

```bash
uv run refkv zarrify https://raw.pixls.us/download/data/Fujifilm/X-E3/DSCF2176.RAF xe3.vzip
```

```
{"format": "raf", "camera": "X-E3", "encoding": "compressed", "sensor": "x-trans", "lossless": true, "bits": 14, "sizes": {"y": 4038, "x": 6048}, "chunkShape": [4038, 768], "chunks": 8, ...}
```

## Viewing

Neuroglancer shows the array once its fork decodes `refkv.fuji_compressed`, which is a separate
change to [the fork](https://github.com/d-v-b/neuroglancer/tree/vzip). zarr-python reads it
through refkv's codec, which `refkv[zarr]` registers.

## Supported and not supported

Supported:

- Compressed sensor data, lossless and lossy, Bayer and X-Trans, of 12, 14 and 16 bits.
- Uncompressed 16-bit sensor data, in either byte order.

Rejected, each with a message that says why:

- 12- and 14-bit **packed** samples: the X-Pro1, X-E1, X-M1, X-A series, X-T100, X-T200, XF10,
  X10, X20, X30, X-S1, X100 and the FinePix models.
- Files with a **second frame** (EXR and SuperCCD SR cameras), and older FinePix files whose
  sensor data is not a TIFF, including SuperCCD's rotated layouts.

## Performance

Zarrifying reads the header, then the metadata records and the start of the sensor data in one
more round: two requests, about 90 KB. The archive is about 40 KB. A reader decodes a whole stripe
to read any sample of it, since a stripe is coded top to bottom.

## How it's verified

`js/test/raf/verify.py` zarrifies the synthetic inputs of [fixtures/raf/](../../../fixtures/raf/)
(their compressed stripes cut from real public-domain files) with the browser code, reads them
back through zarr-python, and compares every sample with rawpy's `raw_image`; it takes a
directory of real files too. `python/tests/ir/test_raf.py` checks the Python host, and
`rust/refkv-ir/tests/raf.rs` each rejection. The codec was written from its specification alone
and matched rawpy on every stripe of four real files, lossless and lossy, Bayer and X-Trans, of 14
and 16 bits.

- Profile: [spec/zarrify/raf.md](../../../spec/zarrify/raf.md#part-2-the-profile)
- Core: [rust/refkv-ir/src/raf.rs](../../../rust/refkv-ir/src/raf.rs)
