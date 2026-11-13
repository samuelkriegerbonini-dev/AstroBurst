import json
import math
import os
import sys

import numpy as np
from scipy import optimize

NODES, WEIGHTS = np.polynomial.legendre.leggauss(600)
THETA = 0.5 * (NODES + 1.0) * math.pi
HALF_PI = 0.5 * math.pi
NAMES = ["t0", "rp", "a_rs", "b", "u1", "u2"]
PERIOD = 3.0
TRUTH = {"t0": 2460310.5, "rp": 0.1, "a_rs": 9.0, "b": 0.3, "u1": 0.4, "u2": 0.25}
GRAVITY = 6.6743e-11


def hidden_fraction(p, z, u1, u2):
    if z >= 1.0 + p:
        return 0.0

    def intensity(r):
        mu = np.sqrt(np.maximum(0.0, 1.0 - r * r))
        return 1.0 - u1 * (1.0 - mu) - u2 * (1.0 - mu) ** 2

    total = 0.0
    full = min(1.0, p - z) if p > z else 0.0
    if full > 0.0:
        r = full * (1.0 - np.cos(THETA)) / 2.0
        dr = full * np.sin(THETA) / 2.0 * HALF_PI
        total += float(np.sum(WEIGHTS * intensity(r) * 2.0 * r * math.pi * dr))
    lo, hi = abs(z - p), min(1.0, z + p)
    if hi > lo and z > 0.0:
        r = lo + (hi - lo) * (1.0 - np.cos(THETA)) / 2.0
        dr = (hi - lo) * np.sin(THETA) / 2.0 * HALF_PI
        half_angle = np.arccos(np.clip((r * r + z * z - p * p) / (2.0 * r * z), -1.0, 1.0))
        total += float(np.sum(WEIGHTS * intensity(r) * 2.0 * r * half_angle * dr))
    return total / (math.pi * (1.0 - u1 / 3.0 - u2 / 6.0))


def shape(time, s):
    x = (time - s["t0"]) / PERIOD
    x -= round(x)
    phase = 2.0 * math.pi * x
    if math.cos(phase) <= 0.0:
        return 1.0
    z = math.hypot(s["a_rs"] * math.sin(phase), s["b"] * math.cos(phase))
    return 1.0 - hidden_fraction(s["rp"], z, s["u1"], s["u2"])


def series_a():
    u, x = [], 12345
    for _ in range(12 * 240):
        x = (1103515245 * x + 12345) % 2147483648
        u.append(x / 2147483648.0)
    time, flux, err = [], [], []
    for i in range(240):
        t = 2460310.38 + 0.001 * i
        noise = 0.3 * (sum(u[12 * i:12 * i + 12]) - 6.0)
        time.append(t)
        flux.append(1000.0 * (1.0 + 0.02 * (t - 2460310.5)) * shape(t, TRUTH) + noise)
        err.append(0.3)
    return time, flux, err


def durations(s):
    rp, a, b = s["rp"], s["a_rs"], s["b"]
    t14 = PERIOD / math.pi * math.asin(math.sqrt(((1.0 + rp) ** 2 - b * b) / (a * a - b * b)))
    t23 = PERIOD / math.pi * math.asin(math.sqrt(((1.0 - rp) ** 2 - b * b) / (a * a - b * b))) if b < 1.0 - rp else None
    return t14, t23


def derived(s):
    t14, t23 = durations(s)
    return {
        "inclination_deg": math.degrees(math.acos(s["b"] / s["a_rs"])),
        "duration_t14_days": t14,
        "duration_t23_days": t23,
        "stellar_density_cgs": 3.0 * math.pi * s["a_rs"] ** 3 / (GRAVITY * (PERIOD * 86400.0) ** 2) * 1.0e-3,
    }


def time_averaging(time, relative):
    order = np.argsort(np.array(time), kind="stable")
    r = np.array(relative)[order]
    n = len(r)
    sigma1 = math.sqrt(float(np.sum(r * r)) / n)
    out = {"cadence_days": float(np.median(np.diff(np.array(time)[order]))), "bin_points": [], "n_bins": [], "rms_ppm": [], "expected_ppm": []}
    size, beta = 1, 1.0
    while n // size >= 4:
        m = n // size
        means = r[:m * size].reshape(m, size).mean(axis=1)
        rms = math.sqrt(float(np.sum(means * means)) / m)
        expected = sigma1 / math.sqrt(size) * math.sqrt(m / (m - 1.0))
        out["bin_points"].append(size)
        out["n_bins"].append(m)
        out["rms_ppm"].append(1.0e6 * rms)
        out["expected_ppm"].append(1.0e6 * expected)
        if size > 1:
            beta = max(beta, rms / expected)
        size *= 2
    out["beta"] = beta
    return out


def fit(time, flux, err, start, free, order, scale_errors):
    t_ref = 0.5 * (min(time) + max(time))
    scale = float(np.median(flux))
    xs = np.array(time) - t_ref
    y = np.array(flux) / scale
    sigma = np.array(err) / scale if err is not None else np.ones(len(time))
    names = [n for n in NAMES if n in free]
    k = len(names)

    def unpack(p):
        s = dict(start)
        for name, value in zip(names, p[:k]):
            s[name] = value + (t_ref if name == "t0" else 0.0)
        return s, p[k:]

    def model(p):
        s, c = unpack(p)
        return np.array([sum(ck * x ** j for j, ck in enumerate(c)) * shape(t, s) for x, t in zip(xs, time)])

    def residual(p):
        return (y - model(p)) / sigma

    x0 = [start[n] - (t_ref if n == "t0" else 0.0) for n in names] + [1.0] + [0.0] * order
    sol = optimize.least_squares(residual, x0, method="trf", jac="3-point", x_scale="jac", xtol=1e-15, ftol=1e-15, gtol=1e-15, max_nfev=2000)
    best, coeff = unpack(sol.x)
    chi2 = float(np.sum(sol.fun ** 2))
    dof = len(time) - len(sol.x)
    weighted = err is not None
    factor = math.sqrt(chi2 / dof) if (not weighted or scale_errors) else 1.0
    cov = np.linalg.inv(sol.jac.T @ sol.jac) * factor * factor
    sd = np.sqrt(np.diag(cov))
    base = np.array([sum(ck * x ** j for j, ck in enumerate(coeff)) for x in xs]) * scale
    full = model(sol.x) * scale
    resid = np.array(flux) - full
    grad_steps = {"rp": 1.0e-6, "a_rs": 1.0e-5, "b": 1.0e-6}
    values = derived(best)
    derived_out = {}
    for key, value in values.items():
        if value is None:
            derived_out[key] = [None, None]
            continue
        g = np.zeros(len(sol.x))
        for i, name in enumerate(names):
            if name not in grad_steps:
                continue
            up, dn = dict(best), dict(best)
            up[name] += grad_steps[name]
            dn[name] -= grad_steps[name]
            g[i] = (derived(up)[key] - derived(dn)[key]) / (2.0 * grad_steps[name])
        derived_out[key] = [value, math.sqrt(float(g @ cov @ g))]
    return {
        "free": names,
        "start": start,
        "baseline_order": order,
        "weighted": weighted,
        "scale_errors": scale_errors,
        "values": best,
        "errors": {n: float(sd[i]) for i, n in enumerate(names)},
        "reference_time": t_ref,
        "baseline": [float(c) * scale for c in coeff],
        "baseline_errors": [float(sd[k + j]) * scale for j in range(order + 1)],
        "chi2": chi2,
        "dof": dof,
        "error_scale": factor,
        "depth": best["rp"] ** 2,
        "depth_err": 2.0 * best["rp"] * float(sd[names.index("rp")]) if "rp" in names else None,
        "rms": math.sqrt(float(np.mean(resid ** 2))),
        "rms_ppm": 1.0e6 * math.sqrt(float(np.mean((resid / base) ** 2))),
        "n_in_transit": int(sum(1 for t in time if shape(t, best) < 1.0)),
        "free_names": names + ["c%d" % j for j in range(order + 1)],
        "correlation": (cov / np.outer(sd, sd)).tolist(),
        "derived": derived_out,
        "time_averaging": time_averaging(time, resid / base),
    }


def main(out):
    time, flux, err = series_a()
    start = {"t0": 2460310.502, "rp": 0.09, "a_rs": 8.0, "b": 0.4, "u1": 0.4, "u2": 0.25}
    bin_start = dict(TRUTH, rp=0.08)
    doc = {
        "source": "scripts/oracle/transit_fit_oracle.py: series A built with a numerical integration of the limb-darkened disc; fits by scipy.optimize.least_squares on the same integration",
        "period": PERIOD,
        "truth": TRUTH,
        "time": time,
        "flux": flux,
        "err": err,
        "fits": {
            "weighted": fit(time, flux, err, start, ["t0", "rp", "a_rs", "b"], 1, False),
            "unweighted": fit(time, flux, None, start, ["t0", "rp", "a_rs", "b"], 1, False),
            "radius_only": fit(time, flux, err, bin_start, ["rp"], 1, False),
            "all_free": fit(time, flux, err, start, NAMES, 1, False),
            "quadratic_scaled": fit(time, flux, err, start, ["t0", "rp", "a_rs", "b"], 2, True),
        },
    }
    with open(out, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(doc, fh, separators=(",", ":"))
    print("points", len(time), "bytes", os.path.getsize(out))


if __name__ == "__main__":
    main(sys.argv[1])
