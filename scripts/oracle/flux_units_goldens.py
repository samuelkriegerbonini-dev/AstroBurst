import json
import platform
import sys
from pathlib import Path

import astropy
import astropy.constants
import astropy.units as u
import numpy

DEFAULT_OUTPUT = Path(__file__).resolve().parents[2] / "src-tauri/tests/fixtures/flux_units/astropy_flux_units.json"
WAVELENGTHS_UM = [0.1216, 0.5, 0.6563, 1.0, 2.2, 4.3, 10.0, 25.0, 1000.0]
FNU_JY = [1e-9, 3.631e-6, 1e-3, 0.0123456, 1.0, 3630.7805477010028, 1.5e6]
FLAM_CGS = u.erg / u.s / u.cm**2 / u.AA
FLAM_SI = u.W / u.m**2 / u.um
NU = "flux_density_nu"
LAMBDA = "flux_density_lambda"
MICRO_SIGN = chr(0xB5)
GREEK_MU = chr(0x3BC)
ASTROPY_SPELLINGS = [
    ("Jy", NU),
    ("mJy", NU),
    ("uJy", NU),
    ("nJy", NU),
    ("MJy", NU),
    ("kJy", NU),
    ("erg / (Hz s cm2)", NU),
    ("erg s-1 cm-2 Hz-1", NU),
    ("W / (Hz m2)", NU),
    ("W m-2 Hz-1", NU),
    ("erg / (Angstrom s cm2)", LAMBDA),
    ("erg / (s cm2 Angstrom)", LAMBDA),
    ("erg s-1 cm-2 Angstrom-1", LAMBDA),
    ("Angstrom-1 cm-2 erg s-1", LAMBDA),
    ("erg / (cm2 s AA)", LAMBDA),
    ("1e-17 erg / (Angstrom s cm2)", LAMBDA),
    ("erg / (s cm2 um)", LAMBDA),
    ("erg / (s cm2 nm)", LAMBDA),
    ("W / (m2 um)", LAMBDA),
    ("W m-2 um-1", LAMBDA),
    ("W / (m2 nm)", LAMBDA),
    ("W / (m2 Angstrom)", LAMBDA),
]
DEFINED_SPELLINGS = [
    (MICRO_SIGN + "Jy", NU, 1e-6),
    (GREEK_MU + "Jy", NU, 1e-6),
    ("JY", NU, 1.0),
    ("jy", NU, 1.0),
    ("Jansky", NU, 1.0),
    ("10 Jy", NU, 10.0),
    ("erg/s/cm2/Hz", NU, 1e23),
    ("W/m2/Hz", NU, 1e26),
    ("erg/s/cm**2/Angstrom", LAMBDA, 1.0),
    ("erg/cm2/s/A", LAMBDA, 1.0),
    ("erg/s/cm2/A", LAMBDA, 1.0),
    ("erg/s/cm^2/A", LAMBDA, 1.0),
    ("ergs/cm2/s/A", LAMBDA, 1.0),
    ("erg cm-2 s-1 A-1", LAMBDA, 1.0),
    ("erg.s**(-1).cm**(-2).angstrom**(-1)", LAMBDA, 1.0),
    ("erg/s/cm2/AA", LAMBDA, 1.0),
    ("erg/s/cm2/Ang", LAMBDA, 1.0),
    ("erg cm**-2 s**-1 Angstrom**-1", LAMBDA, 1.0),
    ("1E-17 erg/cm^2/s/Ang", LAMBDA, 1e-17),
    ("10**(-16)erg.cm**(-2).s**(-1).angstrom**(-1)", LAMBDA, 1e-16),
    ("10^-17 erg/s/cm2/A", LAMBDA, 1e-17),
    ("10**-20 erg/s/cm**2/Angstrom", LAMBDA, 1e-20),
    ("erg/s/cm2/um", LAMBDA, 1e-4),
    ("W/m2/um", LAMBDA, 0.1),
    ("W/m^2/micron", LAMBDA, 0.1),
    ("W m-2 " + MICRO_SIGN + "m-1", LAMBDA, 0.1),
    ("MJy/sr", "surface_brightness", None),
    ("MJy / sr", "surface_brightness", None),
    ("MJy.sr-1", "surface_brightness", None),
    ("Jy/arcsec2", "surface_brightness", None),
    ("erg/s/cm2/A/arcsec2", "surface_brightness", None),
    ("Jy/pixel", "surface_brightness", None),
    ("Jy/beam", "surface_brightness", None),
    ("MJy/sr x pix", "surface_brightness", None),
    ("DN/s", "rate", None),
    ("DN / s", "rate", None),
    ("dn/s", "rate", None),
    ("DN s-1", "rate", None),
    ("count/s", "rate", None),
    ("counts/s", "rate", None),
    ("ct/s", "rate", None),
    ("electron/s", "rate", None),
    ("electrons/s", "rate", None),
    ("ADU/s", "rate", None),
    ("", "unknown", None),
    ("native", "unknown", None),
    ("DN", "unknown", None),
    ("counts", "unknown", None),
    ("Jy x um", "unknown", None),
    ("mag", "unknown", None),
    ("foo", "unknown", None),
    ("MJY", "unknown", None),
    ("mjy", "unknown", None),
    ("erg/s/cm2", "unknown", None),
    ("erg / s cm2", "unknown", None),
    ("Jy/", "unknown", None),
    ("(Jy", "unknown", None),
    ("Jy)", "unknown", None),
    ("erg//s", "unknown", None),
    ("erg/(s/(cm2))", "unknown", None),
    ("1e-17", "unknown", None),
    ("e-/s", "unknown", None),
]


def conversions():
    rows = []
    for lam in WAVELENGTHS_UM:
        eq = u.spectral_density(lam * u.um)
        for fnu in FNU_JY:
            q = fnu * u.Jy
            flam_cgs = float(q.to(FLAM_CGS, equivalencies=eq).value)
            rows.append(
                {
                    "lambda_um": lam,
                    "fnu_jy": fnu,
                    "flambda_cgs": flam_cgs,
                    "flambda_si_um": float(q.to(FLAM_SI, equivalencies=eq).value),
                    "ab_mag": float(q.to(u.ABmag).value),
                    "fnu_jy_back": float((flam_cgs * FLAM_CGS).to(u.Jy, equivalencies=eq).value),
                }
            )
    return rows


def spelling(text, kind, factor):
    row = {"text": text, "kind": kind}
    if kind == NU:
        row["jy_per_unit"] = factor
    if kind == LAMBDA:
        row["cgs_per_unit"] = factor
    return row


def astropy_spellings():
    rows = []
    for text, kind in ASTROPY_SPELLINGS:
        unit = u.Unit(text)
        factor = float(unit.to(u.Jy)) if kind == NU else float(unit.to(FLAM_CGS))
        rows.append(spelling(text, kind, factor))
    return rows


def main():
    output = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUTPUT
    report = {
        "kind": "astropy_flux_unit_goldens",
        "versions": {"python": platform.python_version(), "numpy": numpy.__version__, "astropy": astropy.__version__},
        "method": "astropy.units: spectral_density(lambda) for F_nu and F_lambda, ABmag for AB magnitudes, Unit.to for the factors of astropy_spellings; defined_spellings are written by hand for spellings astropy does not read as flux densities (A as Angstrom, IRAF and ESO forms) and for the refusals",
        "tolerance": {"relative": 1e-12, "ab_mag_absolute": 1e-12},
        "constants": {
            "c_m_per_s": float(astropy.constants.c.value),
            "ab_zero_point_mag": float((1.0 * u.Jy).to(u.ABmag).value),
            "ab_zero_flux_jy": float((0.0 * u.ABmag).to(u.Jy).value),
        },
        "conversions": conversions(),
        "astropy_spellings": astropy_spellings(),
        "defined_spellings": [spelling(text, kind, factor) for text, kind, factor in DEFINED_SPELLINGS],
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=1, allow_nan=False) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"fixture": str(output), "conversions": len(report["conversions"]), "astropy_spellings": len(report["astropy_spellings"]), "defined_spellings": len(report["defined_spellings"])}))


if __name__ == "__main__":
    main()
