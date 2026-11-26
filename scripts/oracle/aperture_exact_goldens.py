import json
import platform
import sys
from pathlib import Path

import numpy
import photutils
from photutils.aperture import CircularAnnulus, CircularAperture

DEFAULT_OUTPUT = Path(__file__).resolve().parents[2] / "src-tauri/tests/fixtures/aperture/photutils_exact_weights.json"
WEIGHT_CASES = [
    (32, 32, 10.0, 10.0, 5.0),
    (32, 32, 17.3, 21.8, 3.7),
    (32, 32, 0.5, 0.5, 2.0),
    (32, 32, 30.0, 12.25, 8.0),
    (32, 32, 15.0, 15.0, 0.4),
    (32, 32, 15.5, 15.5, 1.0),
    (32, 32, 30.6, 0.4, 4.0),
    (64, 64, 32.0, 32.0, 2.0),
    (64, 64, 32.3, 31.7, 3.0),
    (64, 64, 32.3, 31.7, 6.0),
    (64, 64, 1.0, 30.0, 4.0),
    (128, 128, 60.37, 71.62, 26.7),
]
ANNULUS_CASES = [(64, 64, 20.0, 20.0, 4.0, 9.0), (64, 64, 21.4, 18.7, 4.0, 9.0)]
FIELD_SIZE = 256
FIELD_STARS = [(48.0, 52.0), (120.3, 60.6), (200.7, 55.2), (60.4, 150.9), (140.5, 200.5), (205.2, 190.1)]


def exact_weights(rows, cols, x, y, r):
    return CircularAperture((x, y), r).to_mask(method="exact").to_image((rows, cols))


def weight_case(rows, cols, x, y, r):
    image = exact_weights(rows, cols, x, y, r)
    ys, xs = numpy.nonzero(image > 0)
    return {
        "rows": rows,
        "cols": cols,
        "x": x,
        "y": y,
        "r": r,
        "total": float(image.sum()),
        "sum_squares": float((image**2).sum()),
        "pixels": [[int(py), int(px), float(image[py, px])] for py, px in zip(ys, xs)],
    }


def annulus_case(rows, cols, x, y, r_in, r_out):
    image = CircularAnnulus((x, y), r_in, r_out).to_mask(method="exact").to_image((rows, cols))
    return {"rows": rows, "cols": cols, "x": x, "y": y, "r_in": r_in, "r_out": r_out, "total": float(image.sum())}


def centre_sampled_gaussian(rows, cols, x0, y0, amplitude, sigma, background):
    y, x = numpy.mgrid[0:rows, 0:cols].astype(float)
    return (background + amplitude * numpy.exp(-((x - x0) ** 2 + (y - y0) ** 2) / (2.0 * sigma * sigma))).astype(numpy.float32)


def boxcar_f_corr():
    stamp = exact_weights(64, 64, 32.3, 31.7, 6.0).astype(numpy.float32).astype(float)
    squares = 0.0
    for y in range(1, 63):
        for x in range(1, 63):
            squares += (stamp[y - 1 : y + 2, x - 1 : x + 2].sum() / 9.0) ** 2
    return float(numpy.sqrt(9.0 * squares / (stamp**2).sum()))


def interquartile_mean(values):
    ordered = numpy.sort(values)
    lo = len(ordered) // 4
    hi = min(max(3 * len(ordered) // 4, lo + 1), len(ordered))
    return float(ordered[lo:hi].mean())


def field_growth_ratios():
    y, x = numpy.mgrid[0:FIELD_SIZE, 0:FIELD_SIZE]
    image = 100.0 + ((7 * y + 13 * x) % 5) - 2.0
    for x0, y0 in FIELD_STARS:
        image = image + 3000.0 * numpy.exp(-((x - x0) ** 2 + (y - y0) ** 2) / 8.0)
    image = image.astype(numpy.float32).astype(float)
    ratios = []
    for x0, y0 in FIELD_STARS:
        ring = CircularAnnulus((x0, y0), 20.0, 26.0).to_mask(method="exact").to_image(image.shape)
        sky = interquartile_mean(image[ring >= 0.5])
        fluxes = []
        for radius in (3.0, 8.0):
            weights = exact_weights(FIELD_SIZE, FIELD_SIZE, x0, y0, radius)
            fluxes.append(float((weights * image).sum() - sky * weights.sum()))
        ratios.append(fluxes[1] / fluxes[0])
    return ratios


def mean_sum_squares(radius, count, seed):
    rng = numpy.random.default_rng(seed)
    centres = 32.0 + rng.uniform(-0.5, 0.5, size=(count, 2))
    return float(numpy.mean([(exact_weights(64, 64, x, y, radius) ** 2).sum() for x, y in centres]))


def scalar_goldens():
    star = centre_sampled_gaussian(64, 64, 32.0, 32.0, 1000.0, 2.0, 0.0).astype(float)
    three = exact_weights(64, 64, 32.3, 31.7, 3.0)
    six = exact_weights(64, 64, 32.3, 31.7, 6.0)
    cut = exact_weights(64, 64, 1.0, 30.0, 4.0)
    ratios = field_growth_ratios()
    return {
        "depth_r3_pixels": int((three > 0).sum()),
        "depth_r3_total": float(three.sum()),
        "depth_r3_sum_squares": float((three**2).sum()),
        "depth_r6_pixels": int((six > 0).sum()),
        "depth_r6_total": float(six.sum()),
        "depth_r6_sum_squares": float((six**2).sum()),
        "depth_constant_2_sum": float(2.0 * three.sum()),
        "depth_formal_err_sigma_half": float(0.5 * numpy.sqrt((three**2).sum())),
        "depth_boxcar_f_corr": boxcar_f_corr(),
        "depth_r10_mean_sum_squares": mean_sum_squares(10.0, 2000, 1),
        "forced_gaussian_r2_flux": float((exact_weights(64, 64, 32.0, 32.0, 2.0) * star).sum()),
        "forced_edge_on_image_weight": float(cut.sum()),
        "forced_edge_full_weight": float(numpy.pi * 16.0),
        "forced_field_growth_ratios": ratios,
        "forced_field_growth_median": float(numpy.median(ratios)),
    }


def main():
    output = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUTPUT
    payload = {
        "generator": "scripts/oracle/aperture_exact_goldens.py",
        "method": "photutils CircularAperture.to_mask(method='exact'), pixel centres at integer coordinates",
        "versions": {
            "python": platform.python_version(),
            "numpy": numpy.__version__,
            "photutils": photutils.__version__,
        },
        "weights": [weight_case(*case) for case in WEIGHT_CASES],
        "annuli": [annulus_case(*case) for case in ANNULUS_CASES],
        "goldens": scalar_goldens(),
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=1) + "\n", encoding="utf-8")
    print(json.dumps(payload["goldens"], indent=1))
    print(json.dumps(payload["versions"]))
    for case in payload["weights"]:
        print(case["rows"], case["x"], case["y"], case["r"], len(case["pixels"]), case["total"], case["sum_squares"])


if __name__ == "__main__":
    main()
