import json
import platform
import struct
import sys
import warnings
from pathlib import Path

import astropy
import numpy as np
from astropy.io import fits
from astropy.io.fits.card import _format_float
from astropy.nddata import CCDData, StdDevUncertainty, VarianceUncertainty, InverseVariance

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "src-tauri/tests/fixtures/fitsio"
COMPRESSED = ROOT / "src-tauri/tests/fixtures/fits_compressed"
BLOCK = 2880
THEAP_GAP = 16
ABSENT = None
MERGE_KEYS = ["BUNIT", "CRVAL1", "CTYPE1", "DATAMAX", "NAXIS3", "SIMPLE", "EXTEND", "TELESCOP", "INHERIT", "EXTNAME", "NAXIS"]
FLOAT_VALUES = [
    0.1 + 0.2,
    1e-17,
    100.0,
    1.5e-5,
    1e16,
    -2.5,
    0.0,
    180.0 - 3e-9,
    3.0518509475997192e-05,
    6.02214076e23,
    5e-324,
    1.7976931348623157e308,
    -1.2345678901234567e-300,
    123456.78901234567,
    0.00012345678901234567,
    2.0 ** 53 + 2.0,
    1.0 / 3.0,
    26.49,
    -0.0,
]


def card(text):
    return text.ljust(80).encode("ascii")


def pad(data, fill=b" "):
    rest = len(data) % BLOCK
    return data + (fill * (BLOCK - rest) if rest else b"")


def header_view(path, index):
    header = fits.getheader(path, index)
    return {key: (header[key] if key in header else ABSENT) for key in MERGE_KEYS}


def primary_with_data(name, inherit):
    primary = fits.PrimaryHDU(np.arange(48, dtype="f4").reshape(3, 4, 4))
    primary.header["BUNIT"] = "MJy/sr"
    primary.header["CTYPE1"] = "RA---TAN"
    primary.header["CRVAL1"] = 10.0
    primary.header["DATAMAX"] = 5.0
    primary.header["TELESCOP"] = "JWST"
    stamp = fits.ImageHDU(np.ones((2, 2), dtype="f4"), name="STAMP")
    if inherit is not None:
        stamp.header["INHERIT"] = inherit
    path = OUT / name
    fits.HDUList([primary, stamp]).writeto(path, overwrite=True)
    return {"file": name, "astropy_header_1": header_view(path, 1)}


def primary_without_data(name, inherit):
    primary = fits.PrimaryHDU()
    primary.header["TELESCOP"] = "JWST"
    primary.header["FILTER"] = "F200W"
    primary.header["BUNIT"] = "DN"
    primary.header["DATAMAX"] = 7.0
    primary.header["CRVAL1"] = 33.0
    sci = fits.ImageHDU(np.ones((2, 2), dtype="f4"), name="SCI")
    if inherit is not None:
        sci.header["INHERIT"] = inherit
    path = OUT / name
    fits.HDUList([primary, sci]).writeto(path, overwrite=True)
    return {"file": name, "astropy_header_1": header_view(path, 1)}


def scaled_float_columns():
    cases = []
    for code, dtype in [("E", "f4"), ("D", "f8")]:
        for tscal, tzero in [(1e-17, 0.0), (0.5, 5.0)]:
            column = fits.Column(name="FLUX", format=code, array=np.array([1.0, 2.0, 3.0], dtype=dtype))
            table = fits.BinTableHDU.from_columns([column], name="SPEC")
            table.header["TSCAL1"] = tscal
            table.header["TZERO1"] = tzero
            name = f"tscal_{code.lower()}_{len(cases)}.fits"
            fits.HDUList([fits.PrimaryHDU(), table]).writeto(OUT / name, overwrite=True)
            with fits.open(OUT / name) as hdus:
                values = [float(v) for v in hdus[1].data["FLUX"]]
            cases.append({"file": name, "tform": code, "tscal": tscal, "tzero": tzero, "astropy_values": values})
    return cases


def duplicate_keys(name):
    header = b"".join(
        [
            card("SIMPLE  =                    T"),
            card("BITPIX  =                  -32"),
            card("NAXIS   =                    2"),
            card("NAXIS1  =                    2"),
            card("NAXIS2  =                    2"),
            card("EXPTIME =                100.0"),
            card("BUNIT   = 'MJy/sr  '"),
            card("EXPTIME =                200.0"),
            card("BUNIT   = 'Jy      '"),
            card("END"),
        ]
    )
    data = pad(np.zeros(4, dtype=">f4").tobytes(), b"\0")
    (OUT / name).write_bytes(pad(header) + data)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        with fits.open(OUT / name) as hdus:
            header = hdus[0].header
            return {
                "file": name,
                "astropy_exptime": float(header["EXPTIME"]),
                "astropy_bunit": header["BUNIT"],
                "exptime_cards": header.count("EXPTIME"),
            }


def split_hdus(raw):
    hdus = []
    offset = 0
    while offset < len(raw):
        position = offset
        cards = []
        ended = False
        while not ended:
            block = raw[position : position + BLOCK]
            position += BLOCK
            for i in range(36):
                record = block[i * 80 : (i + 1) * 80]
                cards.append(record)
                if record[:8].strip() == b"END":
                    ended = True
                    break
        values = {c[:8].strip().decode(): c[10:].split(b"/")[0].strip() for c in cards if c[8:10] == b"= "}
        naxis = int(values.get("NAXIS", b"0"))
        size = 0
        if naxis:
            size = abs(int(values["BITPIX"])) // 8
            for axis in range(1, naxis + 1):
                size *= int(values[f"NAXIS{axis}"])
            size += int(values.get("PCOUNT", b"0"))
        hdus.append((offset, position, cards, values))
        offset = position + (size + BLOCK - 1) // BLOCK * BLOCK
    return hdus


def heap_gap(name):
    source = COMPRESSED / "rice_i16_default_tile.fits"
    raw = source.read_bytes()
    with fits.open(source) as hdus:
        reference = hdus[1].data.astype("f8")
    primary, table_hdu = split_hdus(raw)
    start, data_start, cards, values = table_hdu
    naxis1, naxis2, pcount = int(values["NAXIS1"]), int(values["NAXIS2"]), int(values["PCOUNT"])
    table = raw[data_start : data_start + naxis1 * naxis2]
    heap = raw[data_start + naxis1 * naxis2 : data_start + naxis1 * naxis2 + pcount]
    rewritten = []
    for record in cards:
        key = record[:8].strip()
        if key == b"PCOUNT":
            rewritten.append(card(f"PCOUNT  = {pcount + THEAP_GAP:20d}"))
        elif key == b"END":
            rewritten.append(card(f"THEAP   = {naxis1 * naxis2 + THEAP_GAP:20d}"))
            rewritten.append(card("END"))
        else:
            rewritten.append(record)
    (OUT / name).write_bytes(raw[:start] + pad(b"".join(rewritten)) + pad(table + b"\0" * THEAP_GAP + heap, b"\0"))
    with fits.open(OUT / name) as hdus:
        decoded = hdus[1].data.astype("f8")
    return {
        "file": name,
        "gap_bytes": THEAP_GAP,
        "theap": naxis1 * naxis2 + THEAP_GAP,
        "astropy_sum": float(np.nansum(decoded)),
        "astropy_identical_to_ungapped": bool(np.array_equal(decoded, reference)),
    }


def float_cards():
    rows = []
    for value in FLOAT_VALUES:
        shortest = repr(float(value)).replace("e", "E")
        written = _format_float(float(value))
        image = str(fits.Card("K", float(value)))
        free_format = f"K       = {shortest}".ljust(80)
        rows.append(
            {
                "bits": struct.pack(">d", value).hex(),
                "python_repr": shortest,
                "astropy_text": written,
                "astropy_card": image,
                "astropy_text_round_trips": float(written) == value and struct.pack(">d", float(written)) == struct.pack(">d", value),
                "astropy_reads_python_repr_bits": struct.pack(">d", float(fits.Card.fromstring(free_format).value)).hex(),
            }
        )
    return rows


def nddata_layers():
    rows = []
    for name, uncertainty in [
        ("ccddata_stddev.fits", StdDevUncertainty),
        ("ccddata_variance.fits", VarianceUncertainty),
        ("ccddata_ivar.fits", InverseVariance),
    ]:
        data = np.ones((2, 2), dtype="f4")
        ccd = CCDData(data, unit="adu", uncertainty=uncertainty(np.full((2, 2), 4.0, dtype="f4")), mask=np.zeros((2, 2), dtype=bool))
        path = OUT / name
        ccd.write(path, overwrite=True)
        with fits.open(path) as hdus:
            layers = [{"extname": hdu.name, "utype": hdu.header.get("UTYPE")} for hdu in hdus[1:]]
        path.unlink()
        rows.append({"uncertainty": uncertainty.__name__, "extensions": layers})
    return rows


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    golden = {
        "generator": "scripts/oracle/fitsio_review_goldens.py",
        "astropy": astropy.__version__,
        "numpy": np.__version__,
        "python": platform.python_version(),
        "merged_headers": [
            primary_with_data("primary_cube_ext_stamp.fits", None),
            primary_with_data("primary_cube_ext_stamp_inherit_f.fits", False),
            primary_without_data("empty_primary_sci.fits", None),
            primary_without_data("empty_primary_sci_inherit_f.fits", False),
        ],
        "scaled_float_columns": scaled_float_columns(),
        "duplicate_keys": duplicate_keys("duplicate_keys.fits"),
        "heap_gap": heap_gap("rice_i16_theap_gap.fits"),
        "float_cards": float_cards(),
        "nddata_layers": nddata_layers(),
    }
    target = OUT / "astropy_fitsio_goldens.json"
    target.write_text(json.dumps(golden, indent=1) + "\n", encoding="ascii")
    print(f"wrote {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
