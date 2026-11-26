import csv
import json
import os

import numpy as np
from astropy.io import fits

VECTOR_FLOAT_COLUMNS = [
    "WAVELENGTH", "FLUX", "FLUX_ERROR", "FLUX_VAR_POISSON", "FLUX_VAR_RNOISE", "FLUX_VAR_FLAT",
    "SURF_BRIGHT", "SB_ERROR", "SB_VAR_POISSON", "SB_VAR_RNOISE", "SB_VAR_FLAT",
    "BACKGROUND", "BKGD_ERROR", "BKGD_VAR_POISSON", "BKGD_VAR_RNOISE", "BKGD_VAR_FLAT", "NPIXELS",
]
TIME_COLUMNS = ["MJD-BEG", "MJD-AVG", "MJD-END", "TDB-BEG", "TDB-MID", "TDB-END"]


def manifest_index(data_root):
    idx = {}
    with open(os.path.join(data_root, "MANIFEST.tsv"), "r", encoding="utf-8", newline="") as fh:
        for r in csv.DictReader(fh, delimiter="\t"):
            idx[r["file"]] = {"set": r["set"], "bytes": int(r["actual_bytes"]), "sha256": r["sha256"]}
    return idx


def file_ref(data_root, rel, midx):
    base = os.path.basename(rel)
    m = midx[base]
    return {"file": base, "set": m["set"], "bytes": m["bytes"], "sha256": m["sha256"]}


def load_x1dints(path):
    out = {"path": path, "specs": [], "int_times": None}
    with fits.open(path, memmap=False) as hdul:
        ph = hdul[0].header
        out["primary"] = {k: ph.get(k) for k in (
            "DATAMODL", "CAL_VER", "EXP_TYPE", "INSTRUME", "DETECTOR", "FILTER", "PUPIL", "GRATING", "SUBARRAY",
            "NINTS", "INTSTART", "INTEND", "EXSEGNUM", "EXSEGTOT", "TARGNAME", "TARGPROP", "EFFINTTM", "TGROUP", "NGROUPS",
            "EXPSTART", "EXPMID", "EXPEND", "BARTDELT", "S_PHOTOM", "DATE", "FILENAME")}
        for i, hdu in enumerate(hdul):
            if hdu.name == "INT_TIMES":
                d = hdu.data
                out["int_times"] = {n: np.array(d[n]) for n in d.names}
                out["int_times_index"] = i
            if hdu.name != "EXTRACT1D":
                continue
            h = hdu.header
            d = hdu.data
            names = list(d.names)
            spec = {
                "hdu_index": i,
                "extver": h.get("EXTVER"),
                "detector_card": h.get("DETECTOR"),
                "detector": h.get("DETECTOR") if h.get("DETECTOR") is not None else ph.get("DETECTOR"),
                "sporder_card": h.get("SPORDER"),
                "sporder": h.get("SPORDER"),
                "naxis1": h["NAXIS1"],
                "naxis2": h["NAXIS2"],
                "names": names,
                "tunit": {h["TTYPE%d" % k]: h.get("TUNIT%d" % k) for k in range(1, h["TFIELDS"] + 1)},
                "tform": {h["TTYPE%d" % k]: h.get("TFORM%d" % k) for k in range(1, h["TFIELDS"] + 1)},
            }
            wave_form = spec["tform"]["WAVELENGTH"]
            repeat = int(wave_form[:-1]) if len(wave_form) > 1 else 1
            if "INT_NUM" in names and "N_ALONGDISP" in names and repeat > 1:
                spec["layout"] = "row_per_integration"
                spec["nelem"] = repeat
                spec["INT_NUM"] = np.array(d["INT_NUM"]).astype(np.int64)
                spec["N_ALONGDISP"] = np.array(d["N_ALONGDISP"]).astype(np.int64)
                spec["SEGMENT"] = np.array(d["SEGMENT"]).astype(np.int64)
                for c in TIME_COLUMNS:
                    spec[c] = np.array(d[c], dtype=np.float64)
                for c in VECTOR_FLOAT_COLUMNS:
                    spec[c] = np.array(d[c], dtype=np.float64).reshape(len(d), repeat)
                spec["DQ"] = np.array(d["DQ"]).astype(np.int64).reshape(len(d), repeat)
            else:
                spec["layout"] = "extension_per_integration"
                spec["nelem"] = len(d)
                spec["INT_NUM"] = np.array([h.get("INT_NUM", -1)], dtype=np.int64)
                spec["N_ALONGDISP"] = np.array([len(d)], dtype=np.int64)
                spec["SEGMENT"] = np.array([ph.get("EXSEGNUM", 1)], dtype=np.int64)
                for c in TIME_COLUMNS:
                    v = h.get(c)
                    spec[c] = np.array([np.nan if v is None else float(v)], dtype=np.float64)
                for c in VECTOR_FLOAT_COLUMNS:
                    spec[c] = np.array(d[c], dtype=np.float64).reshape(1, len(d))
                spec["DQ"] = np.array(d["DQ"]).astype(np.int64).reshape(1, len(d))
            out["specs"].append(spec)
    return out


def series_key(spec):
    return (spec["detector"], spec["sporder"])


def white_light_rows(spec, min_wave, max_wave):
    mid = spec["MJD-AVG"].copy()
    _, keep = np.unique(mid, return_index=True)
    dup = np.ones(mid.shape, dtype=bool)
    dup[keep] = False
    mid[dup] = np.nan
    good = ~np.isnan(mid)
    wave = spec["WAVELENGTH"]
    with np.errstate(invalid="ignore"):
        mask = (wave >= min_wave) & (wave <= max_wave) & good[:, None]
    flux = spec["FLUX"].copy()
    flux[~mask] = np.nan
    sums = np.nansum(flux, axis=1)
    return {"good": good, "mid_mjd": mid, "tdb_mid": spec["TDB-MID"].copy(), "flux_sum": sums, "n_summed": np.sum(mask & ~np.isnan(spec["FLUX"]), axis=1)}


def read_ecsv_raw(path):
    with open(path, "r", encoding="utf-8", newline="") as fh:
        text = fh.read()
    lines = text.split("\n")
    header_lines = [ln for ln in lines if ln.startswith("#")]
    data_lines = [ln for ln in lines if ln and not ln.startswith("#")]
    return {"text": text, "header_lines": header_lines, "data_lines": data_lines, "crlf": "\r\n" in text}


def f64(x):
    return None if x is None else repr(float(x))


def f64_list(a):
    return [repr(float(v)) for v in np.asarray(a, dtype=np.float64).ravel()]


def write_json(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(obj, fh, indent=1)
        fh.write("\n")


def clean(v):
    if isinstance(v, dict):
        return {k: clean(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [clean(x) for x in v]
    if isinstance(v, np.ndarray):
        return [clean(x) for x in v.tolist()]
    if isinstance(v, (np.floating,)):
        v = float(v)
    if isinstance(v, (np.integer,)):
        return int(v)
    if isinstance(v, (np.bool_,)):
        return bool(v)
    if isinstance(v, float):
        if v != v or v in (float("inf"), float("-inf")):
            return None
        return v
    return v


def golden_meta(script, args_template, inputs):
    import platform
    import astropy
    return {
        "script": script,
        "command": "python -I scripts/oracle/%s %s" % (script, args_template),
        "environment": {"python": platform.python_version(), "numpy": np.__version__, "astropy": astropy.__version__},
        "generated": "2026-11-09",
        "inputs": inputs,
        "number_format": "float64 written with Python repr (shortest round-trip decimal); null stands for NaN or not applicable; integers are exact",
        "data_root_note": "inputs live under docs/notCommit/science-focus-data (not committed); sha256 and bytes are the MANIFEST.tsv values, re-checked on disk by tso_golden_index.py",
    }


def write_golden(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(clean(obj), fh, indent=1, allow_nan=False)
        fh.write("\n")
