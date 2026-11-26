import sys, os, math, json
import numpy as np
from scipy import integrate, optimize

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

E1_FILE = "E1/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits"
E1_EDGES = [0.6, 1.0, 2.0, 3.0, 4.0, 5.3]
E1_PERIOD = 1.4079405
E1_A_RS = 336.0
E1_B = 7.16
E1_U1 = 0.2
E1_U2 = 0.25
E1_T0 = 60061.507668
E1_RP_START = 7.73
E1_WHITE_RP_START = 7.9


def occulted_fraction(p, z, u1, u2):
    if z >= 1.0 + p:
        return 0.0
    if p >= 1.0 + z:
        return 1.0
    total = math.pi * (1.0 - u1 / 3.0 - u2 / 6.0)

    def integrand(psi):
        r = math.sin(psi)
        mu = math.cos(psi)
        if r <= p - z:
            theta = math.pi
        elif r >= z + p or r <= z - p:
            theta = 0.0
        else:
            c = (r * r + z * z - p * p) / (2.0 * r * z)
            theta = math.acos(max(-1.0, min(1.0, c)))
        limb = 1.0 - u1 * (1.0 - mu) - u2 * (1.0 - mu) ** 2
        return limb * 2.0 * r * theta * mu

    points = sorted({math.asin(x) for x in (abs(z - p), z + p) if 0.0 < x < 1.0})
    value, _ = integrate.quad(integrand, 0.0, math.pi / 2.0, points=points or None, epsabs=1e-15, epsrel=1e-13, limit=500)
    return value / total


def separation(t, period, t0, a_rs, b):
    phi = 2.0 * math.pi * (np.asarray(t, "f8") - t0) / period
    z = a_rs * np.sqrt(np.sin(phi) ** 2 + (b / a_rs) ** 2 * np.cos(phi) ** 2)
    return z, np.cos(phi) > 0.0


def transit_factor(t, period, t0, a_rs, b, rp, u1, u2):
    z, front = separation(t, period, t0, a_rs, b)
    out = np.ones(len(z))
    for i in range(len(z)):
        if front[i] and z[i] < 1.0 + rp:
            out[i] = 1.0 - occulted_fraction(rp, float(z[i]), u1, u2)
    return out


def uniform_closed_form(p, z):
    if z >= 1.0 + p:
        return 0.0
    if z <= abs(1.0 - p):
        return p * p if p < 1.0 else 1.0
    k1 = math.acos((1.0 - p * p + z * z) / (2.0 * z))
    k0 = math.acos((p * p + z * z - 1.0) / (2.0 * p * z))
    return (p * p * k0 + k1 - math.sqrt((4.0 * z * z - (1.0 + z * z - p * p) ** 2) / 4.0)) / math.pi


def fit(t, y, e, period, t0, a_rs, b, u1, u2, rp_start, order, free_t0=False, free_limb=False):
    t = np.asarray(t, "f8"); y = np.asarray(y, "f8")
    weighted = e is not None
    sigma = np.asarray(e, "f8") if weighted else np.ones(len(y))
    names = ["rp"] + (["t0"] if free_t0 else []) + (["u1", "u2"] if free_limb else []) + ["c%d" % k for k in range(order + 1)]
    z0, front0 = separation(t, period, t0, a_rs, b)
    outside = ~(front0 & (z0 < 1.0 + rp_start))
    level = float(np.median(y[outside])) if outside.any() else float(np.median(y))
    start = [rp_start] + ([0.0] if free_t0 else []) + ([u1, u2] if free_limb else []) + [level] + [0.0] * order

    def unpack(theta):
        k = 0
        rp = theta[k]; k += 1
        dt = 0.0
        if free_t0:
            dt = theta[k]; k += 1
        a, c = u1, u2
        if free_limb:
            a, c = theta[k], theta[k + 1]; k += 2
        return rp, dt, a, c, theta[k:]

    def model(theta):
        rp, dt, a, c, coeff = unpack(theta)
        tau = t - t0
        base = np.zeros(len(t))
        for k, ck in enumerate(coeff):
            base = base + ck * tau ** k
        return base * transit_factor(t, period, t0 + dt, a_rs, b, abs(rp), a, c)

    def residual(theta):
        return (y - model(theta)) / sigma

    sol = optimize.least_squares(residual, start, method="lm", xtol=1e-14, ftol=1e-14, gtol=1e-14, x_scale="jac", max_nfev=4000)
    theta = sol.x.copy()
    steps = np.maximum(np.abs(theta) * 1e-6, 1e-9)
    jac = np.zeros((len(y), len(theta)))
    for k in range(len(theta)):
        up = theta.copy(); up[k] += steps[k]
        dn = theta.copy(); dn[k] -= steps[k]
        jac[:, k] = (residual(up) - residual(dn)) / (2.0 * steps[k])
    cov = np.linalg.inv(jac.T @ jac)
    res = residual(theta)
    chi2 = float(res @ res)
    dof = len(y) - len(theta)
    formal = np.sqrt(np.diag(cov))
    scale = math.sqrt(chi2 / dof) if dof > 0 else float("nan")
    rp = abs(float(theta[0]))
    out = dict(names=names, values=[float(v) for v in theta], formal=[float(v) for v in formal], scaled=[float(v * scale) for v in formal],
               rp=rp, rp_err_formal=float(formal[0]), rp_err_scaled=float(formal[0] * scale), depth=rp * rp,
               depth_err_formal=2.0 * rp * float(formal[0]), depth_err_scaled=2.0 * rp * float(formal[0] * scale),
               chi2=chi2, dof=dof, n=len(y), rms=float(np.sqrt(np.mean((y - model(theta)) ** 2))), weighted=weighted, nfev=int(sol.nfev), status=int(sol.status))
    if free_t0:
        out["t0"] = t0 + float(theta[1])
    return out


def show_fit(label, f):
    print(label, "rp", repr(f["rp"]), "+-", repr(f["rp_err_formal"]), "(formal)", repr(f["rp_err_scaled"]), "(scaled by sqrt(chi2/dof))")
    print("    depth rp^2", repr(f["depth"]), "+-", repr(f["depth_err_formal"]), "(formal)", repr(f["depth_err_scaled"]), "(scaled)",
          "| chi2", repr(f["chi2"]), "dof", f["dof"], "n", f["n"], "rms", repr(f["rms"]), "| nfev", f["nfev"], "status", f["status"])
    print("    parameters", dict(zip(f["names"], [repr(v) for v in f["values"]])))


def cmd_selfcheck():
    worst = 0.0
    for p, z in [(0.1, 0.0), (0.1, 0.5), (0.1, 0.95), (0.1, 1.05), (0.5, 0.5), (0.5, 0.2), (0.7, 0.5), (1.0, 0.5), (1.5, 1.0), (7.28, 7.16), (7.28, 6.5), (7.28, 8.2), (8.0, 7.16)]:
        a = occulted_fraction(p, z, 0.0, 0.0)
        c = uniform_closed_form(p, z)
        worst = max(worst, abs(a - c))
        print("uniform p", p, "z", z, "numeric", repr(a), "closed form", repr(c), "difference", "%.3e" % (a - c))
    for p, u1 in [(0.1, 0.4), (0.3, 0.6), (0.05, 1.0)]:
        a = occulted_fraction(p, 0.0, u1, 0.0)
        c = (p * p * (1.0 - u1) + u1 * (2.0 / 3.0) * (1.0 - (1.0 - p * p) ** 1.5)) / (1.0 - u1 / 3.0)
        worst = max(worst, abs(a - c))
        print("linear limb darkening, centred: p", p, "u1", u1, "numeric", repr(a), "closed form", repr(c), "difference", "%.3e" % (a - c))
    for p, u2 in [(0.1, 0.3), (0.3, 0.5)]:
        a = occulted_fraction(p, 0.0, 0.0, u2)
        s = 1.0 - p * p
        inner = p * p - u2 * (p * p - (4.0 / 3.0) * (1.0 - s ** 1.5) + (1.0 - s * s) / 2.0)
        c = inner / (1.0 - u2 / 6.0)
        worst = max(worst, abs(a - c))
        print("quadratic term only, centred: p", p, "u2", u2, "numeric", repr(a), "closed form", repr(c), "difference", "%.3e" % (a - c))
    print("largest difference", "%.3e" % worst)
    z, front = separation([60000.0, 60000.75, 60001.5], 3.0, 60000.0, 8.0, 0.3)
    print("separation at t0, a quarter period later and half a period later (a/Rs 8, b 0.3):", [repr(float(v)) for v in z], [bool(v) for v in front])


def e1_curves(root):
    from tso_oracle import open_set, pipeline, OPEN
    infos, series = open_set([os.path.join(root, E1_FILE)])
    s = series[("NRS1", 0)]
    white = pipeline(s, *OPEN)
    bins = [pipeline(s, lo, hi, bin_mode=True) for lo, hi in zip(E1_EDGES[:-1], E1_EDGES[1:])]
    return s, white, bins


def normalised(curve, keep=None):
    flux, err, n_used = curve["flux"], curve["err"], curve["n_used"]
    ok = np.isfinite(flux) & (n_used > 0)
    if keep is not None:
        ok &= keep
    med = float(np.median(flux[ok]))
    weighted = bool(np.all(np.isfinite(err[ok]) & (err[ok] > 0)))
    return ok, med, flux[ok] / med, (err[ok] / med if weighted else None)


def cmd_e1(root):
    s, white, bins = e1_curves(root)
    t = white["time"]
    print("E1 series NRS1 order 0, pipeline mode:", len(t), "integrations, time", repr(float(t[0])), "to", repr(float(t[-1])), s["scale"])
    print("fixed for every fit: period", E1_PERIOD, "a/Rs", E1_A_RS, "b", E1_B, "u1", E1_U1, "u2", E1_U2, "(geometry recalled from Vanderburg et al. 2020; limb darkening typed for the test)")
    ok, med, y, e = normalised(white)
    guess = float(t[ok][np.argmin(y)])
    w = fit(t[ok], y, e, E1_PERIOD, guess, E1_A_RS, E1_B, E1_U1, E1_U2, E1_WHITE_RP_START, 1, free_t0=True)
    show_fit("white light, rp, t0 and a linear baseline free (start t0 %r):" % guess, w)
    print("    white-light t0", repr(w["t0"]), "and rp", repr(w["rp"]), "-> hand-over, rounded: t0", repr(E1_T0), "rp start", E1_RP_START)
    wf = fit(t[ok], y, e, E1_PERIOD, E1_T0, E1_A_RS, E1_B, E1_U1, E1_U2, E1_RP_START, 1)
    show_fit("white light with the hand-over t0 fixed, rp and a linear baseline free:", wf)
    z, front = separation(t, E1_PERIOD, E1_T0, E1_A_RS, E1_B)
    inside = front & (z < 1.0 + E1_RP_START)
    print("integrations inside the transit of the hand-over geometry (z < 1 + rp start):", int(inside.sum()), "INT_NUM", int(white["int_num"][inside][0]), "to", int(white["int_num"][inside][-1]),
          "| smallest separation", repr(float(z.min())), "at INT_NUM", int(white["int_num"][int(np.argmin(z))]))
    npx = white["npix"]
    print("white light mean NPIXELS inside", repr(float(np.mean(npx[inside]))), "outside", repr(float(np.mean(npx[~inside]))), "ratio", repr(float(np.mean(npx[inside]) / np.mean(npx[~inside]))))
    for (lo, hi), b in zip(zip(E1_EDGES[:-1], E1_EDGES[1:]), bins):
        ok, med, y, e = normalised(b)
        f = fit(t[ok], y, e, E1_PERIOD, E1_T0, E1_A_RS, E1_B, E1_U1, E1_U2, E1_RP_START, 1)
        npb = b["npix"]
        good = np.isfinite(npb) & ok
        ratio = float(np.mean(npb[inside & good]) / np.mean(npb[~inside & good]))
        show_fit("bin %s to %s um, linear baseline, median %r, %d points, %d inside the transit, weighted %s, NPIXELS ratio %r:" % (lo, hi, med, int(ok.sum()), int((inside & ok).sum()), f["weighted"], ratio), f)


INJ = dict(period=3.0, t0=60000.15, a_rs=8.0, b=0.3, u1=0.3, u2=0.2, rp=[0.095, 0.1, 0.108], n=120, step=0.0025, start=60000.0, slope=0.02, sigma=3.0e-4, seed=20261113, excluded=[0, 1, 60, 61, 119])


def cmd_inject(outdir):
    t = INJ["start"] + INJ["step"] * np.arange(INJ["n"])
    rng = np.random.default_rng(INJ["seed"])
    transit, noise, fits_clean, fits_noisy, fits_free, fits_order0, fits_order2, fits_excluded = [], [], [], [], [], [], [], []
    keep = np.ones(len(t), bool)
    keep[INJ["excluded"]] = False
    base = 1.0 + INJ["slope"] * (t - INJ["t0"])
    for rp in INJ["rp"]:
        tr = transit_factor(t, INJ["period"], INJ["t0"], INJ["a_rs"], INJ["b"], rp, INJ["u1"], INJ["u2"])
        nz = rng.normal(0.0, INJ["sigma"], len(t))
        transit.append([float(v) for v in tr])
        noise.append([float(v) for v in nz])
        e = np.full(len(t), INJ["sigma"])
        for target, y in ((fits_clean, base * tr), (fits_noisy, base * tr + nz)):
            med = float(np.median(y))
            f = fit(t, y / med, e / med, INJ["period"], INJ["t0"], INJ["a_rs"], INJ["b"], INJ["u1"], INJ["u2"], 0.1, 1)
            target.append(f)
        y = base * tr + nz
        med = float(np.median(y))
        fits_free.append(fit(t, y / med, e / med, INJ["period"], INJ["t0"], INJ["a_rs"], INJ["b"], INJ["u1"], INJ["u2"], 0.1, 1, free_limb=True))
        fits_order0.append(fit(t, y / med, e / med, INJ["period"], INJ["t0"], INJ["a_rs"], INJ["b"], INJ["u1"], INJ["u2"], 0.1, 0))
        fits_order2.append(fit(t, y / med, e / med, INJ["period"], INJ["t0"], INJ["a_rs"], INJ["b"], INJ["u1"], INJ["u2"], 0.1, 2))
        medk = float(np.median(y[keep]))
        fits_excluded.append(fit(t[keep], y[keep] / medk, e[keep] / medk, INJ["period"], INJ["t0"], INJ["a_rs"], INJ["b"], INJ["u1"], INJ["u2"], 0.1, 1))
    for k, rp in enumerate(INJ["rp"]):
        show_fit("injected rp %r (depth %r), noise-free:" % (rp, rp * rp), fits_clean[k])
        show_fit("injected rp %r, with noise sigma %r:" % (rp, INJ["sigma"]), fits_noisy[k])
        show_fit("injected rp %r, with noise, u1 and u2 free:" % rp, fits_free[k])
        show_fit("injected rp %r, with noise, constant baseline:" % rp, fits_order0[k])
        show_fit("injected rp %r, with noise, quadratic baseline:" % rp, fits_order2[k])
        show_fit("injected rp %r, with noise, without the rows %s:" % (rp, INJ["excluded"]), fits_excluded[k])
    z, front = separation(t, INJ["period"], INJ["t0"], INJ["a_rs"], INJ["b"])
    inside = front & (z < 1.1)
    print("integrations inside the transit (z < 1 + 0.1):", int(inside.sum()), "of", len(t), "| without the excluded rows:", int((inside & keep).sum()), "of", int(keep.sum()))
    if outdir:
        keys = ("rp", "rp_err_formal", "rp_err_scaled", "depth", "depth_err_formal", "depth_err_scaled", "chi2", "dof", "rms")
        doc = dict(produced_by="python -I scripts/oracle/tso_transit_oracle.py inject <output dir> (numpy %s, scipy quad and least_squares; model by numerical integration of the limb-darkened disk, independent of Mandel and Agol closed forms)" % np.__version__,
                   period_days=INJ["period"], t0=INJ["t0"], a_rs=INJ["a_rs"], b=INJ["b"], u1=INJ["u1"], u2=INJ["u2"], rp=INJ["rp"], baseline_slope_per_day=INJ["slope"], sigma=INJ["sigma"],
                   time=[float(v) for v in t], transit=transit, noise=noise,
                   fit_noisy=[{k: f[k] for k in keys} for f in fits_noisy],
                   fit_noisy_order0=[{k: f[k] for k in keys} for f in fits_order0],
                   fit_noisy_order2=[{k: f[k] for k in keys} for f in fits_order2],
                   excluded_rows=INJ["excluded"],
                   fit_noisy_excluded=[{k: f[k] for k in keys} for f in fits_excluded],
                   fit_noisy_free_limb=[dict({k: f[k] for k in keys}, u1=f["values"][1], u2=f["values"][2], u1_err_formal=f["formal"][1], u2_err_formal=f["formal"][2]) for f in fits_free])
        path = os.path.join(outdir, "tso_transit_injected.json")
        with open(path, "w", newline="\n") as fh:
            json.dump(doc, fh, separators=(",", ":"))
        print("wrote", path, os.path.getsize(path), "bytes")


def main():
    if len(sys.argv) < 2:
        print("usage: tso_transit_oracle.py selfcheck | e1 <data root>/tso | inject [output dir]")
        return
    what = sys.argv[1]
    if what == "selfcheck":
        cmd_selfcheck()
    elif what == "e1":
        cmd_e1(sys.argv[2])
    elif what == "inject":
        cmd_inject(sys.argv[2] if len(sys.argv) > 2 else None)


if __name__ == "__main__":
    main()
