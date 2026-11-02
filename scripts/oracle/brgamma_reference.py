import hashlib
import json
import platform
from pathlib import Path

import numpy as np
import scipy
from scipy.optimize import curve_fit


REST_UM = 2.16612
LIGHT_KMS = 299792.458
DEPTH = 121
STEP_KMS = 20.0
OUTPUT = Path(__file__).resolve().parents[2] / "src-tauri/tests/fixtures/spectral/brgamma_reference.json"


def gaussian(velocity, amplitude, centre, sigma):
    return amplitude * np.exp(-0.5 * ((velocity - centre) / sigma) ** 2)


def reference(sci, err, dq, start):
    velocity = (np.arange(DEPTH) - 60) * STEP_KMS
    continuum = np.r_[0:20, 101:121]
    line = np.arange(35, 86)
    continuum = continuum[(dq[continuum] & 1) == 0]
    line = line[(dq[line] & 1) == 0]
    design = np.column_stack([np.ones(DEPTH), velocity / 1000])
    weighted = design[continuum] / err[continuum, None]
    baseline = np.linalg.lstsq(weighted, sci[continuum] / err[continuum], rcond=None)[0]
    covariance_baseline = np.linalg.inv(weighted.T @ weighted)

    def fit(level):
        return curve_fit(
            gaussian,
            velocity[line],
            sci[line] - design[line] @ level,
            p0=start,
            sigma=err[line],
            absolute_sigma=True,
            method="lm",
            ftol=1e-12,
            xtol=1e-12,
            gtol=1e-12,
            maxfev=10000,
        )

    parameters, conditional_covariance = fit(baseline)
    sensitivity = np.empty((3, 2))
    for column in range(2):
        delta = np.zeros(2)
        delta[column] = 1e-5
        sensitivity[:, column] = (fit(baseline + delta)[0] - fit(baseline - delta)[0]) / (2e-5)
    covariance = conditional_covariance + sensitivity @ covariance_baseline @ sensitivity.T
    amplitude, centre, sigma = parameters
    gradient = np.sqrt(2 * np.pi) * np.array([sigma, 0, amplitude])
    flux = amplitude * sigma * np.sqrt(2 * np.pi)
    flux_err = np.sqrt(gradient @ covariance @ gradient)
    sigma_inst = LIGHT_KMS / (2700 * np.sqrt(8 * np.log(2)))
    residual = sci[line] - design[line] @ baseline - gaussian(velocity[line], *parameters)
    return {
        "amplitude": float(amplitude),
        "velocity_kms": float(centre),
        "sigma_kms": float(sigma),
        "sigma_corr_kms": float(np.sqrt(sigma**2 - sigma_inst**2)),
        "flux": float(flux),
        "flux_err": float(flux_err),
        "snr": float(flux / flux_err),
        "amplitude_err": float(np.sqrt(covariance[0, 0])),
        "v_err_kms": float(np.sqrt(covariance[1, 1])),
        "sigma_err_kms": float(np.sqrt(covariance[2, 2])),
        "continuum_at_rest": float(baseline[0]),
        "continuum_slope_per_kms": float(baseline[1] / 1000),
        "chi2_red": float(np.sum((residual / err[line]) ** 2) / (len(line) - 3)),
        "dropped_dq": np.flatnonzero((dq & 1) != 0).tolist(),
        "detected": bool(abs(flux / flux_err) >= 3),
    }


def main():
    velocity = (np.arange(DEPTH) - 60) * STEP_KMS
    definitions = [
        ("emission", 10.0, 120.0, 60.0, 0),
        ("bit24_without_do_not_use", 4.0, -140.0, 90.0, 1 << 24),
        ("masked_line_and_continuum_outliers", 6.0, 35.0, 70.0, 1 << 31),
        ("absorption", -3.0, -70.0, 80.0, 0),
        ("converged_below_snr_threshold", 0.03, 80.0, 90.0, 0),
        ("no_usable_samples", 5.0, 0.0, 75.0, 1),
    ]
    cases = []
    for index, (name, amplitude, centre, sigma, bits) in enumerate(definitions):
        err = (0.1 * (1 + 0.2 * np.arange(DEPTH) / (DEPTH - 1))).astype(np.float32)
        noise = 0.002 * np.sin(np.arange(DEPTH) * 1.37 + index)
        sci = (2 + 0.0004 * velocity + gaussian(velocity, amplitude, centre, sigma) + noise).astype(np.float32)
        dq = np.full(DEPTH, bits, dtype=np.uint32)
        if index == 2:
            sci[[10, 50]] += 80
            dq[10] = (1 << 24) | 1
            dq[50] = (1 << 31) | 1
        identity = hashlib.sha256(sci.astype("<f4").tobytes() + err.astype("<f4").tobytes() + dq.astype("<u4").tobytes()).hexdigest()
        cases.append({
            "name": name,
            "x": index % 3,
            "y": index // 3,
            "input_sha256": identity,
            "sci": sci.tolist(),
            "err": err.tolist(),
            "dq": dq.tolist(),
            "expected": None if index == 5 else reference(sci.astype(float), err.astype(float), dq, [amplitude, centre, sigma]),
        })
    report = {
        "kind": "synthetic_brgamma_scipy_reference",
        "rest_um": REST_UM,
        "depth": DEPTH,
        "crval3": REST_UM * (1 - 60 * STEP_KMS / LIGHT_KMS),
        "cdelt3": REST_UM * STEP_KMS / LIGHT_KMS,
        "versions": {"python": platform.python_version(), "numpy": np.__version__, "scipy": scipy.__version__},
        "generator_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "method": "NumPy weighted linear continuum; SciPy MINPACK Gaussian fit with absolute ERR; central refits propagate continuum covariance independently of the Rust Jacobian implementation",
        "reference": "https://docs.scipy.org/doc/scipy/reference/generated/scipy.optimize.curve_fit.html",
        "limitations": ["Synthetic fixture, not the unidentified observed Br-gamma spaxels", "Optical velocities; no barycentric correction", "Independent channel errors; no spectral covariance", "Gaussian instrument sigma from R=2700; no empirical LSF"],
        "cases": cases,
    }
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    print(json.dumps({"fixture": str(OUTPUT), "cases": len(cases), "versions": report["versions"], "reference_detections": [c["expected"]["detected"] if c["expected"] else None for c in cases]}))


if __name__ == "__main__":
    main()
