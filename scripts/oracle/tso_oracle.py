import os, math
import numpy as np
from astropy.io import fits

OPEN = (-1.0, 1.0e10)
NEIGHBOURS = 10
MIN_NEIGHBOURS = 4
MAD_TO_SIGMA = 1.4826
VALID_COUNT_RATIO = 0.9


def card(header, key):
    value = header.get(key)
    return value.strip() if isinstance(value, str) else value


def read_file(path):
    rows = {}
    with fits.open(path, memmap=False) as hdul:
        prim = hdul[0].header
        info = {k: card(prim, k) for k in ("DATAMODL", "EXP_TYPE", "FILTER", "DETECTOR", "NINTS", "EXPSTART", "S_OUTLIR", "ASNTABLE", "EXSEGNUM")}
        info["name"] = os.path.basename(path)
        info["level"] = 3 if ("S_OUTLIR" in prim or "tso3" in str(prim.get("ASNTABLE", ""))) else 2
        for hdu in hdul:
            if hdu.name != "EXTRACT1D":
                continue
            h = hdu.header
            det = card(h, "DETECTOR") or card(prim, "DETECTOR")
            key = (det, h.get("SPORDER"))
            d = hdu.data
            if "INT_NUM" in d.names and "N_ALONGDISP" in d.names:
                n = len(d)
                block = dict(
                    int_num=np.asarray(d["INT_NUM"], "i8"), segment=np.asarray(d["SEGMENT"], "i8"),
                    nal=np.asarray(d["N_ALONGDISP"]).astype("i8"),
                    mjd=np.asarray(d["MJD-AVG"], "f8"), tdb=np.asarray(d["TDB-MID"], "f8"),
                    w=np.asarray(d["WAVELENGTH"], "f8").reshape(n, -1), f=np.asarray(d["FLUX"], "f8").reshape(n, -1),
                    e=np.asarray(d["FLUX_ERROR"], "f8").reshape(n, -1), q=np.asarray(d["DQ"]).astype("u8").reshape(n, -1),
                    p=np.asarray(d["NPIXELS"], "f8").reshape(n, -1))
            else:
                m = len(d)
                block = dict(
                    int_num=np.array([h["INT_NUM"]], "i8"), segment=np.array([prim.get("EXSEGNUM", 1)], "i8"), nal=np.array([m], "i8"),
                    mjd=np.array([h.get("MJD-AVG", np.nan)], "f8"), tdb=np.array([h.get("TDB-MID", np.nan)], "f8"),
                    w=np.asarray(d["WAVELENGTH"], "f8")[None, :], f=np.asarray(d["FLUX"], "f8")[None, :],
                    e=np.asarray(d["FLUX_ERROR"], "f8")[None, :], q=np.asarray(d["DQ"]).astype("u8")[None, :],
                    p=np.asarray(d["NPIXELS"], "f8")[None, :])
            for k, v in block.items():
                rows.setdefault(key, {}).setdefault(k, []).append(v)
    return info, {key: {k: np.concatenate(v) for k, v in cols.items()} for key, cols in rows.items()}


def open_set(paths):
    infos, merged = [], {}
    for path in paths:
        info, series = read_file(path)
        infos.append(info)
        for key, cols in series.items():
            for k, v in cols.items():
                merged.setdefault(key, {}).setdefault(k, []).append(v)
    out = {}
    for key, cols in merged.items():
        s = {k: np.concatenate(v) for k, v in cols.items()}
        idx = np.arange(s["w"].shape[1])[None, :]
        pad = idx >= s["nal"][:, None]
        for k in ("w", "f", "e"):
            s[k] = np.where(pad, np.nan, s[k])
        s["q"] = np.where(pad, 1, s["q"])
        s["p"] = np.where(pad, 0.0, s["p"])

        def usable(t):
            return bool(np.all(np.isfinite(t)) and len(np.unique(t)) == len(t))
        if usable(s["tdb"]):
            scale, order = "bmjd_tdb", np.argsort(s["tdb"], kind="stable")
        elif usable(s["mjd"]):
            scale, order = "mjd_utc", np.argsort(s["mjd"], kind="stable")
        else:
            scale, order = "index", np.lexsort((s["int_num"], s["segment"]))
        s = {k: v[order] for k, v in s.items()}
        s["scale"] = scale
        s["time"] = s["tdb"] if scale == "bmjd_tdb" else s["mjd"] if scale == "mjd_utc" else s["int_num"].astype("f8")
        s["valid"] = np.isfinite(s["w"]) & np.isfinite(s["f"]) & ((s["q"] & 1) == 0)
        s["common_grid"] = bool(np.nanmax(np.abs(s["w"] - s["w"][0][None, :])) <= 1e-9) if np.isfinite(s["w"]).any() else False
        out[key] = s
    return infos, out


def fsum_rows(values, mask):
    return np.array([math.fsum(values[t][mask[t]]) for t in range(values.shape[0])])


def error_of_the_sums(errors, used):
    usable = np.isfinite(errors) & (errors > 0)
    without_error = (used & ~usable).sum(axis=1)
    return np.where(without_error > 0, np.nan, np.sqrt(fsum_rows(errors * errors, used & usable)))


def pipeline(s, lo, hi, bin_mode=False):
    mjd = s["mjd"]
    keep = np.zeros(len(mjd), bool)
    seen = set()
    nan_time = repeated = 0
    for t, v in enumerate(mjd):
        if not np.isfinite(v):
            nan_time += 1
        elif v in seen:
            repeated += 1
        else:
            seen.add(v)
            keep[t] = True
    w, f, e, p = s["w"][keep], s["f"][keep], s["e"][keep], s["p"][keep]
    inside = (w >= lo) & ((w < hi) if bin_mode else (w <= hi))
    used = inside & ~np.isnan(f)
    flux = fsum_rows(f, used)
    err = error_of_the_sums(e, used)
    npx = np.array([np.mean(p[t][used[t]]) if used[t].any() else np.nan for t in range(len(flux))])
    return dict(keep=keep, flux=flux, err=err, n_used=used.sum(axis=1), npix=npx, dropped=(nan_time, repeated), time=s["time"][keep], mjd=mjd[keep], int_num=s["int_num"][keep])


def reject(s, base, sigma, keep):
    idx = np.nonzero(base)[0]
    m = np.median(s["f"][keep][:, idx], axis=0)
    out = []
    for pos in range(len(idx)):
        lo = max(0, pos - NEIGHBOURS)
        hi = min(len(idx), pos + NEIGHBOURS + 1)
        nb = np.concatenate([m[lo:pos], m[pos + 1:hi]])
        if len(nb) < MIN_NEIGHBOURS:
            continue
        med = np.median(nb)
        scale = MAD_TO_SIGMA * np.median(np.abs(nb - med))
        z = abs(m[pos] - med) / scale if scale > 0 else (math.inf if m[pos] != med else 0.0)
        if z > sigma:
            out.append((int(idx[pos]), float(s["w"][0][idx[pos]]), float(m[pos]), float(med), float(z), len(nb)))
    return out


def rows_of_the_masked_mode(s):
    count = s["valid"].sum(axis=1)
    return count >= VALID_COUNT_RATIO * np.median(count)


def masked(s, lo, hi, exclude=(), sigma=None, bin_mode=False):
    assert s["common_grid"]
    w0 = s["w"][0]
    keep = rows_of_the_masked_mode(s)
    base = s["valid"][keep].all(axis=0)
    for a, b in exclude:
        base &= ~((w0 >= a) & (w0 <= b))
    rejected = reject(s, base, sigma, keep) if sigma is not None else []
    for r in rejected:
        base[r[0]] = False
    inside = (w0 >= lo) & ((w0 < hi) if bin_mode else (w0 <= hi))
    cand = base & inside
    f, e, p = s["f"][keep][:, cand], s["e"][keep][:, cand], s["p"][keep][:, cand]
    flux = np.array([math.fsum(row) for row in f])
    err = error_of_the_sums(e, np.ones(e.shape, bool))
    npx = p.mean(axis=1) if cand.any() else np.full(len(flux), np.nan)
    return dict(keep=keep, flux=flux, err=err, n_used=int(cand.sum()), n_valid_in_every_row=int(s["valid"][keep].all(axis=0).sum()), npix=npx, rejected=rejected,
                time=s["time"][keep], int_num=s["int_num"][keep], left_out=[int(v) for v in s["int_num"][~keep]])


def show(label, arr, k=3):
    a = [repr(float(v)) for v in arr[:k]]
    b = [repr(float(v)) for v in arr[-k:]]
    print("   ", label, "first", a, "last", b, "fsum", repr(math.fsum(arr)), "n", len(arr))


def defaults(info, key, mode):
    exp, flt = info["EXP_TYPE"], info["FILTER"]
    if exp == "NIS_SOSS" and flt == "CLEAR":
        if mode == "pipeline" or key[1] == 1:
            return (0.93, 2.82), "built-in default"
        return {2: (0.64, 0.83), 3: (0.64, 0.95)}[key[1]], "built-in default"
    if exp == "MIR_LRS-SLITLESS":
        return (5.0, 12.0), "file log when the file records it, else built-in default"
    if exp == "NRC_TSGRISM" and flt == "F322W2":
        return (2.47, 3.94), "file log when the file records it, else built-in default"
    return OPEN, "open"
