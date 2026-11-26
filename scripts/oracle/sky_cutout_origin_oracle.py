import numpy as np
import astropy
import astropy.units as u
from astropy.coordinates import SkyCoord
from astropy.nddata import Cutout2D
from astropy.wcs import WCS


SCALE_ARCSEC = 1.0
SHAPE = (64, 64)
TARGET = SkyCoord(150.0 * u.deg, 2.0 * u.deg, frame="icrs")
CASES = [
    (20.13, 30.0, 6, 4),
    (20.71, 30.0, 7, 4),
    (49.5, 49.5, 6, 3),
]


def tan_wcs(cx, cy):
    wcs = WCS(naxis=2)
    wcs.wcs.ctype = ["RA---TAN", "DEC--TAN"]
    wcs.wcs.crval = [150.0, 2.0]
    wcs.wcs.crpix = [cx + 1.0, cy + 1.0]
    wcs.wcs.cdelt = [-SCALE_ARCSEC / 3600.0, SCALE_ARCSEC / 3600.0]
    wcs.wcs.cunit = ["deg", "deg"]
    return wcs


def origin(cx, cy, width_px, height_px):
    data = np.zeros(SHAPE)
    size = (height_px * SCALE_ARCSEC / 60.0 * u.arcmin, width_px * SCALE_ARCSEC / 60.0 * u.arcmin)
    cut = Cutout2D(data, TARGET, size, wcs=tan_wcs(cx, cy), mode="partial", fill_value=np.nan)
    return cut.origin_original, cut.shape


print("astropy", astropy.__version__)
for cx, cy, width_px, height_px in CASES:
    (x0, y0), shape = origin(cx, cy, width_px, height_px)
    print(f"target=({cx}, {cy}) box={width_px}x{height_px} px origin_original=({x0}, {y0}) shape={shape}")
