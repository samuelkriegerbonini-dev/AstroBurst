import sys, os, math, json, re, glob
import astropy
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from tso_oracle import *

L3 = {
    "A1": "A1/jw02459-o013_t007_nircam_f322w2-grismr-subgrism64_x1dints.fits",
    "A3": "A3/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1dints.fits",
    "A4": "A4/jw04498-o013_t008_miri_p750l-slitlessprism_x1dints.fits",
    "A5": "A5/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits",
    "A6": "A6/jw04967-o005_t004_nirspec_clear-prism-s1600a1-sub512_x1dints.fits",
    "B1": "B1/jw01274-o013_t008_miri_p750l-slitlessprism_x1dints.fits",
    "C1": "C/jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits",
    "E1": "E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits",
}
WHTLT = {k: os.path.basename(v).replace("_x1dints.fits", "_whtlt.ecsv") for k, v in L3.items()}


def cmd_curves(root, sigma):
    for label, rel in L3.items():
        infos, series = open_set([os.path.join(root, rel)])
        for key, s in series.items():
            (lo, hi), origin = defaults(infos[0], key, "pipeline")
            p = pipeline(s, lo, hi)
            print("==", label, key, "time_scale", s["scale"], "rows", len(s["time"]), "channels", s["w"].shape[1], "common_grid", s["common_grid"], "level", infos[0]["level"])
            print("  pipeline limits", (lo, hi), origin, "kept", len(p["flux"]), "dropped (nan_time, repeated_time)", p["dropped"], "n_used min/max", int(p["n_used"].min()), int(p["n_used"].max()))
            show("flux", p["flux"]); show("flux_err", p["err"]); show("npixels_mean", p["npix"])
            (mlo, mhi), morigin = defaults(infos[0], key, "masked")
            m = masked(s, mlo, mhi)
            print("  masked limits", (mlo, mhi), morigin, "rows", len(m["flux"]), "left out (INT_NUM)", m["left_out"], "valid in every row kept", m["n_valid_in_every_row"], "of", s["w"].shape[1], "n_used", m["n_used"])
            show("flux", m["flux"]); show("flux_err", m["err"]); show("npixels_mean", m["npix"])
            r = masked(s, mlo, mhi, sigma=sigma)
            print("  masked + reject", sigma, ": n_used", r["n_used"], "rejected (index, wavelength, time-median, neighbour median, z, neighbours)", [(a, repr(b), repr(c), repr(d), "%.6g" % z, n) for a, b, c, d, z, n in r["rejected"]])
            if r["rejected"]:
                show("flux", r["flux"]); show("flux_err", r["err"])


def cmd_pick(root):
    infos, series = open_set([os.path.join(root, L3["E1"])])
    s = series[("NRS1", 0)]
    factor = math.sqrt(math.pi / 2)
    print("median error factor sqrt(pi/2)", repr(factor))

    def reduce(rows, median):
        v = s["valid"][rows]; f = s["f"][rows]; e = s["e"][rows]
        out = []
        for j in range(f.shape[1]):
            ok = v[:, j]
            n = int(ok.sum())
            if n == 0:
                out.append((float("nan"), float("nan"), 0)); continue
            eok = ok & np.isfinite(e[:, j]) & (e[:, j] > 0)
            mean_err = math.sqrt(math.fsum(e[eok, j] ** 2)) / n if int(eok.sum()) == n else float("nan")
            out.append((float(np.median(f[ok, j])), factor * mean_err, n) if median else (math.fsum(f[ok, j]) / n, mean_err, n))
        return out
    med = reduce(slice(None), True)
    mean = reduce(slice(0, 10), False)
    for j in (0, 14, 15, 16, 200, 430):
        print("channel", j, "wavelength", repr(float(s["w"][0][j])), "| median", repr(med[j][0]), repr(med[j][1]), "n", med[j][2], "| mean of rows 0..9", repr(mean[j][0]), repr(mean[j][1]), "n", mean[j][2],
              "| integration index 185", repr(float(s["f"][185][j])), repr(float(s["e"][185][j])), "dq", int(s["q"][185][j]))
    print("INT_NUM at index 185", int(s["int_num"][185]), "time", repr(float(s["time"][185])))
    print("channels with no valid sample", sum(1 for m in med if m[2] == 0), "channels valid in every row", int(s["valid"].all(axis=0).sum()))


def whtlt_series(path):
    from astropy.table import Table
    t = Table.read(path, format="ascii.ecsv")
    names = t.colnames
    schema = "legacy" if "MJD" in names and not any(n.startswith("MJD_UTC") for n in names) else "current"
    out = []
    for n in names:
        m = re.fullmatch("whitelight_flux(?:_order_([0-9]+))?(?:_([A-Z0-9]+))?", n)
        if not m:
            continue
        order = int(m.group(1)) if m.group(1) else None
        det = m.group(2)
        suffix = "_" + det if det else ""
        tcol = "MJD" if schema == "legacy" else "MJD_UTC" + suffix
        mjd = np.asarray(t[tcol], "f8"); flux = np.asarray(t[n], "f8")
        ok = np.isfinite(mjd) & np.isfinite(flux)
        out.append(dict(column=n, detector=det or t.meta.get("detector"), sporder=order, mjd=mjd[ok], flux=flux[ok], unit=str(t[n].unit) if t[n].unit is not None else None))
    return schema, out


def compare(paths, wpath, label):
    infos, series = open_set(paths)
    schema, ws = whtlt_series(wpath)
    print("==", label, "schema", schema)
    for w in ws:
        c = [k for k in series if (w["detector"] is None or k[0].upper() == str(w["detector"]).upper())]
        if w["sporder"] is not None:
            c = [k for k in c if k[1] == w["sporder"]]
        if len(c) != 1:
            print("   no single match for", w["column"]); continue
        key = c[0]
        s = series[key]
        (lo, hi), origin = defaults(infos[0], key, "pipeline")
        p = pipeline(s, lo, hi)
        joined = 0; worst = 0.0
        for tm, fl in zip(w["mjd"], w["flux"]):
            d = np.abs(p["mjd"] - tm)
            j = int(np.argmin(d))
            if d[j] <= 1e-9:
                joined += 1
                worst = max(worst, abs(p["flux"][j] - fl) / abs(fl))
        print("  ", w["column"], "unit", w["unit"], "->", key, "n_whtlt", len(w["mjd"]), "n_joined", joined, "max_rel_diff", repr(float(worst)), "limits", (lo, hi))


def cmd_whtlt(root):
    w = os.path.join(root, "whtlt")
    for label in L3:
        compare([os.path.join(root, L3[label])], os.path.join(w, WHTLT[label]), label + " level 3")
    a5 = [os.path.join(root, "A5", "jw03375001001_03101_00001-seg001_nrs1_x1dints.fits"), os.path.join(root, "A5", "jw03375001001_03101_00001-seg001_nrs2_x1dints.fits")]
    compare(a5, os.path.join(w, WHTLT["A5"]), "A5 level 2b")
    segs = sorted(glob.glob(os.path.join(root, "C", "jw02327001001_03101_00001-seg00*_nrs1_x1dints.fits")))
    compare(list(reversed(segs)), os.path.join(w, WHTLT["C1"]), "C six level 2b segments, given in reverse order")
    infos3, s3 = open_set([os.path.join(root, L3["C1"])])
    infos2, s2 = open_set(list(reversed(segs)))
    a, b = s3[("NRS1", -1)], s2[("NRS1", -1)]

    def bits(x):
        return x.view("u8")
    print("stitch: rows", len(a["time"]), len(b["time"]), "INT_NUM equal", bool((a["int_num"] == b["int_num"]).all()), "SEGMENT equal", bool((a["segment"] == b["segment"]).all()),
          "TDB-MID bits equal", bool((bits(a["tdb"]) == bits(b["tdb"])).all()), "MJD-AVG bits equal", bool((bits(a["mjd"]) == bits(b["mjd"])).all()),
          "WAVELENGTH bits equal", bool((bits(a["w"]) == bits(b["w"])).all()), "DQ equal", bool((a["q"] == b["q"]).all()))
    diff = ~(np.isnan(a["f"]) & np.isnan(b["f"])) & (a["f"] != b["f"])
    print("FLUX elements differing", int(diff.sum()), "of", a["f"].size)


def box_depth(flux, err, t_in, t_out):
    fi, fo = flux[t_in], flux[t_out]
    f_in, f_out = fi.mean(), fo.mean()
    s_in, s_out = fi.std(ddof=1) / math.sqrt(len(fi)), fo.std(ddof=1) / math.sqrt(len(fo))
    p_in, p_out = math.sqrt((err[t_in] ** 2).sum()) / len(fi), math.sqrt((err[t_out] ** 2).sum()) / len(fo)
    ratio = f_in / f_out
    return (1 - ratio, ratio * math.sqrt((s_in / f_in) ** 2 + (s_out / f_out) ** 2), ratio * math.sqrt((p_in / f_in) ** 2 + (p_out / f_out) ** 2), f_in, f_out)


def cmd_quicklook(root, outdir):
    name = os.path.basename(L3["E1"])
    infos, series = open_set([os.path.join(root, L3["E1"])])
    s = series[("NRS1", 0)]
    edges = [0.6, 1.0, 2.0, 3.0, 4.0, 5.3]
    pairs = list(zip(edges[:-1], edges[1:]))
    n = s["int_num"]
    t_in = (n >= 183) & (n <= 188)
    t_out = ((n >= 1) & (n <= 170)) | ((n >= 201) & (n <= 240))

    def nn(a):
        return [None if not np.isfinite(v) else float(v) for v in a]
    for mode in ("pipeline", "masked"):
        white = pipeline(s, *OPEN) if mode == "pipeline" else masked(s, *OPEN)
        bins = [pipeline(s, lo, hi, bin_mode=True) if mode == "pipeline" else masked(s, lo, hi, bin_mode=True) for lo, hi in pairs]
        kept = white["int_num"]
        k_in = (kept >= 183) & (kept <= 188)
        k_out = ((kept >= 1) & (kept <= 170)) | ((kept >= 201) & (kept <= 240))
        if mode == "masked":
            print(mode, "integrations left out for their valid count (INT_NUM)", white["left_out"], "rows kept", len(kept), "in the window", int(k_in.sum()))
        print(mode, "white D, sigma scatter, sigma formal, F_in, F_out", [repr(float(v)) for v in box_depth(white["flux"], white["err"], k_in, k_out)])
        for (lo, hi), b in zip(pairs, bins):
            print(mode, "bin", lo, hi, [repr(float(v)) for v in box_depth(b["flux"], b["err"], k_in, k_out)], "n_used", b["n_used"] if mode == "masked" else "per row")
        npx = white["npix"]
        print(mode, "npixels_mean: mean in window", repr(float(npx[k_in].mean())), "mean out", repr(float(npx[k_out].mean())), "ratio", repr(float(npx[k_in].mean() / npx[k_out].mean())),
              "median", repr(float(np.median(npx))), "integrations below 0.9 x median (INT_NUM)", [int(v) for v in kept[npx < 0.9 * np.median(npx)]])
        if mode == "pipeline" and outdir:
            head = {"file": name, "set": "tso/E1", "sha256": "a5e7022d22e779fdf43b14fba6e7d23644c9547c920b6a52c13435c63706bfbc",
                    "produced_by": "python -I scripts/oracle/tso_checks.py quicklook <data root>/tso <output dir> (astropy %s, numpy %s, exact sums with math.fsum)" % (astropy.__version__, np.__version__)}
            white_doc = dict(head, series={"detector": "NRS1", "sporder": 0}, mode="pipeline", limits_um=[float(np.nanmin(s["w"])), float(np.nanmax(s["w"]))], limits_origin="open",
                             time_scale="bmjd_tdb", flux_unit="Jy", int_num=[int(v) for v in n], time=nn(s["time"]), mjd_utc=nn(s["mjd"]), flux=nn(white["flux"]), flux_err=nn(white["err"]),
                             n_used=[int(v) for v in white["n_used"]], npixels_mean=nn(white["npix"]))
            docs = {"tso_quicklook_e1_white.json": white_doc,
                    "tso_quicklook_e1_bins_flux.json": dict(head, edges_um=edges, flux=[nn(b["flux"]) for b in bins]),
                    "tso_quicklook_e1_bins_err.json": dict(head, edges_um=edges, flux_err=[nn(b["err"]) for b in bins])}
            for fname, doc in docs.items():
                path = os.path.join(outdir, fname)
                with open(path, "w", newline="\n") as fh:
                    json.dump(doc, fh, separators=(",", ":"))
                print("wrote", path, os.path.getsize(path), "bytes")
    print("window times (TDB-MID): in", repr(float(s["time"][t_in][0])), repr(float(s["time"][t_in][-1])), "baseline 1", repr(float(s["time"][0])), repr(float(s["time"][169])), "baseline 2", repr(float(s["time"][200])), repr(float(s["time"][239])))


def cmd_sets(root):
    def rejected_text(items):
        return [(a, repr(b)) for a, b, *_ in items]
    segs = sorted(glob.glob(os.path.join(root, "C", "jw02327001001_03101_00001-seg00*_nrs1_x1dints.fits")))
    infos, series = open_set(list(reversed(segs)))
    for key in sorted(series):
        s = series[key]
        p = pipeline(s, *OPEN)
        print("== C six level 2b segments, given in reverse order", key, "rows kept", len(p["flux"]), "INT_NUM", int(p["int_num"][0]), "to", int(p["int_num"][-1]), "dropped (nan_time, repeated_time)", p["dropped"], "time[0]", repr(float(p["time"][0])))
        show("pipeline flux", p["flux"])
        for sigma in (1000.0, 100.0):
            m = masked(s, *OPEN, sigma=sigma)
            print("  masked + reject", sigma, ": rows", len(m["flux"]), "rejected (index, wavelength)", rejected_text(m["rejected"]), "n_used", m["n_used"])
            show("flux", m["flux"])
    a5 = [os.path.join(root, "A5", "jw03375001001_03101_00001-seg001_nrs1_x1dints.fits"), os.path.join(root, "A5", "jw03375001001_03101_00001-seg001_nrs2_x1dints.fits")]
    infos, series = open_set(a5)
    for key in sorted(series):
        s = series[key]
        p = pipeline(s, *OPEN)
        print("== A5 level 2b pair", key, "rows kept", len(p["flux"]), "dropped (nan_time, repeated_time)", p["dropped"], "time[0]", repr(float(p["time"][0])))
        show("pipeline flux", p["flux"])
        m = masked(s, *OPEN, sigma=100.0)
        print("  masked + reject 100.0 : rows", len(m["flux"]), "rejected (index, wavelength)", rejected_text(m["rejected"]), "n_used", m["n_used"])
        show("flux", m["flux"])
    infos, series = open_set([os.path.join(root, L3["E1"])])
    s = series[("NRS1", 0)]
    edges = [0.6, 1.0, 2.0, 3.0, 4.0, 5.3]
    for lo, hi in zip(edges[:-1], edges[1:]):
        m = masked(s, lo, hi, bin_mode=True)
        at_186 = int(np.nonzero(m["int_num"] == 187)[0][0])
        print("== E1 masked bin", lo, hi, "rows", len(m["flux"]), "left out (INT_NUM)", m["left_out"], "n_used", m["n_used"], "flux[0]", repr(float(m["flux"][0])), "flux at INT_NUM 187 (position %d)" % at_186, repr(float(m["flux"][at_186])), "fsum", repr(math.fsum(m["flux"])))
    p = pipeline(s, 1.0, 5.0)
    print("== E1 pipeline, limits 1.0 to 5.0: n_used", int(p["n_used"].min()), "to", int(p["n_used"].max()), "flux[0]", repr(float(p["flux"][0])), "fsum", repr(math.fsum(p["flux"])))
    m = masked(s, 1.0, 5.0, exclude=[(2.5, 3.0)])
    print("== E1 masked, limits 1.0 to 5.0, exclusion 2.5 to 3.0: rows", len(m["flux"]), "left out (INT_NUM)", m["left_out"], "valid in every row kept", m["n_valid_in_every_row"], "n_used", m["n_used"], "flux[0]", repr(float(m["flux"][0])), "fsum", repr(math.fsum(m["flux"])))


def main():
    if len(sys.argv) < 3:
        print("usage: tso_checks.py curves|pick|whtlt|quicklook|sets <data root>/tso [reject sigma | output dir]")
        return
    what, root = sys.argv[1], sys.argv[2]
    if what == "curves":
        cmd_curves(root, float(sys.argv[3]) if len(sys.argv) > 3 else 1000.0)
    elif what == "pick":
        cmd_pick(root)
    elif what == "whtlt":
        cmd_whtlt(root)
    elif what == "quicklook":
        cmd_quicklook(root, sys.argv[3] if len(sys.argv) > 3 else None)
    elif what == "sets":
        cmd_sets(root)


main()
