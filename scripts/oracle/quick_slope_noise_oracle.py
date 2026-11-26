import json
import math
import os
import sys

import numpy as np

SEED = 20261125
RAMPS = 400_000
OUT = os.path.join(os.path.dirname(__file__), "..", "..", "src-tauri", "tests", "fixtures", "ramp", "quick_slope_noise_oracle.json")


def ramps(rng, ngroups, tgroup_s, rate_dn_s, read_dn, gain_e_dn):
    electrons = rate_dn_s * tgroup_s * gain_e_dn
    if electrons > 0.0:
        steps = rng.poisson(electrons, size=(RAMPS, ngroups)).astype(np.float64) / gain_e_dn
    else:
        steps = np.zeros((RAMPS, ngroups))
    steps[:, 0] = 0.0
    return np.cumsum(steps, axis=1) + rng.normal(0.0, read_dn, size=(RAMPS, ngroups))


def ols_weights(ngroups, tgroup_s):
    times = np.arange(ngroups) * tgroup_s
    centred = times - times.mean()
    return centred / np.sum(centred * centred)


def exact_ols_std(ngroups, tgroup_s, rate_dn_s, read_dn, gain_e_dn):
    w = ols_weights(ngroups, tgroup_s)
    tails = np.array([w[k:].sum() for k in range(1, ngroups)])
    return math.sqrt(read_dn ** 2 * np.sum(w * w) + (rate_dn_s * tgroup_s / gain_e_dn) * np.sum(tails * tails))


def closed_form_ols_std(n, t, rate, read, gain):
    return math.sqrt(12.0 * read ** 2 / (n * (n * n - 1.0) * t * t) + 6.0 * (n * n + 1.0) * rate / (5.0 * n * (n * n - 1.0) * t * gain))


def ols_case(rng, name, ngroups, tgroup_s, rate_dn_s, read_dn, gain_e_dn):
    data = ramps(rng, ngroups, tgroup_s, rate_dn_s, read_dn, gain_e_dn)
    slope = data @ ols_weights(ngroups, tgroup_s)
    return {
        "name": name,
        "estimator": "unweighted least squares over all groups",
        "ngroups": ngroups,
        "tgroup_s": tgroup_s,
        "rate_dn_s": rate_dn_s,
        "read_dn": read_dn,
        "gain_e_dn": gain_e_dn,
        "monte_carlo_std_dn_s": float(slope.std(ddof=1)),
        "monte_carlo_mean_dn_s": float(slope.mean()),
        "exact_std_dn_s": exact_ols_std(ngroups, tgroup_s, rate_dn_s, read_dn, gain_e_dn),
        "closed_form_std_dn_s": closed_form_ols_std(ngroups, tgroup_s, rate_dn_s, read_dn, gain_e_dn),
        "read_only_std_dn_s": closed_form_ols_std(ngroups, tgroup_s, 0.0, read_dn, gain_e_dn),
    }


def median_case(rng, name, ngroups, tgroup_s, rate_dn_s, read_dn, gain_e_dn, dropped):
    data = ramps(rng, ngroups, tgroup_s, rate_dn_s, read_dn, gain_e_dn)
    kept = [i for i in range(ngroups - 1) if i not in dropped]
    slope = np.median(np.diff(data, axis=1)[:, kept], axis=1) / tgroup_s
    return {
        "name": name,
        "estimator": "median of the group differences left after dropping the listed ones",
        "ngroups": ngroups,
        "tgroup_s": tgroup_s,
        "rate_dn_s": rate_dn_s,
        "read_dn": read_dn,
        "gain_e_dn": gain_e_dn,
        "dropped_differences": dropped,
        "monte_carlo_std_dn_s": float(slope.std(ddof=1)),
        "monte_carlo_mean_dn_s": float(slope.mean()),
    }


def main():
    rng = np.random.default_rng(SEED)
    t = 14.589
    cases = [
        ols_case(rng, "ols_dark", 10, t, 0.0, 8.0, 1.0),
        ols_case(rng, "ols_half", 10, t, 0.5, 8.0, 1.0),
        ols_case(rng, "ols_five", 10, t, 5.0, 8.0, 1.0),
        ols_case(rng, "ols_fifty", 10, t, 50.0, 8.0, 1.0),
        ols_case(rng, "ols_fifty_twenty_groups", 20, t, 50.0, 8.0, 1.0),
        ols_case(rng, "ols_miri_like", 40, 2.775, 200.0, 5.0, 3.9),
        median_case(rng, "median_dark_hit_at_group_five", 10, t, 0.0, 8.0, 1.0, [4]),
        median_case(rng, "median_half_hit_at_group_five", 10, t, 0.5, 8.0, 1.0, [4]),
        median_case(rng, "median_fifty_hit_at_group_five", 10, t, 50.0, 8.0, 1.0, [4]),
        median_case(rng, "median_dark_three_groups", 3, t, 0.0, 8.0, 1.0, []),
        median_case(rng, "median_fifty_two_hits", 10, t, 50.0, 8.0, 1.0, [1, 5]),
    ]
    document = {
        "generator": "scripts/oracle/quick_slope_noise_oracle.py",
        "numpy": np.__version__,
        "python": sys.version.split()[0],
        "seed": SEED,
        "ramps_per_case": RAMPS,
        "read_noise_difference_sigma_dn": {"read_dn": 8.0, "sigma_dn": 8.0 * math.sqrt(2.0)},
        "cases": cases,
    }
    with open(OUT, "w", encoding="utf-8", newline="\n") as handle:
        json.dump(document, handle, indent=2)
        handle.write("\n")
    for case in cases:
        print(case["name"], round(case["monte_carlo_std_dn_s"], 5), round(case.get("closed_form_std_dn_s", float("nan")), 5))


if __name__ == "__main__":
    main()
