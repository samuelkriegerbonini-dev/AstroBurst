import warnings

import astropy
import astropy.units as u
import numpy as np
from astropy.coordinates import FK5, SkyCoord
from astropy.io import fits
from astropy.wcs import WCS

warnings.simplefilter("ignore")


def header(cards):
    h = fits.Header()
    for key, value in cards:
        h[key] = value
    return h


def sky(cards, pixels):
    w = WCS(header(cards))
    return [w.all_pix2world([[x, y]], 0)[0].tolist() for x, y in pixels]


def show(label, cards, pixels):
    for (x, y), (lon, lat) in zip(pixels, sky(cards, pixels)):
        print(f"{label} pixel=({x}, {y}) world=({lon!r}, {lat!r})")


BASE = [("NAXIS", 2), ("NAXIS1", 480), ("NAXIS2", 480), ("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"),
        ("CRPIX1", 240.5), ("CRPIX2", 240.5)]
UNIT_PIXELS = [(0.0, 0.0), (479.0, 100.0)]
DEG_CDELT = 0.25 / 3600.0
DEG_CRVAL = (196848.0 / 3600.0, -108000.0 / 3600.0)
UNITS = [("deg", 1.0), ("arcmin", 60.0), ("arcsec", 3600.0), ("mas", 3.6e6), ("rad", np.pi / 180.0)]

print("astropy", astropy.__version__)
print("# WCS-05 celestial CUNIT")
for unit, per_degree in UNITS:
    cards = BASE + [("CUNIT1", unit), ("CUNIT2", unit),
                    ("CDELT1", -DEG_CDELT * per_degree), ("CDELT2", DEG_CDELT * per_degree),
                    ("CRVAL1", DEG_CRVAL[0] * per_degree), ("CRVAL2", DEG_CRVAL[1] * per_degree)]
    for key, value in cards[7:]:
        print(f"cunit={unit} card {key} = {value!r}")
    show(f"cunit={unit}", cards, UNIT_PIXELS)
mixed = BASE + [("CUNIT1", "arcsec"), ("CUNIT2", "deg"), ("CD1_1", -0.25), ("CD1_2", 0.05), ("CD2_1", 1.0e-5), ("CD2_2", DEG_CDELT),
                ("CRVAL1", 196848.0), ("CRVAL2", -30.0)]
show("cunit=arcsec,deg with CD", mixed, UNIT_PIXELS)

print("# WCS-09 FK5 J2000 header read as ICRS positions")
fk5_cards = [("NAXIS", 2), ("NAXIS1", 200), ("NAXIS2", 200), ("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"),
             ("CRPIX1", 100.5), ("CRPIX2", 100.5), ("CRVAL1", 150.0), ("CRVAL2", 2.2),
             ("CDELT1", -6.94444461259988e-05), ("CDELT2", 6.94444461259988e-05), ("RADESYS", "FK5"), ("EQUINOX", 2000.0)]
w = WCS(header(fk5_cards))
for x, y in [(99.5, 99.5), (10.0, 180.0)]:
    lon, lat = w.all_pix2world([[x, y]], 0)[0].tolist()
    icrs = SkyCoord(lon * u.deg, lat * u.deg, frame=FK5(equinox="J2000")).icrs
    print(f"fk5 pixel=({x}, {y}) fk5=({lon!r}, {lat!r}) icrs=({icrs.ra.deg!r}, {icrs.dec.deg!r})")
    back = SkyCoord(icrs.ra, icrs.dec, frame="icrs").transform_to(FK5(equinox="J2000"))
    px = w.all_world2pix([[back.ra.deg, back.dec.deg]], 0)[0].tolist()
    print(f"fk5 icrs=({icrs.ra.deg!r}, {icrs.dec.deg!r}) pixel=({px[0]!r}, {px[1]!r})")

print("# WCS-15 poles and fiducial points on non TAN projections")
CAR = [("NAXIS", 2), ("NAXIS1", 100), ("NAXIS2", 100), ("CTYPE1", "RA---CAR"), ("CTYPE2", "DEC--CAR"),
       ("CRPIX1", 50.0), ("CRPIX2", 50.0), ("CRVAL1", 30.0), ("CRVAL2", 40.0), ("CDELT1", -0.01), ("CDELT2", 0.01)]
ZEA = [("NAXIS", 2), ("NAXIS1", 100), ("NAXIS2", 100), ("CTYPE1", "RA---ZEA"), ("CTYPE2", "DEC--ZEA"),
       ("CRPIX1", 50.0), ("CRPIX2", 50.0), ("CRVAL1", 30.0), ("CRVAL2", 40.0), ("CDELT1", -0.01), ("CDELT2", 0.01)]
POLE_PIXELS = [(0.0, 0.0), (80.0, 15.0)]
show("car default", CAR, POLE_PIXELS)
show("car LONPOLE=0 (the default stated)", CAR + [("LONPOLE", 0.0)], POLE_PIXELS)
show("car LONPOLE=30", CAR + [("LONPOLE", 30.0)], POLE_PIXELS)
show("car LONPOLE=330", CAR + [("LONPOLE", 330.0)], POLE_PIXELS)
south = [(k, -20.0 if k == "CRVAL2" else v) for k, v in CAR]
show("car south default", south, POLE_PIXELS)
show("car south LONPOLE=180 (the default stated)", south + [("LONPOLE", 180.0)], POLE_PIXELS)
show("zea default", ZEA, POLE_PIXELS)
show("zea PV1_1=0 PV1_2=90 (the default stated)", ZEA + [("PV1_1", 0.0), ("PV1_2", 90.0)], POLE_PIXELS)
show("zea PV1_1=0 PV1_2=45", ZEA + [("PV1_1", 0.0), ("PV1_2", 45.0)], POLE_PIXELS)
show("zea LONPOLE=150", ZEA + [("LONPOLE", 150.0)], POLE_PIXELS)

print("# CUBE-07 missing CRPIX on a spectral axis")
spec = WCS(header([("NAXIS", 1), ("NAXIS1", 10), ("CTYPE1", "WAVE"), ("CUNIT1", "um"), ("CRVAL1", 2.0), ("CDELT1", 0.001)]))
print("crpix", spec.wcs.crpix.tolist())
values = (spec.all_pix2world(np.arange(10.0), 0)[0] * u.m).to(u.um).value
print("wave_um", [repr(float(v)) for v in values])

print("# WCS-15 zenithal projections with a non default LONPOLE")
TAN = [(k, v.replace("ZEA", "TAN") if isinstance(v, str) else v) for k, v in ZEA]
show("zea LONPOLE=210", ZEA + [("LONPOLE", 210.0)], POLE_PIXELS)
show("tan LONPOLE=170", TAN + [("LONPOLE", 170.0)], POLE_PIXELS)
show("tan LONPOLE=190", TAN + [("LONPOLE", 190.0)], POLE_PIXELS)
SIN = [(k, v.replace("ZEA", "SIN") if isinstance(v, str) else v) for k, v in ZEA]
show("sin LONPOLE=120", SIN + [("LONPOLE", 120.0)], POLE_PIXELS)

print("# WCS-01 and WCS-09 Pan-STARRS cutout with old style PC cards and RADESYS FK5")
PS1 = [("NAXIS", 2), ("NAXIS1", 480), ("NAXIS2", 480), ("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"),
       ("CRVAL1", 54.6835441588982), ("CRVAL2", -29.999999999999), ("CRPIX1", -19507.5), ("CRPIX2", -27002.5),
       ("CDELT1", 6.94444461259988e-05), ("CDELT2", 6.94444461259988e-05),
       ("PC001001", -1.0), ("PC001002", 0.0), ("PC002001", 0.0), ("PC002002", 1.0), ("RADESYS", "FK5")]
STAR = (53.13872873150106, -28.085398991615556)
ps1 = WCS(header(PS1))
raw = ps1.all_world2pix([[STAR[0], STAR[1]]], 0)[0].tolist()
print(f"ps1 star taken as header frame numbers pixel=({raw[0]!r}, {raw[1]!r})")
star_fk5 = SkyCoord(STAR[0] * u.deg, STAR[1] * u.deg, frame="icrs").transform_to(FK5(equinox="J2000"))
aware = ps1.all_world2pix([[star_fk5.ra.deg, star_fk5.dec.deg]], 0)[0].tolist()
print(f"ps1 star as ICRS pixel=({aware[0]!r}, {aware[1]!r})")
for x, y in [(239.5, 239.5), (0.0, 0.0), (479.0, 0.0)]:
    lon, lat = ps1.all_pix2world([[x, y]], 0)[0].tolist()
    icrs = SkyCoord(lon * u.deg, lat * u.deg, frame=FK5(equinox="J2000")).icrs
    print(f"ps1 pixel=({x}, {y}) fk5=({lon!r}, {lat!r}) icrs=({float(icrs.ra.deg)!r}, {float(icrs.dec.deg)!r})")
rot = [(k, v) for k, v in PS1 if not k.startswith("PC00") and not k.startswith("CDELT")]
rot = [(k, (481.0 - v) if k in ("CRPIX1", "CRPIX2") else v) for k, v in rot]
rot += [("CD1_1", 6.94444461259988e-05), ("CD1_2", 0.0), ("CD2_1", 0.0), ("CD2_2", -6.94444461259988e-05)]
rotated = WCS(header(rot)).all_world2pix([[star_fk5.ra.deg, star_fk5.dec.deg]], 0)[0].tolist()
print(f"ps1 rotated 180 star as ICRS pixel=({rotated[0]!r}, {rotated[1]!r}) expected=({479.0 - aware[0]!r}, {479.0 - aware[1]!r})")
