import argparse
import importlib.metadata
import json
import math
import platform
from pathlib import Path

import numpy as np
from astropy.modeling import fitting, models
from scipy import ndimage
from scipy.signal import convolve2d, correlate2d
from scipy.special import erf

CREATED_BY = "scripts/oracle/cubeprep_review_goldens.py"
GOLDEN = "cubeprep_review_goldens.json"
SIGMA_TO_FWHM = 2.0 * math.sqrt(2.0 * math.log(2.0))

NOISE_KERNEL_SIZE = 41
NOISE_KERNEL_SIGMA_PX = 2.0
NOISE_BOX = 15
NOISE_SHIFT = (0.5, 0.5)

PSF_FWHM_MAS = 50.0
PSF_SOURCE_MAS = 15.8
PSF_TARGET_MAS = 40.0
PSF_SOURCE_SIDE = 161
PSF_TARGET_SIDE = 65

AREA_SCALE_IN = 0.031
AREA_SCALE_OUT = 0.063


def gaussian_kernel(size, sigma):
    y, x = np.mgrid[:size, :size] - size // 2
    kernel = np.exp(-(x * x + y * y) / (2.0 * sigma * sigma))
    return kernel / kernel.sum()


def bilinear_weights(fy, fx):
    return np.outer([1.0 - fy, fy], [1.0 - fx, fx])


def aligned_map(weights):
    full = correlate2d(weights, weights, mode="full")
    centre = full.shape[0] // 2
    return full / full[centre, centre]


def region_sum_variance(weights, kernel, box):
    region = np.ones((box, box))
    after_kernel = convolve2d(region, kernel[::-1, ::-1], mode="full")
    on_input = convolve2d(after_kernel, weights[::-1, ::-1], mode="full")
    return float((on_input ** 2).sum())


def pixel_variance(weights, kernel):
    on_input = convolve2d(kernel[::-1, ::-1], weights[::-1, ::-1], mode="full")
    return float((on_input ** 2).sum())


def noise_case():
    weights = bilinear_weights(*NOISE_SHIFT)
    kernel = gaussian_kernel(NOISE_KERNEL_SIZE, NOISE_KERNEL_SIGMA_PX)
    return {
        "aligned_map": aligned_map(weights).tolist(),
        "aligned_err": float(np.sqrt((weights ** 2).sum())),
        "kernel_size": NOISE_KERNEL_SIZE,
        "kernel_sigma_px": NOISE_KERNEL_SIGMA_PX,
        "kernel_sum_squares": float((kernel ** 2).sum()),
        "box": NOISE_BOX,
        "region_sum_variance": region_sum_variance(weights, kernel, NOISE_BOX),
        "matched_pixel_variance": pixel_variance(weights, kernel),
    }


def integrated_line(side, pixel, sigma):
    edges = (np.arange(side + 1) - side / 2.0) * pixel
    cumulative = 0.5 * (1.0 + erf(edges / (math.sqrt(2.0) * sigma)))
    return np.diff(cumulative)


def integrated_gaussian(side, pixel, sigma):
    line = integrated_line(side, pixel, sigma)
    image = np.outer(line, line)
    return image / image.sum()


def fitted_fwhm(image, pixel):
    side = image.shape[0]
    y, x = np.mgrid[:side, :side]
    start = models.Gaussian2D(amplitude=image.max(), x_mean=side // 2, y_mean=side // 2, x_stddev=1.0, y_stddev=1.0)
    start.theta.fixed = True
    fit = fitting.LevMarLSQFitter()(start, x, y, image)
    return float(math.sqrt(abs(fit.x_stddev.value * fit.y_stddev.value)) * SIGMA_TO_FWHM * pixel)


def cumulative_rebin(values, output_len, factor, axis):
    moved = np.moveaxis(values, axis, -1)
    length = moved.shape[-1]
    cumulative = np.concatenate([np.zeros(moved.shape[:-1] + (1,)), np.cumsum(moved, axis=-1)], axis=-1)
    centre_in = (length - 1) / 2
    centre_out = (output_len - 1) / 2
    edges = []
    for output in range(output_len):
        centre = centre_in + (output - centre_out) * factor
        edges += [centre - factor / 2, centre + factor / 2]
    coordinates = np.clip(np.array(edges) + 0.5, 0.0, float(length))
    out = np.empty(moved.shape[:-1] + (output_len,))
    for index in np.ndindex(moved.shape[:-1]):
        sampled = ndimage.map_coordinates(cumulative[index], [coordinates], order=3, mode="nearest")
        out[index] = sampled[1::2] - sampled[0::2]
    return np.moveaxis(out, -1, axis)


def area_resampled_source(sigma):
    line = integrated_line(PSF_SOURCE_SIDE, PSF_SOURCE_MAS, sigma)
    source = np.outer(line, line).astype(np.float32).astype(np.float64)
    factor = PSF_TARGET_MAS / PSF_SOURCE_MAS
    binned = cumulative_rebin(cumulative_rebin(source, PSF_TARGET_SIDE, factor, 0), PSF_TARGET_SIDE, factor, 1)
    return binned / binned.sum()


def psf_case():
    sigma = PSF_FWHM_MAS / SIGMA_TO_FWHM
    target = integrated_gaussian(PSF_TARGET_SIDE, PSF_TARGET_MAS, sigma)
    centre = PSF_TARGET_SIDE // 2
    point = np.exp(-((np.arange(PSF_TARGET_SIDE) - centre) * PSF_TARGET_MAS) ** 2 / (2.0 * sigma * sigma))
    sampled = np.outer(point, point)
    sampled /= sampled.sum()
    area = area_resampled_source(sigma)
    return {
        "fwhm_mas": PSF_FWHM_MAS,
        "source_pixel_mas": PSF_SOURCE_MAS,
        "target_pixel_mas": PSF_TARGET_MAS,
        "source_side": PSF_SOURCE_SIDE,
        "integrated_peak": float(target[centre, centre]),
        "integrated_next": float(target[centre, centre + 1]),
        "integrated_diagonal": float(target[centre + 1, centre + 1]),
        "integrated_second": float(target[centre, centre + 2]),
        "integrated_fwhm_mas": fitted_fwhm(target, PSF_TARGET_MAS),
        "point_sampled_peak": float(sampled[centre, centre]),
        "point_sampled_fwhm_mas": fitted_fwhm(sampled, PSF_TARGET_MAS),
        "area_method_peak": float(area[centre, centre]),
        "area_method_next": float(area[centre, centre + 1]),
        "area_method_diagonal": float(area[centre + 1, centre + 1]),
        "area_method_second": float(area[centre, centre + 2]),
        "area_method_fwhm_mas": fitted_fwhm(area, PSF_TARGET_MAS),
    }


def area_case():
    return {"scale_in_arcsec": AREA_SCALE_IN, "scale_out_arcsec": AREA_SCALE_OUT, "pixel_area_ratio": (AREA_SCALE_OUT / AREA_SCALE_IN) ** 2}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=Path(__file__).resolve().parents[2] / "src-tauri" / "tests" / "fixtures" / "cubeprep")
    args = parser.parse_args()
    golden = {
        "created_by": CREATED_BY,
        "python": platform.python_version(),
        "numpy": np.__version__,
        "scipy": importlib.metadata.version("scipy"),
        "astropy": importlib.metadata.version("astropy"),
        "matched_noise": noise_case(),
        "area_psf": psf_case(),
        "lightstack_area_rule": area_case(),
    }
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / GOLDEN).write_text(json.dumps(golden, indent=2) + "\n")
    print(json.dumps(golden, indent=2))


if __name__ == "__main__":
    main()
