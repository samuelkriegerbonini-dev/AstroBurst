import json
import os
import platform
import sys
from pathlib import Path

import astropy
import numpy
import scipy
from astropy.io import fits
from scipy.optimize import brentq, least_squares
from scipy.special import erf

DEFAULT_OUTPUT = Path(__file__).resolve().parents[2] / "src-tauri/tests/fixtures/survey_qa/euclid_e1_vis_image_fwhm.json"
DATA_ENV = "ASTROBURST_SCIENCE_DATA_DIR"
IMAGE = "survey/E1/EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_20241021T041200.630008Z_00.00_cut60_A.fits"
PIXEL_SCALE_ARCSEC = 0.1
HALF_STAMP = 8
OVERSAMPLE = 9
PROFILE_OVERSAMPLE = 41
AZIMUTH_STEPS = 72
STARS = [
    {"id": -531288437281048064, "x": 337.17675170716575, "y": 127.33561326757126, "point_like_prob": 0.85688704252},
    {"id": -531333416280977000, "x": 194.03916149564793, "y": 382.9983652463852, "point_like_prob": 0.78124177456},
    {"id": -531376414280981396, "x": 57.50839472978532, "y": 367.0048787024957, "point_like_prob": 0.39032259583},
    {"id": -531248933280926511, "x": 462.1290207987586, "y": 565.0723755902422, "point_like_prob": 0.37178790569},
]
MODELS = ["moffat", "double_gaussian", "gaussian"]
AZIMUTHS = 2.0 * numpy.pi * (numpy.arange(AZIMUTH_STEPS) + 0.5) / AZIMUTH_STEPS


def pixel_integrated_gaussian(dx, dy, sigma):
    scale = numpy.sqrt(2.0) * sigma
    along_x = 0.5 * (erf((dx + 0.5) / scale) - erf((dx - 0.5) / scale))
    along_y = 0.5 * (erf((dy + 0.5) / scale) - erf((dy - 0.5) / scale))
    return along_x * along_y


def pixel_integrated_moffat(dx, dy, gamma, alpha, oversample):
    offsets = (numpy.arange(oversample) + 0.5) / oversample - 0.5
    total = numpy.zeros_like(numpy.asarray(dx, dtype=float))
    for oy in offsets:
        for ox in offsets:
            total = total + (1.0 + ((dx + ox) ** 2 + (dy + oy) ** 2) / (gamma * gamma)) ** (-alpha)
    return total / (oversample * oversample)


def star_light(model, shape, dx, dy, oversample):
    if model == "moffat":
        return shape[0] * pixel_integrated_moffat(dx, dy, shape[1], shape[2], oversample)
    if model == "gaussian":
        return shape[0] * pixel_integrated_gaussian(dx, dy, shape[1])
    return shape[0] * pixel_integrated_gaussian(dx, dy, shape[1]) + shape[2] * pixel_integrated_gaussian(dx, dy, shape[3])


def image_fwhm(model, shape):
    def azimuthal_mean(radius):
        return float(numpy.mean(star_light(model, shape, radius * numpy.cos(AZIMUTHS), radius * numpy.sin(AZIMUTHS), PROFILE_OVERSAMPLE)))

    peak = azimuthal_mean(0.0)
    return 2.0 * brentq(lambda radius: azimuthal_mean(radius) - 0.5 * peak, 0.05, 20.0, xtol=1e-10)


def fit_star(image, x0, y0, model):
    xi, yi = int(round(x0)), int(round(y0))
    stamp = numpy.asarray(image[yi - HALF_STAMP : yi + HALF_STAMP + 1, xi - HALF_STAMP : xi + HALF_STAMP + 1], dtype=float)
    yy, xx = numpy.mgrid[-HALF_STAMP : HALF_STAMP + 1, -HALF_STAMP : HALF_STAMP + 1].astype(float)
    border = numpy.concatenate([stamp[0, :], stamp[-1, :], stamp[1:-1, 0], stamp[1:-1, -1]])
    background = float(numpy.median(border))
    flux = float(stamp.sum() - background * stamp.size)
    peak = float(stamp.max() - background)
    start, lower, upper = {
        "moffat": ([peak, 1.2, 2.5], [0.0, 0.2, 1.01], [numpy.inf, 40.0, 50.0]),
        "gaussian": ([flux, 0.8], [0.0, 0.15], [numpy.inf, 8.0]),
        "double_gaussian": ([0.7 * flux, 0.6, 0.3 * flux, 1.5], [0.0, 0.15, 0.0, 0.15], [numpy.inf, 8.0, numpy.inf, 8.0]),
    }[model]

    def residual(p):
        return (star_light(model, p[3:], xx - p[0], yy - p[1], OVERSAMPLE) + p[2] - stamp).ravel()

    fit = least_squares(
        residual,
        [x0 - xi, y0 - yi, background] + start,
        bounds=([-2.0, -2.0, -numpy.inf] + lower, [2.0, 2.0, numpy.inf] + upper),
        x_scale="jac",
        xtol=1e-13,
        ftol=1e-13,
        gtol=1e-13,
        max_nfev=5000,
    )
    return {
        "x": float(fit.x[0] + xi),
        "y": float(fit.x[1] + yi),
        "background": float(fit.x[2]),
        "shape": [float(value) for value in fit.x[3:]],
        "image_fwhm_px": float(image_fwhm(model, fit.x[3:])),
        "residual_rms_over_peak": float(numpy.sqrt(numpy.mean(fit.fun**2)) / stamp.max()),
    }


def main():
    root = Path(os.environ[DATA_ENV])
    output = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUTPUT
    image = fits.getdata(root / IMAGE).astype(float)
    stars = []
    for star in STARS:
        entry = dict(star)
        entry["fits"] = {model: fit_star(image, star["x"], star["y"], model) for model in MODELS}
        stars.append(entry)
    medians = {model: float(numpy.median([star["fits"][model]["image_fwhm_px"] for star in stars]) * PIXEL_SCALE_ARCSEC) for model in MODELS}
    payload = {
        "generator": "scripts/oracle/euclid_image_fwhm_oracle.py",
        "versions": {
            "python": platform.python_version(),
            "numpy": numpy.__version__,
            "scipy": scipy.__version__,
            "astropy": astropy.__version__,
        },
        "image": IMAGE,
        "pixel_scale_arcsec": PIXEL_SCALE_ARCSEC,
        "quantity": "FWHM of the star image: the full width at half maximum of the azimuthally averaged profile of the star after the pixel has integrated it, the quantity the application reports",
        "method": "each of the four catalogue stars: a circular profile integrated over every pixel plus a constant, fitted by unweighted least squares to a 17 x 17 stamp; the fitted star image (the profile after the pixel integral) is averaged over 72 azimuths and its half maximum radius is found by root finding. Three profiles: Moffat (9 x 9 sub-samples in the fit, 41 x 41 for the width), sum of two Gaussians and one Gaussian (both integrated in closed form)",
        "shape_parameters": {"moffat": "amplitude, gamma [px], alpha", "gaussian": "flux, sigma [px]", "double_gaussian": "flux, sigma [px], flux, sigma [px]"},
        "reasoning": "the PSF is 1.8 to 2.1 px wide, so the width of the star image depends on the profile that is read through the pixels: the three profiles fit the same four stars and differ by 17 percent. The reference is therefore the interval between the lowest and the highest median, not one number with a 5 percent tolerance. The earlier reference (0.17663 arcsec, median of two stars) was the FWHM of a Moffat evaluated at pixel centres: one more profile model, which sits below this interval because it does not integrate over the pixel in the fit",
        "stars": stars,
        "median_image_fwhm_arcsec": medians,
        "image_fwhm_arcsec_min": min(medians.values()),
        "image_fwhm_arcsec_max": max(medians.values()),
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=1) + "\n", encoding="utf-8")
    for star in stars:
        print(star["id"], {model: (round(fit["image_fwhm_px"], 4), round(fit["residual_rms_over_peak"], 4)) for model, fit in star["fits"].items()})
    print(medians, payload["image_fwhm_arcsec_min"], payload["image_fwhm_arcsec_max"])


if __name__ == "__main__":
    main()
