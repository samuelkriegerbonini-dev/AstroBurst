import json
import math
import os
import sys

import numpy as np
from scipy import optimize

NODES, WEIGHTS = np.polynomial.legendre.leggauss(600)
THETA = 0.5 * (NODES + 1.0) * math.pi
HALF_PI = 0.5 * math.pi
PERIOD = 3.0
TRUTH = {"t0": 2460312.5, "rp": 0.1, "a_rs": 9.0, "b": 0.3, "flux_ratio": 0.002}


def overlap(p, z):
    if z >= 1.0 + p:
        return 0.0
    total = 0.0
    full = min(1.0, p - z) if p > z else 0.0
    if full > 0.0:
        r = full * (1.0 - np.cos(THETA)) / 2.0
        dr = full * np.sin(THETA) / 2.0 * HALF_PI
        total += float(np.sum(WEIGHTS * 2.0 * r * math.pi * dr))
    lo, hi = abs(z - p), min(1.0, z + p)
    if hi > lo and z > 0.0:
        r = lo + (hi - lo) * (1.0 - np.cos(THETA)) / 2.0
        dr = (hi - lo) * np.sin(THETA) / 2.0 * HALF_PI
        half_angle = np.arccos(np.clip((r * r + z * z - p * p) / (2.0 * r * z), -1.0, 1.0))
        total += float(np.sum(WEIGHTS * 2.0 * r * half_angle * dr))
    return total / math.pi


def shape(time, s):
    x = (time - s["t0"]) / PERIOD
    x -= round(x)
    phase = 2.0 * math.pi * x
    if math.cos(phase) <= 0.0:
        return 1.0
    z = math.hypot(s["a_rs"] * math.sin(phase), s["b"] * math.cos(phase))
    hidden = overlap(s["rp"], z) / (s["rp"] * s["rp"])
    return 1.0 - s["flux_ratio"] / (1.0 + s["flux_ratio"]) * hidden


def series_e():
    u, x = [], 12345
    for _ in range(12 * 240):
        x = (1103515245 * x + 12345) % 2147483648
        u.append(x / 2147483648.0)
    time, flux, err = [], [], []
    for i in range(240):
        t = 2460312.38 + 0.001 * i
        noise = 0.05 * (sum(u[12 * i:12 * i + 12]) - 6.0)
        time.append(t)
        flux.append(1000.0 * (1.0 - 0.01 * (t - 2460312.5)) * shape(t, TRUTH) + noise)
        err.append(0.05)
    return time, flux, err


def fit(time, flux, err, start, free, order):
    t_ref = 0.5 * (min(time) + max(time))
    scale = float(np.median(flux))
    xs = np.array(time) - t_ref
    y = np.array(flux) / scale
    sigma = np.array(err) / scale
    k = len(free)

    def unpack(p):
        s = dict(start)
        for name, value in zip(free, p[:k]):
            s[name] = value + (t_ref if name == "t0" else 0.0)
        return s, p[k:]

    def residual(p):
        s, c = unpack(p)
        model = np.array([sum(ck * x ** j for j, ck in enumerate(c)) * shape(t, s) for x, t in zip(xs, time)])
        return (y - model) / sigma

    x0 = [start[n] - (t_ref if n == "t0" else 0.0) for n in free] + [1.0] + [0.0] * order
    sol = optimize.least_squares(residual, x0, method="trf", jac="3-point", x_scale="jac", xtol=1e-15, ftol=1e-15, gtol=1e-15, max_nfev=2000)
    best, coeff = unpack(sol.x)
    sd = np.sqrt(np.diag(np.linalg.inv(sol.jac.T @ sol.jac)))
    ratio = best["flux_ratio"]
    ratio_err = float(sd[free.index("flux_ratio")])
    return {
        "free": free,
        "start": start,
        "baseline_order": order,
        "values": best,
        "errors": {n: float(sd[i]) for i, n in enumerate(free)},
        "reference_time": t_ref,
        "baseline": [float(c) * scale for c in coeff],
        "baseline_errors": [float(sd[k + j]) * scale for j in range(order + 1)],
        "chi2": float(np.sum(sol.fun ** 2)),
        "dof": len(time) - len(sol.x),
        "depth": ratio / (1.0 + ratio),
        "depth_err": ratio_err / (1.0 + ratio) ** 2,
    }


def main(out):
    time, flux, err = series_e()
    start = dict(TRUTH, flux_ratio=0.001)
    doc = {
        "source": "scripts/oracle/transit_eclipse_oracle.py: series E built with a numerical integration of the planet disc hidden by the star; fits by scipy.optimize.least_squares on the same integration",
        "period": PERIOD,
        "truth": TRUTH,
        "time": time,
        "flux": flux,
        "err": err,
        "fits": {
            "ratio_only": fit(time, flux, err, start, ["flux_ratio"], 1),
            "time_and_ratio": fit(time, flux, err, dict(start, t0=2460312.503), ["t0", "flux_ratio"], 1),
        },
    }
    with open(out, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(doc, fh, separators=(",", ":"))
    print("points", len(time), "bytes", os.path.getsize(out))


if __name__ == "__main__":
    main(sys.argv[1])
