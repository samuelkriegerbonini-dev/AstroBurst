import json
import platform
import sys
from pathlib import Path

import numpy
import scipy
from scipy import optimize

DEFAULT_OUTPUT = Path(__file__).resolve().parents[2] / "src-tauri/tests/fixtures/line_measure/late_convergence_scipy.json"
SEED = 31337
REALISATION = 10
CHANNELS = 101
AXIS_START = 1.0
AXIS_STEP = 0.001
MIDDLE = 50
NOISE = 0.2
LINE = (47, 53)
WINDOWS = ((34, 43), (57, 66))
SIGMA_LOWER_BOUND_FRACTION = 0.25
TOLERANCE = 1e-15


def axis():
    return AXIS_START + AXIS_STEP * numpy.arange(CHANNELS)


def spectrum():
    rng = numpy.random.default_rng(SEED)
    x = axis()
    flux = None
    for _ in range(REALISATION + 1):
        centre = x[MIDDLE] + AXIS_STEP * rng.uniform(-0.5, 0.5)
        flux = (1.0 + numpy.exp(-(x - centre) ** 2 / (2 * AXIS_STEP * AXIS_STEP)) + rng.normal(0, NOISE, CHANNELS)).astype(numpy.float32)
    return flux


def straight_continuum(x, flux):
    channels = [z for window in WINDOWS for z in range(window[0], window[1] + 1)]
    reference = x[(LINE[0] + LINE[1]) // 2]
    slope, level = numpy.polyfit(x[channels] - reference, flux[channels].astype(float), 1)
    return lambda at: level + slope * (at - reference)


def gaussian_optimum(x, flux):
    continuum = straight_continuum(x, flux)
    channels = numpy.arange(LINE[0], LINE[1] + 1)
    xs = x[channels]
    excess = flux[channels].astype(float) - continuum(xs)
    low, high = xs[0], xs[-1]

    def residuals(p):
        return p[0] * numpy.exp(-(xs - p[1]) ** 2 / (2.0 * p[2] * p[2])) - excess

    start = [float(excess.max()), float(xs[numpy.argmax(excess)]), AXIS_STEP]
    bounds = ([0.0, low, SIGMA_LOWER_BOUND_FRACTION * AXIS_STEP], [numpy.inf, high, high - low])
    best = optimize.least_squares(residuals, start, bounds=bounds, xtol=TOLERANCE, ftol=TOLERANCE, gtol=TOLERANCE, max_nfev=100000)
    jacobian = best.jac
    dof = len(xs) - 3
    covariance = numpy.linalg.inv(jacobian.T @ jacobian) * (2.0 * best.cost / dof)
    return best.x, numpy.sqrt(numpy.diag(covariance))


def main():
    output = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUTPUT
    x = axis()
    flux = spectrum()
    params, errors = gaussian_optimum(x, flux)
    document = {
        "generator": "scripts/oracle/line_fit_late_convergence_oracle.py",
        "python": platform.python_version(),
        "numpy": numpy.__version__,
        "scipy": scipy.__version__,
        "method": "scipy.optimize.least_squares (trf) with xtol = ftol = gtol = 1e-15 on the continuum-subtracted line channels; continuum from numpy.polyfit of degree 1 over the two windows; errors from inv(J^T J) x RSS / dof",
        "seed": SEED,
        "realisation": REALISATION,
        "axis_start": AXIS_START,
        "axis_step": AXIS_STEP,
        "line": list(LINE),
        "continuum": [list(window) for window in WINDOWS],
        "flux_f32": [float(v) for v in flux],
        "amplitude": float(params[0]),
        "centre": float(params[1]),
        "sigma": float(params[2]),
        "amplitude_err": float(errors[0]),
        "centre_err": float(errors[1]),
        "sigma_err": float(errors[2]),
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(document, indent=1) + "\n")
    print(output, document["amplitude"], document["centre"], document["sigma"], document["amplitude_err"], document["centre_err"], document["sigma_err"])


if __name__ == "__main__":
    main()
