import json
import os
import sys

import numpy as np
from astropy.io import fits

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import tso_golden_lib as tsolib

SCALARS = ["INT_NUM", "N_ALONGDISP", "SEGMENT"] + tsolib.TIME_COLUMNS
VECTORS = tsolib.VECTOR_FLOAT_COLUMNS + ["DQ"]
HEADER_KEYS = ["FILENAME", "DATAMODL", "CAL_VER", "DETECTOR", "NINTS", "INTSTART", "INTEND", "EXSEGNUM", "EXSEGTOT", "NGROUPS", "TGROUP", "EFFINTTM", "EFFEXPTM", "DURATION", "DATE-OBS", "TIME-OBS", "DATE-BEG", "DATE-END", "EXPSTART", "EXPMID", "EXPEND", "BARTDELT", "BSTRTIME", "BMIDTIME", "BENDTIME", "DATE", "ASNTABLE", "S_OUTLIR", "S_PXREPL", "S_TRCMOD", "S_PHOTOM", "S_EXTR1D"]


def stack(specs):
    out = {}
    for c in SCALARS:
        out[c] = np.concatenate([s[c] for s in specs])
    nmax = max(s["nelem"] for s in specs)
    for c in VECTORS:
        parts = []
        for s in specs:
            a = s[c]
            if a.shape[1] != nmax:
                raise SystemExit("different nelem inside one series")
            parts.append(a)
        out[c] = np.concatenate(parts, axis=0)
    return out


def sort_by_time(t):
    order = np.argsort(t["MJD-AVG"], kind="stable")
    return {k: v[order] for k, v in t.items()}, order


def compare_tables(a, b):
    res = {"rows_a": int(len(a["INT_NUM"])), "rows_b": int(len(b["INT_NUM"])), "columns": {}}
    if len(a["INT_NUM"]) != len(b["INT_NUM"]):
        res["error"] = "row count differs"
        return res
    for c in SCALARS + VECTORS:
        x = a[c]
        y = b[c]
        same_shape = x.shape == y.shape
        entry = {"same_shape": same_shape}
        if same_shape:
            xf = x.astype(np.float64)
            yf = y.astype(np.float64)
            both_nan = np.isnan(xf) & np.isnan(yf)
            equal = (xf == yf) | both_nan
            entry["bitwise_equal_all"] = bool(equal.all())
            entry["n_elements"] = int(equal.size)
            entry["n_different"] = int((~equal).sum())
            nan_mismatch = np.isnan(xf) != np.isnan(yf)
            entry["n_nan_mismatch"] = int(nan_mismatch.sum())
            fin = ~np.isnan(xf) & ~np.isnan(yf) & ~equal
            if fin.any():
                d = np.abs(xf[fin] - yf[fin])
                entry["max_abs_diff"] = repr(float(d.max()))
                denom = np.maximum(np.abs(xf[fin]), np.abs(yf[fin]))
                with np.errstate(divide="ignore", invalid="ignore"):
                    rel = d / denom
                entry["max_rel_diff"] = repr(float(np.nanmax(rel)))
                entry["median_rel_diff"] = repr(float(np.nanmedian(rel)))
                if x.ndim == 2:
                    rows_diff = np.unique(np.where(~equal)[0])
                    entry["rows_with_difference"] = [int(v) for v in rows_diff[:50]]
                    entry["n_rows_with_difference"] = int(len(rows_diff))
        res["columns"][c] = entry
    return res


def primary_keys(path):
    with fits.open(path, memmap=False) as h:
        ph = h[0].header
        return {k: (ph[k] if k in ph else None) for k in HEADER_KEYS}


def white(spec_table, lo, hi):
    with np.errstate(invalid="ignore"):
        m = (spec_table["WAVELENGTH"] >= lo) & (spec_table["WAVELENGTH"] <= hi)
    f = spec_table["FLUX"].copy()
    f[~m] = np.nan
    return np.nansum(f, axis=1)


def run(label, level3_path, seg_paths, asn_path, out):
    l3 = tsolib.load_x1dints(level3_path)
    res = {"label": label, "level3": os.path.basename(level3_path), "segments": [os.path.basename(p) for p in seg_paths]}
    res["level3_primary"] = primary_keys(level3_path)
    res["segment_primary"] = [primary_keys(p) for p in seg_paths]
    res["level3_extract1d"] = [{"hdu": s["hdu_index"], "extver": s["extver"], "detector": s["detector"], "sporder": s["sporder"], "rows": int(len(s["INT_NUM"])), "INT_NUM": [int(v) for v in s["INT_NUM"]], "SEGMENT": sorted({int(v) for v in s["SEGMENT"]}), "MJD-AVG_first": repr(float(s["MJD-AVG"][0])), "MJD-AVG_last": repr(float(s["MJD-AVG"][-1]))} for s in l3["specs"]]
    with open(asn_path, "r", encoding="utf-8") as fh:
        asn = json.load(fh)
    res["asn"] = {"asn_type": asn.get("asn_type"), "asn_rule": asn.get("asn_rule"), "asn_id": asn.get("asn_id"), "program": asn.get("program"), "version_id": asn.get("version_id"), "code_version": asn.get("code_version"), "degraded_status": asn.get("degraded_status"), "target": asn.get("target"), "asn_pool": asn.get("asn_pool"), "top_level_keys": list(asn.keys()), "products": [{"name": p.get("name"), "members": [{k: m.get(k) for k in m} for m in p.get("members", [])]} for p in asn.get("products", [])]}
    keys = []
    for s in l3["specs"]:
        k = tsolib.series_key(s)
        if k not in keys:
            keys.append(k)
    res["series"] = []
    segs = [tsolib.load_x1dints(p) for p in seg_paths]
    for k in keys:
        l3_specs = [s for s in l3["specs"] if tsolib.series_key(s) == k]
        seg_specs = [s for f in segs for s in f["specs"] if tsolib.series_key(s) == k]
        entry = {"detector": k[0], "sporder": int(k[1]), "level3_extensions": len(l3_specs), "level2b_extensions": len(seg_specs)}
        if not seg_specs:
            entry["error"] = "no level-2b spectrum with this key"
            res["series"].append(entry)
            continue
        t3 = stack(l3_specs)
        t2 = stack(seg_specs)
        entry["level3_hdu_order_INT_NUM"] = [int(v) for v in t3["INT_NUM"]]
        entry["level3_hdu_order_time_strictly_increasing"] = bool(np.all(np.diff(t3["MJD-AVG"]) > 0))
        entry["level3_hdu_order_INT_NUM_strictly_increasing_by_1"] = bool(np.all(np.diff(t3["INT_NUM"]) == 1))
        entry["level3_gap_between_extensions_s"] = [repr(float((b["MJD-BEG"][0] - a["MJD-END"][-1]) * 86400.0)) for a, b in zip(l3_specs[:-1], l3_specs[1:])]
        entry["level3_step_inside_extension_s"] = repr(float((l3_specs[0]["MJD-AVG"][1] - l3_specs[0]["MJD-AVG"][0]) * 86400.0)) if len(l3_specs[0]["MJD-AVG"]) > 1 else None
        entry["level3_step_across_extension_boundary_s"] = [repr(float((b["MJD-AVG"][0] - a["MJD-AVG"][-1]) * 86400.0)) for a, b in zip(l3_specs[:-1], l3_specs[1:])]
        s3, o3 = sort_by_time(t3)
        s2, o2 = sort_by_time(t2)
        entry["level3_sort_is_identity"] = bool(np.all(o3 == np.arange(len(o3))))
        entry["level2b_sort_is_identity"] = bool(np.all(o2 == np.arange(len(o2))))
        entry["comparison_sorted_by_MJD-AVG"] = compare_tables(s3, s2)
        w3 = white(s3, -1.0, 1.0e10)
        w2 = white(s2, -1.0, 1.0e10)
        with np.errstate(divide="ignore", invalid="ignore"):
            rel = np.abs(w3 - w2) / np.abs(w3)
        entry["white_light_level3_vs_level2b_max_rel"] = repr(float(np.nanmax(rel)))
        entry["white_light_level3_first5"] = tsolib.f64_list(w3[:5])
        entry["white_light_level2b_first5"] = tsolib.f64_list(w2[:5])
        entry["white_light_bitwise_equal_rows"] = int((w3 == w2).sum())
        entry["flux_nan_level3"] = int(np.isnan(s3["FLUX"]).sum())
        entry["flux_nan_level2b"] = int(np.isnan(s2["FLUX"]).sum())
        res["series"].append(entry)
    out[label] = res


def main():
    out_path = sys.argv[1]
    data_tso = sys.argv[2]
    out = {}
    c = os.path.join(data_tso, "C")
    run("C", os.path.join(c, "jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits"),
        [os.path.join(c, "jw02327001001_03101_00001-seg%03d_nrs1_x1dints.fits" % k) for k in range(1, 7)],
        os.path.join(c, "jw02327-o001_20260925t123033_tso3_00001_asn.json"), out)
    a5 = os.path.join(data_tso, "A5")
    run("A5", os.path.join(a5, "jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits"),
        [os.path.join(a5, "jw03375001001_03101_00001-seg001_nrs1_x1dints.fits"), os.path.join(a5, "jw03375001001_03101_00001-seg001_nrs2_x1dints.fits")],
        os.path.join(a5, "jw03375-o001_20261013t072715_tso3_00001_asn.json"), out)
    with open(out_path, "w", encoding="utf-8") as fh:
        json.dump(out, fh, indent=1, default=str)
    if len(sys.argv) > 4:
        data_root = sys.argv[3]
        gold_dir = sys.argv[4]
        midx = tsolib.manifest_index(data_root)
        for label, r in out.items():
            inputs = [{"file": r["level3"], **{k: midx[r["level3"]][k] for k in ("set", "bytes", "sha256")}}]
            for sname in r["segments"]:
                inputs.append({"file": sname, **{k: midx[sname][k] for k in ("set", "bytes", "sha256")}})
            series = []
            for s in r["series"]:
                cmp = s["comparison_sorted_by_MJD-AVG"]
                equal = [c for c, e in cmp["columns"].items() if e.get("bitwise_equal_all")]
                diff = {c: {"n_different": e["n_different"], "n_elements": e["n_elements"], "n_nan_mismatch": e["n_nan_mismatch"], "max_abs_diff": float(e["max_abs_diff"]), "max_rel_diff": float(e["max_rel_diff"]), "median_rel_diff_of_differing": float(e["median_rel_diff"]), "rows_with_difference": e.get("n_rows_with_difference")} for c, e in cmp["columns"].items() if not e.get("bitwise_equal_all")}
                series.append({
                    "key": {"detector": s["detector"], "sporder": s["sporder"]},
                    "level3_extract1d_count": s["level3_extensions"],
                    "level2b_extract1d_count": s["level2b_extensions"],
                    "rows": cmp["rows_a"],
                    "level3_int_num_in_hdu_order": s["level3_hdu_order_INT_NUM"],
                    "level3_time_strictly_increasing_in_hdu_order": s["level3_hdu_order_time_strictly_increasing"],
                    "level3_int_num_step_is_1": s["level3_hdu_order_INT_NUM_strictly_increasing_by_1"],
                    "time_sort_is_identity_level3": s["level3_sort_is_identity"],
                    "time_sort_is_identity_level2b_concatenated_in_segment_order": s["level2b_sort_is_identity"],
                    "mjd_avg_step_inside_extension_s": float(s["level3_step_inside_extension_s"]) if s["level3_step_inside_extension_s"] else None,
                    "mjd_avg_step_across_extension_boundaries_s": [float(v) for v in s["level3_step_across_extension_boundary_s"]],
                    "columns_bitwise_equal_level3_vs_level2b": equal,
                    "columns_different": diff,
                    "flux_nan_count_level3": s["flux_nan_level3"],
                    "flux_nan_count_level2b": s["flux_nan_level2b"],
                    "white_light_open_limits_max_rel_diff_level3_vs_level2b": float(s["white_light_level3_vs_level2b_max_rel"]),
                })
            g = {
                "golden": "tso/stitch",
                "label": label,
                "produced_by": tsolib.golden_meta("tso_golden_stitch.py", "<scratch json> <data root>/tso <data root> <goldens/tso dir>", inputs),
                "comparison": "level-3 EXTRACT1D rows of one series, concatenated and sorted by MJD-AVG, against the level-2b segment rows concatenated and sorted the same way; NaN equals NaN",
                "level3_primary": {k: r["level3_primary"][k] for k in ("DETECTOR", "NINTS", "INTSTART", "INTEND", "EXSEGNUM", "EXSEGTOT", "EXPSTART", "EXPEND", "EFFEXPTM", "DURATION", "DATE-BEG", "DATE-END")},
                "segment_primary": [{k: sp[k] for k in ("FILENAME", "DETECTOR", "NINTS", "INTSTART", "INTEND", "EXSEGNUM", "EXSEGTOT", "EXPSTART", "EXPEND", "EFFEXPTM", "DURATION", "DATE-BEG", "DATE-END")} for sp in r["segment_primary"]],
                "level3_extract1d": r["level3_extract1d"],
                "association": {"asn_type": r["asn"]["asn_type"], "asn_rule": r["asn"]["asn_rule"], "code_version": r["asn"]["code_version"], "product_name": r["asn"]["products"][0]["name"], "members": [{"expname": m["expname"], "exptype": m["exptype"]} for m in r["asn"]["products"][0]["members"]]},
                "series": series,
            }
            for e in g["level3_extract1d"]:
                e["MJD-AVG_first"] = float(e["MJD-AVG_first"])
                e["MJD-AVG_last"] = float(e["MJD-AVG_last"])
            tsolib.write_golden(os.path.join(gold_dir, "stitch_%s.json" % r["level3"].replace("_x1dints.fits", "")), g)
            print("golden written for", label)
    for label, r in out.items():
        print("#", label)
        print("  level3 primary:", {k: r["level3_primary"][k] for k in ("DETECTOR", "NINTS", "INTSTART", "INTEND", "EXSEGNUM", "EXSEGTOT", "DATE-BEG", "DATE-END", "EXPSTART", "EXPEND", "EFFEXPTM", "DURATION")})
        for sp in r["segment_primary"]:
            print("  seg primary:", {k: sp[k] for k in ("FILENAME", "DETECTOR", "NINTS", "INTSTART", "INTEND", "EXSEGNUM", "EXSEGTOT", "DATE-BEG", "DATE-END", "EXPSTART", "EXPEND", "EFFEXPTM", "DURATION")})
        for e in r["level3_extract1d"]:
            print("  L3 ext:", e)
        print("  asn:", json.dumps(r["asn"])[:1800])
        for s in r["series"]:
            print("  series", s["detector"], s["sporder"], "l3 ext", s["level3_extensions"], "l2b ext", s["level2b_extensions"])
            if "error" in s:
                print("    ", s["error"])
                continue
            print("    INT_NUM hdu order:", s["level3_hdu_order_INT_NUM"])
            print("    time increasing:", s["level3_hdu_order_time_strictly_increasing"], "int+1:", s["level3_hdu_order_INT_NUM_strictly_increasing_by_1"], "sort identity l3/l2b:", s["level3_sort_is_identity"], s["level2b_sort_is_identity"])
            print("    step inside ext s:", s["level3_step_inside_extension_s"], "across boundaries:", s["level3_step_across_extension_boundary_s"], "gap end->beg:", s["level3_gap_between_extensions_s"])
            cmp = s["comparison_sorted_by_MJD-AVG"]
            print("    rows", cmp["rows_a"], cmp["rows_b"])
            for c, e in cmp["columns"].items():
                print("      %-17s equal=%s diff=%s/%s nanmis=%s maxabs=%s maxrel=%s medrel=%s rows=%s" % (c, e.get("bitwise_equal_all"), e.get("n_different"), e.get("n_elements"), e.get("n_nan_mismatch"), e.get("max_abs_diff"), e.get("max_rel_diff"), e.get("median_rel_diff"), e.get("n_rows_with_difference")))
            print("    white light l3 vs l2b max rel:", s["white_light_level3_vs_level2b_max_rel"], "bitwise rows:", s["white_light_bitwise_equal_rows"], "flux nan l3/l2b:", s["flux_nan_level3"], s["flux_nan_level2b"])
            print("      l3 first5", s["white_light_level3_first5"])
            print("      l2 first5", s["white_light_level2b_first5"])


main()
