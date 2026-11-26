import math
import os
import re
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import tso_golden_lib as tsolib

COL_RE = re.compile(r"^# - \{name: ([^,}]+)(?:, unit: ([^,}]+))?, datatype: ([^,}]+)\}$")
META_RE = re.compile(r"^# - \{([A-Za-z_]+): (.*)\}$")
FLUX_RE = re.compile(r"^whitelight_flux(?:_order_(\d+))?(?:_([A-Z0-9]+))?$")


def parse(path):
    raw = tsolib.read_ecsv_raw(path)
    cols, meta = [], {}
    in_meta = False
    for ln in raw["header_lines"]:
        m = COL_RE.match(ln)
        if m and not in_meta:
            cols.append({"name": m.group(1), "unit": m.group(2), "datatype": m.group(3)})
            continue
        if ln.startswith("# meta:"):
            in_meta = True
            continue
        if in_meta:
            m2 = META_RE.match(ln)
            if m2:
                meta[m2.group(1)] = m2.group(2)
    names = raw["data_lines"][0].split(" ")
    rows = [ln.split(" ") for ln in raw["data_lines"][1:]]
    data = {n: np.array([float(r[i]) for r in rows], dtype=np.float64) for i, n in enumerate(names)}
    return raw, cols, meta, names, data, rows


def build(data_root, rel, midx):
    path = os.path.join(data_root, rel)
    raw, cols, meta, names, data, rows = parse(path)
    legacy = "MJD" in names
    units = {c["name"]: c["unit"] for c in cols}
    series = []
    for n in names:
        m = FLUX_RE.match(n)
        if not m:
            continue
        order = int(m.group(1)) if m.group(1) else None
        det = m.group(2)
        if legacy:
            tname, bname = "MJD", None
        else:
            tname = "MJD_UTC" + ("_" + det if det else "")
            bname = "BJD_TDB" + ("_" + det if det else "")
        t = data[tname]
        fl = data[n]
        ok = np.isfinite(t)
        tf = t[ok]
        ff = fl[ok]
        k = len(ff)
        entry = {
            "flux_column": n, "time_column": tname, "bjd_column": bname,
            "detector_suffix": det, "order_suffix": order, "flux_unit": units.get(n), "time_unit": units.get(tname),
            "rows_with_finite_time": int(k),
            "time_first": float(tf[0]), "time_last": float(tf[-1]),
            "time_strictly_increasing": bool(np.all(np.diff(tf) > 0)) if k > 1 else None,
            "cadence_s_median": float(np.median(np.diff(tf)) * 86400.0) if k > 1 else None,
            "flux_first3": [float(v) for v in ff[:3]], "flux_last3": [float(v) for v in ff[-3:]],
            "flux_sum_fsum": math.fsum(ff.tolist()), "flux_min": float(ff.min()), "flux_max": float(ff.max()),
            "flux_negative_rows": int((ff < 0).sum()),
        }
        if bname:
            b = data[bname][ok]
            entry["bjd_first"] = float(b[0])
            entry["bjd_last"] = float(b[-1])
            entry["bjd_minus_time_s_first"] = float((b[0] - tf[0]) * 86400.0)
        series.append(entry)
    g = {
        "golden": "tso/whtlt",
        "produced_by": tsolib.golden_meta("tso_golden_whtlt.py", "<data root> <goldens/tso dir>", [tsolib.file_ref(data_root, rel, midx)]),
        "ecsv": {
            "first_line": raw["header_lines"][0],
            "line_ending": "CRLF" if raw["crlf"] else "LF",
            "n_header_lines": len(raw["header_lines"]),
            "delimiter_key_present": any("delimiter:" in ln for ln in raw["header_lines"]),
            "delimiter_used": "single space",
            "schema_line": [ln for ln in raw["header_lines"] if ln.startswith("# schema:")][0],
            "has_serialized_columns_block": any("__serialized_columns__" in ln for ln in raw["header_lines"]),
            "has_yaml_anchor_or_alias": any("&id" in ln or "*id" in ln for ln in raw["header_lines"]),
            "column_name_line": raw["data_lines"][0],
        },
        "layout": "legacy (MJD, whitelight_flux)" if legacy else "current (MJD_UTC, BJD_TDB, whitelight_flux with optional order and detector suffixes)",
        "columns": cols,
        "meta": {k: (int(v) if k == "number_of_integrations" else (None if v == "null" else v)) for k, v in meta.items()},
        "meta_key_order": list(meta.keys()),
        "n_data_rows": len(rows),
        "non_numeric_tokens": sorted({tok for r in rows for tok in r if not re.match(r"^-?\d", tok)}),
        "series": series,
    }
    if "jw02358" in rel:
        t = data["MJD_UTC_NRS1"]
        b = data["BJD_TDB_NRS1"]
        fl = data["whitelight_flux_NRS1"]
        med = float(np.median(fl))
        low = np.where(fl < 0.9 * med)[0]
        kmin = int(np.argmin(fl))
        outside = np.ones(len(fl), dtype=bool)
        outside[low] = False
        mo = float(np.median(fl[outside]))
        g["transit_scan"] = {
            "median_all": med,
            "rows_below_0p9_median_index0": [int(low[0]), int(low[-1])],
            "rows_below_0p9_median_contiguous": bool(np.all(np.diff(low) == 1)),
            "rows_below_0p9_median_count": int(len(low)),
            "first_low_mjd_utc": float(t[low[0]]), "last_low_mjd_utc": float(t[low[-1]]),
            "first_low_bjd_tdb": float(b[low[0]]), "last_low_bjd_tdb": float(b[low[-1]]),
            "minimum_index0": kmin, "minimum_mjd_utc": float(t[kmin]), "minimum_bjd_tdb": float(b[kmin]), "minimum_flux": float(fl[kmin]),
            "median_outside": mo,
            "depth_at_minimum": 1.0 - float(fl[kmin]) / mo,
            "depth_mean_of_low_rows": 1.0 - float(np.mean(fl[low])) / mo,
            "normalised_flux_index0_175_to_194": [float(v / mo) for v in fl[175:195]],
        }
    if "jw07849" in rel:
        fl = data["whitelight_flux"]
        g["note"] = "Kepler-12: integrations 3 to 71 stay near 0.2 Jy; from integration 72 on the series is erratic between 0.02 and 0.16 Jy. No transit or eclipse can be read from it."
        g["quality_scan"] = {"median_rows_3_to_71": float(np.median(fl[2:71])), "rel_std_rows_3_to_71": float(np.std(fl[2:71], ddof=1) / np.mean(fl[2:71])), "median_rows_72_to_100": float(np.median(fl[71:])), "rows_below_half_of_first_median": [int(k + 1) for k in np.where(fl < 0.5 * np.median(fl[2:71]))[0]]}
    return g


def main():
    data_root = sys.argv[1]
    out_dir = sys.argv[2]
    midx = tsolib.manifest_index(data_root)
    wd = os.path.join(data_root, "tso", "whtlt")
    for name in sorted(os.listdir(wd)):
        if not name.endswith(".ecsv"):
            continue
        g = build(data_root, "tso/whtlt/" + name, midx)
        out = os.path.join(out_dir, "whtlt_" + name.replace("_whtlt.ecsv", "") + ".json")
        tsolib.write_golden(out, g)
        print(name, "rows", g["n_data_rows"], "series", [(s["flux_column"], s["flux_unit"], s["rows_with_finite_time"]) for s in g["series"]])


main()
