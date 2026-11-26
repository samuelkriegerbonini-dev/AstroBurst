import math
import os
import struct
import sys

import numpy as np
from astropy.io import fits

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import tso_golden_lib as tsolib

OPEN = (-1.0, 1.0e10)
SOSS_OWN = {1: (0.93, 2.82), 2: (0.64, 0.83), 3: (0.64, 0.95)}
W = "tso/whtlt/"

CASES = [
    {"label": "A1", "rel": "tso/A1/jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits", "whtlt": W + "jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_whtlt.ecsv",
     "archived": lambda o: (2.47, 3.94), "archived_source": "white_light step parameters recorded in the ASDF HDU cal_logs of this file: min_wavelength 2.47, max_wavelength 3.94 (jwst_nircam_pars-whitelightstep_0005.asdf)"},
    {"label": "A3", "rel": "tso/A3/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1dints.fits", "whtlt": W + "jw07073-o004_t001_niriss_clear-gr700xd-substrip256_whtlt.ecsv",
     "archived": lambda o: SOSS_OWN[1], "archived_source": "order 1 CLEAR range 0.93 to 2.82 um reused for every order (range-reuse quirk, confirmed bitwise on this file); step parameters in cal_logs are None",
     "extra": {"soss_own_range_per_order": lambda o: SOSS_OWN[int(o)]}},
    {"label": "A4", "rel": "tso/A4/jw04498-o013_t008_miri_p750l-slitlessprism_x1dints.fits", "whtlt": W + "jw04498-o013_t008_miri_p750l-slitlessprism_whtlt.ecsv",
     "archived": lambda o: (5.0, 12.0), "archived_source": "white_light step parameters recorded in the ASDF HDU cal_logs of this file: min_wavelength 5.0, max_wavelength 12.0 (jwst_miri_pars-whitelightstep_0001.asdf)"},
    {"label": "A5", "rel": "tso/A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits", "whtlt": W + "jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_whtlt.ecsv",
     "archived": lambda o: OPEN, "archived_source": "cal_logs: no pars-whitelightstep match, min_wavelength None, max_wavelength None, so -1.0 and 1.0e10"},
    {"label": "A5a", "rel": "tso/A5/jw03375001001_03101_00001-seg001_nrs1_x1dints.fits", "whtlt": W + "jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_whtlt.ecsv", "level2b": True,
     "archived": lambda o: OPEN, "archived_source": "level 2b has no whtlt; open limits used and compared with the level-3 whtlt of the same observation"},
    {"label": "A5b", "rel": "tso/A5/jw03375001001_03101_00001-seg001_nrs2_x1dints.fits", "whtlt": W + "jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_whtlt.ecsv", "level2b": True,
     "archived": lambda o: OPEN, "archived_source": "level 2b has no whtlt; open limits used and compared with the level-3 whtlt of the same observation"},
    {"label": "A6", "rel": "tso/A6/jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512_x1dints.fits", "whtlt": W + "jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512_whtlt.ecsv",
     "archived": lambda o: OPEN, "archived_source": "cal_logs: no pars-whitelightstep match, min_wavelength None, max_wavelength None, so -1.0 and 1.0e10"},
    {"label": "B1", "rel": "tso/B1/jw01274-o013_t008_miri_p750l-slitlessprism_x1dints.fits", "whtlt": W + "jw01274-o013_t008_miri_p750l-slitlessprism_whtlt.ecsv",
     "archived": lambda o: (5.0, 12.0), "archived_source": "not recorded in this cal_ver 1.15.1 file; 5.0 to 12.0 um is the MIRI value seen in A4 and reproduces the legacy whtlt to 3e-16 relative (window search: min in (4.9957, 5.0763], max in [11.9833, 12.0003))"},
    {"label": "C1", "rel": "tso/C/jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits", "whtlt": W + "jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_whtlt.ecsv",
     "archived": lambda o: OPEN, "archived_source": "cal_logs: no pars-whitelightstep match, min_wavelength None, max_wavelength None, so -1.0 and 1.0e10"},
]
for k in range(1, 7):
    CASES.append({"label": "C%d" % (k + 1), "rel": "tso/C/jw02327001001_03101_00001-seg%03d_nrs1_x1dints.fits" % k, "whtlt": W + "jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_whtlt.ecsv", "level2b": True,
                  "archived": lambda o: OPEN, "archived_source": "level 2b has no whtlt; open limits used and compared with the level-3 whtlt of the same observation"})
CASES.append({"label": "E1", "rel": "tso/E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits", "whtlt": W + "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv",
              "archived": lambda o: OPEN, "archived_source": "cal_logs: no pars-whitelightstep match, min_wavelength None, max_wavelength None, so -1.0 and 1.0e10"})


def read_whtlt(path):
    raw = tsolib.read_ecsv_raw(path)
    names = raw["data_lines"][0].split(" ")
    rows = [ln.split(" ") for ln in raw["data_lines"][1:]]
    return names, {n: np.array([float(r[i]) for r in rows], dtype=np.float64) for i, n in enumerate(names)}


def whtlt_names(wnames, detector, sporder, multi_order, multi_det):
    if "MJD" in wnames:
        return "MJD", None, "whitelight_flux"
    flux = "whitelight_flux"
    if multi_order:
        flux += "_order_%d" % sporder
    det = ""
    if multi_det or detector in ("NRS1", "NRS2"):
        det = "_" + detector
    return "MJD_UTC" + det, "BJD_TDB" + det, flux + det


def storage_block(path, spec):
    with fits.open(path, memmap=False) as hdul:
        hdu = hdul[spec["hdu_index"]]
        loc = hdu.fileinfo()["datLoc"]
        h = hdu.header
        names = [h["TTYPE%d" % i] for i in range(1, h["TFIELDS"] + 1)]
        out = {"NAXIS1": h["NAXIS1"], "NAXIS2": h["NAXIS2"], "PCOUNT": h["PCOUNT"], "TFIELDS": h["TFIELDS"]}
        for nm in ("WAVELENGTH", "DQ", "N_ALONGDISP", "INT_NUM"):
            if nm in names:
                i = names.index(nm) + 1
                out[nm] = {"column": i, "TFORM": h["TFORM%d" % i], "TDIM": h.get("TDIM%d" % i), "TUNIT": h.get("TUNIT%d" % i), "TZERO": h.get("TZERO%d" % i)}
    n = spec["nelem"]
    with open(path, "rb") as fh:
        if spec["layout"] == "row_per_integration":
            fh.seek(loc + 4 + 140 * n)
            b = fh.read(4)
            out["N_ALONGDISP"]["row0_stored_bytes_hex"] = b.hex()
            out["N_ALONGDISP"]["row0_stored_int32"] = struct.unpack(">i", b)[0]
            out["N_ALONGDISP"]["row0_value"] = struct.unpack(">i", b)[0] + 2147483648
            fh.seek(loc + 4 + 88 * n)
            b = fh.read(4)
            out["DQ"]["row0_element0_stored_bytes_hex"] = b.hex()
            out["DQ"]["row0_element0_value"] = struct.unpack(">i", b)[0] + 2147483648
            out["row_length_rule"] = "NAXIS1 = 140 * repeat + 60"
            out["row_length_rule_holds"] = (140 * n + 60) == out["NAXIS1"]
        else:
            fh.seek(loc + 88)
            b = fh.read(4)
            out["DQ"]["row0_stored_bytes_hex"] = b.hex()
            out["DQ"]["row0_value"] = struct.unpack(">i", b)[0] + 2147483648
            out["row_length_rule"] = "NAXIS1 = 17 * 8 + 4 = 140"
            out["row_length_rule_holds"] = out["NAXIS1"] == 140
    return out


def white_block(group, limits, wnames, wdata, tname, bname, fname, level2b):
    mjd, tdb, sums, ints, fs = [], [], [], [], []
    rows_total = 0
    for s in group:
        r = tsolib.white_light_rows(s, limits[0], limits[1])
        g = r["good"]
        rows_total += len(g)
        mjd.append(r["mid_mjd"][g])
        tdb.append(r["tdb_mid"][g])
        sums.append(r["flux_sum"][g])
        ints.append(s["INT_NUM"][g])
        wave = s["WAVELENGTH"]
        with np.errstate(invalid="ignore"):
            m = (wave >= limits[0]) & (wave <= limits[1]) & np.isfinite(s["FLUX"])
        for k in np.where(g)[0]:
            fs.append(math.fsum(s["FLUX"][k][m[k]].tolist()))
    mjd = np.concatenate(mjd)
    tdb = np.concatenate(tdb)
    sums = np.concatenate(sums)
    ints = np.concatenate(ints)
    fs = np.array(fs)
    order = np.argsort(mjd, kind="stable")
    mjd, tdb, sums, ints, fs = mjd[order], tdb[order], sums[order], ints[order], fs[order]
    n = len(sums)
    head = list(range(min(5, n)))
    tail = list(range(max(0, n - 5), n))
    with np.errstate(divide="ignore", invalid="ignore"):
        sens = np.abs(sums - fs) / np.abs(fs)
    blk = {
        "limits_um": [limits[0], limits[1]],
        "rows_in_series": rows_total,
        "rows_kept": n,
        "rows_dropped_nan_or_repeated_mjd_avg": rows_total - n,
        "sorted_by": "MJD-AVG ascending",
        "first5": [{"int_num": int(ints[k]), "mjd_avg": float(mjd[k]), "tdb_mid": float(tdb[k]), "sum": float(sums[k])} for k in head],
        "last5": [{"int_num": int(ints[k]), "mjd_avg": float(mjd[k]), "tdb_mid": float(tdb[k]), "sum": float(sums[k])} for k in tail],
        "sum_all_fsum": math.fsum(sums.tolist()),
        "numpy_nansum_vs_exact_sum_max_rel": float(np.nanmax(sens)) if n else None,
    }
    if fname is not None and fname in wdata:
        wt = wdata[tname]
        wf = wdata[fname]
        pairs = []
        for i in range(len(wt)):
            if not np.isfinite(wt[i]):
                continue
            j = np.where(mjd == wt[i])[0]
            if len(j) == 1:
                pairs.append((i, int(j[0])))
        rel = [abs(sums[j] - wf[i]) / abs(wf[i]) if wf[i] != 0 else float("inf") for i, j in pairs]
        wsel = np.array([wf[i] for i, _ in pairs])
        wtime = np.array([wt[i] for i, _ in pairs])
        nw = len(pairs)
        blk["whtlt"] = {
            "time_column": tname, "bjd_column": bname, "flux_column": fname,
            "whtlt_rows_finite_time": int(np.isfinite(wt).sum()),
            "rows_joined_on_identical_time": nw,
            "first5": [{"time": float(wtime[k]), "flux": float(wsel[k])} for k in range(min(5, nw))],
            "last5": [{"time": float(wtime[k]), "flux": float(wsel[k])} for k in range(max(0, nw - 5), nw)],
            "sum_all_fsum": math.fsum(wsel.tolist()),
            "max_rel_diff_computed_vs_whtlt": float(max(rel)) if rel else None,
            "rows_bitwise_equal": int(sum(1 for i, j in pairs if sums[j] == wf[i])),
            "within_1e-4": bool(max(rel) <= 1e-4) if rel else None,
            "expected_to_match": not level2b,
        }
        if bname is not None and bname in wdata and pairs:
            blk["whtlt"]["bjd_equals_tdb_mid_bitwise_rows"] = int(sum(1 for i, j in pairs if wdata[bname][i] == tdb[j]))
    return blk


def all_rows_block(group, limits):
    sums, ints, tdb = [], [], []
    for s in group:
        wave = s["WAVELENGTH"]
        with np.errstate(invalid="ignore"):
            m = (wave >= limits[0]) & (wave <= limits[1])
        fl = s["FLUX"].copy()
        fl[~m] = np.nan
        sums.append(np.nansum(fl, axis=1))
        ints.append(s["INT_NUM"])
        tdb.append(s["TDB-MID"])
    sums = np.concatenate(sums)
    ints = np.concatenate(ints)
    tdb = np.concatenate(tdb)
    n = len(sums)
    return {
        "meaning": "per-row sums in HDU and row order with no row dropped; use it to test the sum when the time rule removes rows",
        "limits_um": [limits[0], limits[1]],
        "rows": n,
        "first5": [{"int_num": int(ints[k]), "tdb_mid": float(tdb[k]), "sum": float(sums[k])} for k in range(min(5, n))],
        "last5": [{"int_num": int(ints[k]), "tdb_mid": float(tdb[k]), "sum": float(sums[k])} for k in range(max(0, n - 5), n)],
        "sum_all_fsum": math.fsum(sums.tolist()),
    }


def build(case, data_root, midx):
    path = os.path.join(data_root, case["rel"])
    f = tsolib.load_x1dints(path)
    wpath = os.path.join(data_root, case["whtlt"])
    wnames, wdata = read_whtlt(wpath)
    with fits.open(path, memmap=False) as hdul:
        hdus = [{"index": i, "name": h.name, "extver": h.header.get("EXTVER"), "naxis1": h.header.get("NAXIS1"), "naxis2": h.header.get("NAXIS2")} for i, h in enumerate(hdul)]
        ph = hdul[0].header
        primary = {k: ph.get(k) for k in ("DATAMODL", "CAL_VER", "EXP_TYPE", "INSTRUME", "DETECTOR", "FILTER", "PUPIL", "GRATING", "SUBARRAY", "NINTS", "INTSTART", "INTEND", "EXSEGNUM", "EXSEGTOT", "TARGNAME", "S_PHOTOM")}
    specs = f["specs"]
    keys = []
    for s in specs:
        k = tsolib.series_key(s)
        if k not in keys:
            keys.append(k)
    multi_det = len({k[0] for k in keys}) > 1
    multi_order = len({k[1] for k in keys}) > 1
    level2b = bool(case.get("level2b"))
    if len(hdus) > 14:
        hdu_list = hdus[:3] + [{"omitted": len(hdus) - 5}] + hdus[-2:]
    else:
        hdu_list = hdus
    out = {
        "golden": "tso/x1dints",
        "label": case["label"],
        "produced_by": tsolib.golden_meta("tso_golden_x1dints.py", "<data root> <goldens/tso dir>", [tsolib.file_ref(data_root, case["rel"], midx), tsolib.file_ref(data_root, case["whtlt"], midx)]),
        "primary": primary,
        "product_level": "2b" if level2b else "3",
        "layout": specs[0]["layout"],
        "n_hdus": len(hdus),
        "hdus": hdu_list,
        "n_extract1d": len(specs),
        "int_times_rows": int(len(f["int_times"]["integration_number"])) if f["int_times"] is not None else None,
        "series_keys": [{"detector": k[0], "sporder": int(k[1])} for k in keys],
        "series": [],
    }
    for k in keys:
        group = [s for s in specs if tsolib.series_key(s) == k]
        det, sporder = k
        tname, bname, fname = whtlt_names(wnames, det, int(sporder), multi_order, multi_det)
        if level2b and len({s2["detector"] for s2 in specs}) == 1 and det in ("NRS1", "NRS2"):
            tname, bname, fname = "MJD_UTC_" + det, "BJD_TDB_" + det, "whitelight_flux_" + det
        nal = np.concatenate([s["N_ALONGDISP"] for s in group])
        rep = np.concatenate([np.full(len(s["N_ALONGDISP"]), s["nelem"]) for s in group])
        intn = np.concatenate([s["INT_NUM"] for s in group])
        seg = np.concatenate([s["SEGMENT"] for s in group])
        mjd = np.concatenate([s["MJD-AVG"] for s in group])
        tdb = np.concatenate([s["TDB-MID"] for s in group])
        mbeg = np.concatenate([s["MJD-BEG"] for s in group])
        mend = np.concatenate([s["MJD-END"] for s in group])
        flux_nan = 0
        dq_nonzero = 0
        nan_dq0 = 0
        finite_dq1 = 0
        ferr_nan = 0
        n_valid = 0
        for s in group:
            valid = np.arange(s["nelem"])[None, :] < s["N_ALONGDISP"][:, None]
            fn = np.isnan(s["FLUX"]) & valid
            flux_nan += int(fn.sum())
            dq_nonzero += int(((s["DQ"] != 0) & valid).sum())
            nan_dq0 += int((fn & (s["DQ"] == 0)).sum())
            finite_dq1 += int((valid & ~np.isnan(s["FLUX"]) & (s["DQ"] != 0)).sum())
            ferr_nan += int((np.isnan(s["FLUX_ERROR"]) & valid).sum())
            n_valid += int(valid.sum())
        first = None
        for s in group:
            for r in range(len(s["N_ALONGDISP"])):
                n0 = int(s["N_ALONGDISP"][r])
                if n0 > 0 and np.isfinite(s["WAVELENGTH"][r, :n0]).any():
                    first = (s, r, n0)
                    break
            if first:
                break
        s0, r0, n0 = first
        w0 = s0["WAVELENGTH"][r0, :n0]
        d = np.diff(w0)
        finite_mjd = np.isfinite(mjd)
        finite_tdb = np.isfinite(tdb)
        entry = {
            "key": {"detector": det, "sporder": int(sporder)},
            "detector_source": "EXTRACT1D header" if group[0]["detector_card"] is not None else "primary header (EXTRACT1D has no DETECTOR card)",
            "extract1d_hdus": [s["hdu_index"] for s in group] if len(group) <= 12 else {"first": group[0]["hdu_index"], "last": group[-1]["hdu_index"], "count": len(group)},
            "rows_per_extract1d": [int(len(s["N_ALONGDISP"])) for s in group] if group[0]["layout"] == "row_per_integration" else {"each": int(group[0]["naxis2"]), "meaning": "spectral elements, one integration per extension"},
            "integrations": int(len(intn)),
            "tform_repeat": sorted({int(s["nelem"]) for s in group}),
            "n_alongdisp": {"min": int(nal.min()), "max": int(nal.max()), "sum": int(nal.sum()), "rows_zero": int((nal == 0).sum()), "rows_below_tform_repeat": int((nal < rep).sum()), "source": "N_ALONGDISP column" if group[0]["layout"] == "row_per_integration" else "NAXIS2 of each extension (legacy layout has no N_ALONGDISP)"},
            "int_num": {"first": int(intn[0]), "last": int(intn[-1]), "min": int(intn.min()), "max": int(intn.max()), "distinct": int(len(np.unique(intn))), "source": "INT_NUM column" if group[0]["layout"] == "row_per_integration" else "INT_NUM keyword of each EXTRACT1D"},
            "segment_values": sorted({int(v) for v in seg}),
            "units": {"WAVELENGTH": group[0]["tunit"].get("WAVELENGTH"), "FLUX": group[0]["tunit"].get("FLUX"), "FLUX_ERROR": group[0]["tunit"].get("FLUX_ERROR"), "MJD-AVG": group[0]["tunit"].get("MJD-AVG"), "TDB-MID": group[0]["tunit"].get("TDB-MID")},
            "time": {
                "mjd_avg_first": float(mjd[0]) if finite_mjd[0] else None, "mjd_avg_last": float(mjd[-1]) if finite_mjd[-1] else None,
                "mjd_avg_min_finite": float(np.nanmin(mjd)), "mjd_avg_max_finite": float(np.nanmax(mjd)),
                "mjd_avg_nan_rows": int((~finite_mjd).sum()), "mjd_avg_distinct_finite": int(len(np.unique(mjd[finite_mjd]))),
                "tdb_mid_first": float(tdb[0]) if finite_tdb[0] else None, "tdb_mid_last": float(tdb[-1]) if finite_tdb[-1] else None,
                "tdb_mid_nan_rows": int((~finite_tdb).sum()), "tdb_mid_distinct_finite": int(len(np.unique(tdb[finite_tdb]))),
                "mjd_beg_first": float(mbeg[0]), "mjd_end_last": float(mend[-1]),
                "strictly_increasing_in_hdu_and_row_order": bool(np.all(np.diff(mjd[finite_mjd]) > 0)) if finite_mjd.sum() > 1 else None,
                "source": "per-row columns" if group[0]["layout"] == "row_per_integration" else "keywords MJD-AVG and TDB-MID of each EXTRACT1D",
            },
            "first_valid_row": {"hdu": s0["hdu_index"], "row0": int(r0), "int_num": int(s0["INT_NUM"][r0]), "n_alongdisp": n0, "wavelength_first3": [float(v) for v in w0[:3]], "wavelength_last3": [float(v) for v in w0[-3:]], "direction": "ascending" if np.all(d > 0) else ("descending" if np.all(d < 0) else "mixed")},
            "masking_counts": {"valid_elements": n_valid, "flux_nan": flux_nan, "dq_nonzero": dq_nonzero, "flux_nan_with_dq_zero": nan_dq0, "flux_finite_with_dq_nonzero": finite_dq1, "flux_error_nan": ferr_nan},
            "storage": storage_block(path, group[0]),
        }
        lim = case["archived"](int(sporder))
        wl = white_block(group, lim, wnames, wdata, tname, bname, fname, level2b)
        wl["limits_source"] = case["archived_source"]
        entry["white_light_as_archived"] = wl
        if wl["rows_dropped_nan_or_repeated_mjd_avg"] > 0:
            entry["white_light_all_rows_archived_limits"] = all_rows_block(group, lim)
        if lim != OPEN:
            entry["white_light_open_limits"] = white_block(group, OPEN, wnames, wdata, tname, bname, fname, True)
            entry["white_light_open_limits"]["whtlt"]["expected_to_match"] = False
        for name, fn in case.get("extra", {}).items():
            blk = white_block(group, fn(int(sporder)), wnames, wdata, tname, bname, fname, True)
            blk["whtlt"]["expected_to_match"] = int(sporder) == 1
            entry["white_light_" + name] = blk
        out["series"].append(entry)
    return out


def main():
    data_root = sys.argv[1]
    out_dir = sys.argv[2]
    midx = tsolib.manifest_index(data_root)
    for case in CASES:
        g = build(case, data_root, midx)
        name = "x1dints_" + os.path.basename(case["rel"]).replace("_x1dints.fits", "") + ".json"
        tsolib.write_golden(os.path.join(out_dir, name), g)
        for s in g["series"]:
            wl = s["white_light_as_archived"]
            wh = wl.get("whtlt", {})
            print("%-4s %-70s key=%s/%d ints=%d kept=%d lim=%s maxrel=%s bitwise=%s/%s sens=%s" % (case["label"], name, s["key"]["detector"], s["key"]["sporder"], s["integrations"], wl["rows_kept"], wl["limits_um"], wh.get("max_rel_diff_computed_vs_whtlt"), wh.get("rows_bitwise_equal"), wh.get("rows_joined_on_identical_time"), wl["numpy_nansum_vs_exact_sum_max_rel"]))
            for extra in [k for k in s if k.startswith("white_light_") and k != "white_light_as_archived"]:
                e = s[extra]
                print("       %-40s lim=%s maxrel=%s bitwise=%s" % (extra, e["limits_um"], e.get("whtlt", {}).get("max_rel_diff_computed_vs_whtlt"), e.get("whtlt", {}).get("rows_bitwise_equal")))


main()
