import math
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import tso_golden_lib as tsolib

IN_INT = (183, 188)
OUT_INT = ((1, 170), (201, 240))
BIN_EDGES = [0.6, 1.0, 2.0, 3.0, 4.0, 5.3]
CENTROID_INT = (171, 200)
T0_BJD_RECALLED = 2458779.375083
P_D_RECALLED = 1.4079405


def depth_block(S, E, k_in, k_out):
    n_in = len(k_in)
    n_out = len(k_out)
    f_in = float(np.sum(S[k_in]) / n_in)
    f_out = float(np.sum(S[k_out]) / n_out)
    d = 1.0 - f_in / f_out
    s_in_sc = float(np.std(S[k_in], ddof=1) / math.sqrt(n_in))
    s_out_sc = float(np.std(S[k_out], ddof=1) / math.sqrt(n_out))
    s_in_pl = float(math.sqrt(float(np.sum(E[k_in] ** 2))) / n_in)
    s_out_pl = float(math.sqrt(float(np.sum(E[k_out] ** 2))) / n_out)

    def sig(si, so):
        return (f_in / f_out) * math.sqrt((si / f_in) ** 2 + (so / f_out) ** 2)

    return {
        "n_in": n_in, "n_out": n_out, "F_in": f_in, "F_out": f_out, "D": d,
        "sigma_in_scatter": s_in_sc, "sigma_out_scatter": s_out_sc, "sigma_D_scatter": sig(s_in_sc, s_out_sc),
        "sigma_in_pipeline": s_in_pl, "sigma_out_pipeline": s_out_pl, "sigma_D_pipeline": sig(s_in_pl, s_out_pl),
        "sigma_D_larger": max(sig(s_in_sc, s_out_sc), sig(s_in_pl, s_out_pl)),
    }


def series(flux, err, elem_mask):
    f = np.where(elem_mask[None, :], flux, np.nan)
    fin = np.isfinite(f)
    S = np.nansum(f, axis=1)
    e2 = np.where(fin, err ** 2, 0.0)
    E = np.sqrt(np.sum(e2, axis=1))
    return S, E, fin.sum(axis=1)


def crossing(t, n, level, k0, k1):
    for k in range(k0, k1):
        a, b = n[k], n[k + 1]
        if (a - level) * (b - level) <= 0 and a != b:
            return float(t[k] + (level - a) * (t[k + 1] - t[k]) / (b - a))
    return None


def main():
    data_root = sys.argv[1]
    out_dir = sys.argv[2]
    rel = "tso/E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits"
    relw = "tso/whtlt/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv"
    midx = tsolib.manifest_index(data_root)
    f = tsolib.load_x1dints(os.path.join(data_root, rel))
    s = f["specs"][0]
    flux = s["FLUX"]
    err = s["FLUX_ERROR"]
    wave = s["WAVELENGTH"]
    npx = s["NPIXELS"]
    int_num = s["INT_NUM"]
    n_int, n_el = flux.shape
    w0 = wave[0]
    same_wave = bool(np.all(wave == w0[None, :]))
    k_in = np.where((int_num >= IN_INT[0]) & (int_num <= IN_INT[1]))[0]
    k_out = np.where(((int_num >= OUT_INT[0][0]) & (int_num <= OUT_INT[0][1])) | ((int_num >= OUT_INT[1][0]) & (int_num <= OUT_INT[1][1])))[0]
    all_el = np.ones(n_el, dtype=bool)
    common = np.isnan(flux).sum(axis=0) == 0
    W, EW, nfin = series(flux, err, all_el)
    med = float(np.median(W))
    norm = W / med
    rule_in = np.where(norm < 0.10)[0]
    variants = {}
    for vname, base_mask in (("A_pipeline_sum_nan_skipped", all_el), ("B_elements_finite_in_every_integration", common)):
        S, E, nf = series(flux, err, base_mask)
        block = {"white": depth_block(S, E, k_in, k_out)}
        block["white"]["n_elements_selected"] = int(base_mask.sum())
        block["white"]["finite_elements_per_integration_in_window"] = [int(nf[k_in].min()), int(nf[k_in].max())]
        block["white"]["finite_elements_per_integration_out_window"] = [int(nf[k_out].min()), int(nf[k_out].max())]
        bins = []
        for lo, hi in zip(BIN_EDGES[:-1], BIN_EDGES[1:]):
            m = base_mask & (w0 >= lo) & (w0 < hi)
            Sb, Eb, nfb = series(flux, err, m)
            b = depth_block(Sb, Eb, k_in, k_out)
            b["lo_um"] = lo
            b["hi_um"] = hi
            b["n_elements_selected"] = int(m.sum())
            b["finite_elements_per_integration_in_window"] = [int(nfb[k_in].min()), int(nfb[k_in].max())]
            b["finite_elements_per_integration_out_window"] = [int(nfb[k_out].min()), int(nfb[k_out].max())]
            bins.append(b)
        block["bins"] = bins
        variants[vname] = block
    f_out = variants["A_pipeline_sum_nan_skipped"]["white"]["F_out"]
    n_out_norm = W / f_out
    kc = np.where((int_num >= CENTROID_INT[0]) & (int_num <= CENTROID_INT[1]))[0]
    deficit = 1.0 - n_out_norm[kc]
    timing = {}
    for tname in ("TDB-MID", "MJD-AVG"):
        t = s[tname]
        cen = float(np.sum(t[kc] * deficit) / np.sum(deficit))
        kmin = int(np.argmin(W))
        t_in = crossing(t, n_out_norm, 0.5, kc[0], kmin)
        t_eg = crossing(t, n_out_norm, 0.5, kmin, kc[-1])
        timing[tname] = {
            "deficit_centroid": cen,
            "time_of_minimum": float(t[kmin]),
            "int_num_of_minimum": int(int_num[kmin]),
            "half_flux_crossing_ingress": t_in,
            "half_flux_crossing_egress": t_eg,
            "half_flux_midpoint": (t_in + t_eg) / 2.0,
            "half_flux_duration_min": (t_eg - t_in) * 1440.0,
        }
    cen_bjd = timing["TDB-MID"]["deficit_centroid"] + 2400000.5
    epoch = round((cen_bjd - T0_BJD_RECALLED) / P_D_RECALLED)
    predicted = T0_BJD_RECALLED + epoch * P_D_RECALLED
    npx_common_in = float(np.mean(npx[np.ix_(k_in, np.where(common)[0])]))
    npx_common_out = float(np.mean(npx[np.ix_(k_out, np.where(common)[0])]))
    base_npx = np.median(npx[k_out], axis=0)
    lost = [(int(int_num[k]), int(((npx[k] < base_npx - 1e-9) & common).sum())) for k in range(n_int) if int(((npx[k] < base_npx - 1e-9) & common).sum()) >= 10]
    w = tsolib.read_ecsv_raw(os.path.join(data_root, relw))
    wn = w["data_lines"][0].split(" ")
    wr = np.array([[float(x) for x in ln.split(" ")] for ln in w["data_lines"][1:]])
    wf = wr[:, wn.index("whitelight_flux_NRS1")]
    golden = {
        "golden": "tso/quicklook",
        "produced_by": tsolib.golden_meta("tso_golden_quicklook.py", "<data root> <goldens/tso dir>", [tsolib.file_ref(data_root, rel, midx), tsolib.file_ref(data_root, relw, midx)]),
        "caveat": "Regression pin of the level-3 file, not a planet measurement: between INT_NUM 180 and 191 the extraction lost pixels (NPIXELS falls from 7 to about 3), so in-transit FLUX is biased low. See npixels_diagnostic.",
        "label": "quick look, no limb darkening or systematics model",
        "series_key": {"detector": s["detector"], "sporder": int(s["sporder"])},
        "n_integrations": int(n_int),
        "n_elements": int(n_el),
        "wavelength_identical_in_all_rows": same_wave,
        "definitions": {
            "S_b(t)": "numpy nansum of FLUX over the selected elements of integration t (NaN skipped)",
            "e_b(t)": "sqrt of the sum of FLUX_ERROR^2 over the selected elements whose FLUX is finite",
            "F_in, F_out": "arithmetic mean of S_b over the window integrations",
            "D": "1 - F_in / F_out",
            "sigma_x_scatter": "sample standard deviation (ddof = 1) of S_b over the window, divided by sqrt(N)",
            "sigma_x_pipeline": "sqrt(sum over the window of e_b^2) / N",
            "sigma_D": "(F_in / F_out) * sqrt((sigma_in / F_in)^2 + (sigma_out / F_out)^2), research section 6.2 item 5",
            "bin membership": "lo <= WAVELENGTH < hi, WAVELENGTH taken from the first row",
            "variant A": "all elements, NaN skipped per integration (what the pipeline white light does)",
            "variant B": "only elements whose FLUX is finite in every one of the 240 integrations",
            "tolerance_hint": "numpy pairwise summation; a sequential sum differs at the 1e-15 relative level, compare with 1e-10 relative",
        },
        "windows": {
            "in_transit_int_num": [IN_INT[0], IN_INT[1]],
            "out_of_transit_int_num": [[OUT_INT[0][0], OUT_INT[0][1]], [OUT_INT[1][0], OUT_INT[1][1]]],
            "selection": "in: the contiguous integrations whose pipeline white light is below 0.10 of its median; out: everything at least 10 integrations away from the dip",
            "rule_check_int_num_below_0p10_of_median": [int(int_num[k]) for k in rule_in],
            "in_window_tdb_mid": [float(s["TDB-MID"][k_in[0]]), float(s["TDB-MID"][k_in[-1]])],
            "in_window_mjd_avg": [float(s["MJD-AVG"][k_in[0]]), float(s["MJD-AVG"][k_in[-1]])],
        },
        "bin_edges_um": BIN_EDGES,
        "white_light_pipeline": {
            "median": med,
            "normalised_by_median_int_num_176_to_195": [[int(int_num[k]), float(norm[k])] for k in range(175, 195)],
            "equals_whtlt_bitwise_rows": int(np.sum(W == wf)),
            "whtlt_rows": int(len(wf)),
        },
        "variants": variants,
        "timing": timing,
        "ephemeris_check": {
            "status": "T0 and P are the values recalled in research-tso.md section 8.4 (Vanderburg et al. 2020), not verified against the paper",
            "T0_BJD_TDB": T0_BJD_RECALLED, "P_d": P_D_RECALLED,
            "observed_centroid_BJD_TDB": cen_bjd,
            "epoch": int(epoch),
            "predicted_BJD_TDB": predicted,
            "observed_minus_predicted_min": (cen_bjd - predicted) * 1440.0,
        },
        "npixels_diagnostic": {
            "mean_NPIXELS_common_elements_in_window": npx_common_in,
            "mean_NPIXELS_common_elements_out_window": npx_common_out,
            "integrations_with_10_or_more_common_elements_below_baseline_NPIXELS": lost,
            "n_common_elements": int(common.sum()),
            "nan_flux_elements_per_integration_in_window": [int(np.isnan(flux[k]).sum()) for k in k_in],
            "nan_flux_elements_per_integration_out_window_min_max": [int(np.isnan(flux[k_out]).sum(axis=1).min()), int(np.isnan(flux[k_out]).sum(axis=1).max())],
        },
    }
    path = os.path.join(out_dir, "quicklook_jw02358-o030_nirspec_prism.json")
    tsolib.write_golden(path, golden)
    print("written", path)
    for vname, block in variants.items():
        print(vname)
        b = block["white"]
        print("  white n_el=%d  F_in=%r F_out=%r D=%r sD_sc=%r sD_pl=%r fin_in=%s fin_out=%s" % (b["n_elements_selected"], b["F_in"], b["F_out"], b["D"], b["sigma_D_scatter"], b["sigma_D_pipeline"], b["finite_elements_per_integration_in_window"], b["finite_elements_per_integration_out_window"]))
        for b in block["bins"]:
            print("  bin %.1f-%.1f n_el=%d F_in=%r F_out=%r D=%r sD_sc=%r sD_pl=%r fin_in=%s" % (b["lo_um"], b["hi_um"], b["n_elements_selected"], b["F_in"], b["F_out"], b["D"], b["sigma_D_scatter"], b["sigma_D_pipeline"], b["finite_elements_per_integration_in_window"]))
    print("timing", timing)
    print("ephemeris", golden["ephemeris_check"])
    print("npixels", golden["npixels_diagnostic"])
    print("rule in:", golden["windows"]["rule_check_int_num_below_0p10_of_median"], "same_wave", same_wave)


main()
