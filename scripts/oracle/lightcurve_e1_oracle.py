import json
import os
import sys

import astropy
import numpy as np
from astropy.io import fits

E1_FILE = "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits"
PRODUCED_BY = "python -I scripts/oracle/lightcurve_e1_oracle.py <data root>/tso <output dir> (astropy %s, numpy %s)"


def white_light(path):
    with fits.open(path, memmap=False) as hdul:
        table = hdul["EXTRACT1D"].data
        rows = len(table)
        flux = np.asarray(table["FLUX"], "f8").reshape(rows, -1)
        error = np.asarray(table["FLUX_ERROR"], "f8").reshape(rows, -1)
        return dict(
            elements=flux.shape[1],
            int_num=[int(v) for v in table["INT_NUM"]],
            bmjd_tdb=[float(v) for v in np.asarray(table["TDB-MID"], "f8")],
            mjd_utc=[float(v) for v in np.asarray(table["MJD-AVG"], "f8")],
            flux=[float(v) for v in np.nansum(flux, axis=1)],
            flux_err=[float(v) for v in np.sqrt(np.nansum(np.where(np.isfinite(flux), error * error, np.nan), axis=1))],
        )


def main():
    if len(sys.argv) < 3:
        print("usage: lightcurve_e1_oracle.py <data root>/tso <output dir>")
        return
    curve = white_light(os.path.join(sys.argv[1], "E1", E1_FILE))
    doc = {
        "source": "%s, EXTRACT1D, level 3" % E1_FILE,
        "produced_by": PRODUCED_BY % (astropy.__version__, np.__version__),
        "definition": "flux = numpy nansum of FLUX over all %d elements per integration; flux_err = sqrt(sum of FLUX_ERROR^2 over the elements with finite FLUX); bmjd_tdb = TDB-MID; mjd_utc = MJD-AVG" % curve.pop("elements"),
        "unit": "Jy",
    }
    doc.update(curve)
    path = os.path.join(sys.argv[2], "e1_white_light.json")
    with open(path, "w", newline="\n") as fh:
        json.dump(doc, fh, separators=(",", ":"))
    print("wrote", path, os.path.getsize(path), "bytes")


main()
