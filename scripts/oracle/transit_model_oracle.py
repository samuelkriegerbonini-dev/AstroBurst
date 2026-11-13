import json
import math
import os
import sys

import numpy as np

NODES, WEIGHTS = np.polynomial.legendre.leggauss(600)
THETA = 0.5 * (NODES + 1.0) * math.pi
HALF_PI = 0.5 * math.pi


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


def main(out):
    radii = [0.01, 0.05, 0.1, 0.15, 0.3, 0.5, 0.8, 1.0, 1.5, 7.28]
    fractions = [0.0, 0.05, 0.2, 0.35, 0.5, 0.65, 0.8, 0.95, 0.999]
    limb = [[0.0, 0.0], [0.4, 0.25], [0.1, 0.5], [0.9, -0.2], [0.6, 0.0]]
    cases = []
    for p in radii:
        lo, hi = max(0.0, p - 1.0), 1.0 + p
        zs = [lo + (hi - lo) * f for f in fractions] + [p, abs(1.0 - p)]
        for z in zs:
            for u1, u2 in limb:
                cases.append([p, z, u1, u2, 1.0 - hidden_fraction(p, z, u1, u2)])
    doc = {
        "source": "scripts/oracle/transit_model_oracle.py: Gauss-Legendre integration (600 nodes) of the limb-darkened disc hidden by the planet",
        "columns": ["p", "z", "u1", "u2", "flux"],
        "cases": cases,
    }
    with open(out, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(doc, fh, separators=(",", ":"))
    print("cases", len(cases), "bytes", os.path.getsize(out))


if __name__ == "__main__":
    main(sys.argv[1])
