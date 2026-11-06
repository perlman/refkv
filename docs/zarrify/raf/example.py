# /// script
# requires-python = ">=3.12"
# dependencies = ["refkv[zarr]>=0.2.0", "matplotlib"]
# ///
"""A Fujifilm X-E3 raw photo (raw.pixls.us, CC0), zarrified without copying it: refkv reads the
file's structure and presents its sensor data as Zarr, each chunk one of the file's compressed
stripes, read and decoded only when plotted.

Run with `uv run docs/zarrify/raf/example.py`; the figure is saved as raf.png.
"""

import matplotlib.pyplot as plt
import numpy as np
import zarr

import refkv.zarr
from refkv.zarrify import zarrify

URL = "https://raw.pixls.us/download/data/Fujifilm/X-E3/DSCF2176.RAF"

root = zarr.open_group(refkv.zarr.store(zarrify(URL)), mode="r")
mosaic = root.get_array("0")  # [y, x], the X-Trans mosaic
cy, cx = mosaic.shape[0] // 2, mosaic.shape[1] // 2
whole = np.asarray(mosaic[::8, : 768 * 2 : 4])  # the first two stripes, subsampled
crop = np.asarray(mosaic[cy:cy + 48, cx:cx + 48])  # 48 x 48 samples: the color filter's pattern
fig, axes = plt.subplots(1, 2, figsize=(10, 5))
axes[0].imshow(whole, cmap="gray")
axes[0].set_title("two stripes, subsampled")
axes[1].imshow(crop, cmap="gray", interpolation="nearest")
axes[1].set_title("48 x 48 samples of the mosaic")
for ax in axes:
    ax.axis("off")

fig.tight_layout()
fig.savefig("raf.png", dpi=100)
plt.show()
