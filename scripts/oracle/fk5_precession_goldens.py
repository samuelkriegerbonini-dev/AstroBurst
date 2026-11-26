import json
import sys

import astropy
import astropy.units as u
from astropy.coordinates import FK5, ICRS, SkyCoord
from astropy.time import Time


EQUINOXES = ["J1975.0", "J1950.0", "B1950.0", "J2015.5", "J2000.0"]
POSITIONS = [(150.0, 30.0), (10.0, -60.0), (359.9, 85.0), (200.5, 0.1)]


def case(equinox, lon, lat):
    time = Time(equinox)
    source = SkyCoord(lon * u.deg, lat * u.deg, frame=FK5(equinox=time))
    j2000 = source.transform_to(FK5(equinox=Time("J2000.0")))
    icrs = source.transform_to(ICRS())
    as_read = SkyCoord(lon * u.deg, lat * u.deg, frame=ICRS())
    return {
        "equinox": equinox,
        "julian_year": float(time.jyear),
        "lon_deg": lon,
        "lat_deg": lat,
        "fk5_j2000_ra_deg": float(j2000.ra.deg),
        "fk5_j2000_dec_deg": float(j2000.dec.deg),
        "icrs_ra_deg": float(icrs.ra.deg),
        "icrs_dec_deg": float(icrs.dec.deg),
        "unprecessed_error_arcmin": float(as_read.separation(icrs).arcmin),
        "fk5_j2000_to_icrs_mas": float(SkyCoord(j2000.ra, j2000.dec, frame=ICRS()).separation(icrs).mas),
    }


def main(out_path):
    cases = [case(equinox, lon, lat) for equinox in EQUINOXES for lon, lat in POSITIONS]
    payload = {
        "generator": "scripts/oracle/fk5_precession_goldens.py",
        "astropy": astropy.__version__,
        "cases": cases,
    }
    with open(out_path, "w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=1)
        handle.write("\n")
    print("astropy", astropy.__version__, "cases", len(cases))
    for c in cases:
        print(c["equinox"], c["lon_deg"], c["lat_deg"], c["fk5_j2000_ra_deg"], c["fk5_j2000_dec_deg"], round(c["unprecessed_error_arcmin"], 4), round(c["fk5_j2000_to_icrs_mas"], 3))


if __name__ == "__main__":
    main(sys.argv[1])
