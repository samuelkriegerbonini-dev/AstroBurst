import json
import platform
import sys
from pathlib import Path

import astropy
import astropy.units as u
import numpy
import photutils
import scipy
from scipy.optimize import curve_fit

DEFAULT_OUTPUT = Path(__file__).resolve().parents[2] / "src-tauri/tests/fixtures/cube/cube_review_goldens.json"
SEED = 20261125
MAD_TO_SIGMA = 1.482602218505602
C_KMS = 299792.458

INTEGRATED_UNITS = [
    ("MJy/sr", "km s-1", "MJy sr-1 km s-1"),
    ("Jy/beam", "km s-1", "Jy beam-1 km s-1"),
    ("Jy", "km s-1", "Jy km s-1"),
    ("K", "km s-1", "K km s-1"),
    ("uJy/arcsec2", "km s-1", "uJy arcsec-2 km s-1"),
    ("MJy/sr", "chan", "MJy sr-1 chan"),
    ("Jy/beam", "chan", "Jy beam-1 chan"),
    ("Jy", "chan", "Jy chan"),
]
LEGACY_UNITS = ["MJy/sr km/s", "Jy/beam km/s", "MJy/sr x channels"]


def unit_rows():
    rows = []
    for bunit, factor, written in INTEGRATED_UNITS:
        intended = u.Unit(bunit) * u.Unit(factor, format="fits")
        parsed = u.Unit(written, format="fits")
        rows.append(
            {
                "bunit": bunit,
                "factor": factor,
                "written": written,
                "parsed": parsed.to_string(),
                "equals_intended": bool(parsed.is_equivalent(intended) and numpy.isclose(parsed.to(intended), 1.0)),
            }
        )
    return rows


def legacy_rows():
    rows = []
    for text in LEGACY_UNITS:
        try:
            parsed = u.Unit(text, format="fits").to_string()
        except ValueError as error:
            parsed = "does not parse: " + str(error).splitlines()[0]
        rows.append({"written": text, "parsed": parsed})
    return rows


def moment_two(values, velocities, floor):
    used = values > floor
    weight = numpy.where(used, values, 0.0)
    total = weight.sum(axis=1)
    mean = (weight * velocities).sum(axis=1) / total
    variance = (weight * velocities**2).sum(axis=1) / total - mean**2
    return numpy.sqrt(numpy.clip(variance, 0.0, None))


def moment_bias(peak_snr, channels, window, pixels, channel_kms, sigma_channels, threshold):
    rng = numpy.random.default_rng(SEED)
    depth = channels + 2 * window
    z = numpy.arange(depth, dtype=float)
    centre = window + (channels - 1) / 2.0
    line = peak_snr * numpy.exp(-0.5 * ((z - centre) / sigma_channels) ** 2)
    cube = line[None, :] + rng.standard_normal((pixels, depth))
    in_window = (z < window) | (z >= window + channels)
    xw = z[in_window]
    design = numpy.vstack([numpy.ones_like(xw), xw]).T
    coefficients, *_ = numpy.linalg.lstsq(design, cube[:, in_window].T, rcond=None)
    baseline = coefficients[0][:, None] + coefficients[1][:, None] * z[None, :]
    residual = cube - baseline
    window_residual = residual[:, in_window]
    median = numpy.median(window_residual, axis=1, keepdims=True)
    mad = numpy.median(numpy.abs(window_residual - median), axis=1)
    n = int(in_window.sum())
    noise = mad * MAD_TO_SIGMA * numpy.sqrt(n / (n - 2))
    noise = numpy.maximum(noise, numpy.median(noise))
    in_range = ~in_window
    velocities = (z[in_range] - centre) * channel_kms
    values = residual[:, in_range]
    plain = moment_two(values, velocities[None, :], numpy.zeros((pixels, 1)))
    clipped = moment_two(values, velocities[None, :], (threshold * noise)[:, None])
    return {
        "peak_snr": peak_snr,
        "channels": channels,
        "window_channels": window,
        "pixels": pixels,
        "channel_kms": channel_kms,
        "true_sigma_kms": sigma_channels * channel_kms,
        "threshold_sigma": threshold,
        "median_m2_positive_weights_kms": float(numpy.nanmedian(plain)),
        "median_m2_thresholded_kms": float(numpy.nanmedian(clipped)),
        "median_noise_estimate": float(numpy.median(noise)),
    }


def gaussian(x, amplitude, centre, sigma):
    return amplitude * numpy.exp(-0.5 * ((x - centre) / sigma) ** 2)


def coverage_without_err(trials, line_channels, window, peak_snr):
    rng = numpy.random.default_rng(SEED)
    depth = line_channels + 2 * window + 10
    z = numpy.arange(depth, dtype=float)
    centre_channel = (depth - 1) / 2.0
    line = slice(int(centre_channel) - line_channels // 2, int(centre_channel) + line_channels // 2 + 1)
    left = slice(line.start - 5 - window, line.start - 5)
    right = slice(line.stop + 5, line.stop + 5 + window)
    beyond_formal = 0
    beyond_floor = 0
    fitted = 0
    for _ in range(trials):
        truth = centre_channel + rng.uniform(-0.4, 0.4)
        spectrum = 1.0 + gaussian(z, 1.0, truth, 1.0) + rng.standard_normal(depth) / peak_snr
        xc = numpy.concatenate([z[left], z[right]])
        yc = numpy.concatenate([spectrum[left], spectrum[right]])
        slope, intercept = numpy.polyfit(xc, yc, 1)
        residual = yc - (intercept + slope * xc)
        scatter = MAD_TO_SIGMA * numpy.median(numpy.abs(residual - numpy.median(residual)))
        y = spectrum[line] - (intercept + slope * z[line])
        try:
            params, formal = curve_fit(gaussian, z[line], y, p0=[y.max(), z[line][numpy.argmax(y)], 1.0], absolute_sigma=False)
            _, absolute = curve_fit(
                gaussian, z[line], y, p0=params, sigma=numpy.full(y.size, scatter), absolute_sigma=True
            )
        except RuntimeError:
            continue
        formal_error = numpy.sqrt(formal[1, 1])
        absolute_error = numpy.sqrt(absolute[1, 1])
        if not numpy.isfinite(formal_error) or not numpy.isfinite(absolute_error):
            continue
        fitted += 1
        beyond_formal += abs(params[1] - truth) > 3.0 * formal_error
        beyond_floor += abs(params[1] - truth) > 3.0 * max(formal_error, absolute_error)
    return {
        "trials": trials,
        "fitted": fitted,
        "line_channels": line_channels,
        "window_channels_each_side": window,
        "peak_snr": peak_snr,
        "fraction_beyond_3_sigma_formal_errors": beyond_formal / fitted,
        "fraction_beyond_3_sigma_absolute_errors_floored": beyond_floor / fitted,
        "normal_expectation": 0.0026997960632601866,
    }


def float32_table(first, step, count):
    table = (first + step * numpy.arange(count)).astype(numpy.float32).astype(numpy.float64)
    steps = numpy.diff(table)
    median_step = float(numpy.median(steps))
    endpoint_step = float((table[-1] - table[0]) / (count - 1))
    k = numpy.arange(count)
    return {
        "first": first,
        "step": step,
        "count": count,
        "median_step": median_step,
        "endpoint_step": endpoint_step,
        "max_deviation_over_step_with_median_step": float(numpy.max(numpy.abs(table - (table[0] + k * median_step))) / abs(median_step)),
        "max_deviation_over_step_with_endpoint_step": float(
            numpy.max(numpy.abs(table - (table[0] + k * endpoint_step))) / abs(endpoint_step)
        ),
    }


def lsf_widths(resolving_power_sigma_kms, source_sigma_kms, redshift):
    stretch = 1.0 + redshift
    observer_sigma = stretch * source_sigma_kms
    observer_instrument = stretch * resolving_power_sigma_kms
    resolved = source_sigma_kms > resolving_power_sigma_kms
    return {
        "sigma_inst_kms": resolving_power_sigma_kms,
        "source_sigma_kms": source_sigma_kms,
        "redshift": redshift,
        "source_sigma_corr_kms": float(numpy.sqrt(source_sigma_kms**2 - resolving_power_sigma_kms**2)) if resolved else None,
        "observer_sigma_corr_kms": float(numpy.sqrt(observer_sigma**2 - observer_instrument**2)) if resolved else None,
        "observer_sigma_corr_before_the_fix_kms": float(numpy.sqrt(observer_sigma**2 - resolving_power_sigma_kms**2))
        if observer_sigma > resolving_power_sigma_kms
        else None,
    }


REAL_CUBE = "jw01266-o005_t001_nirspec_g395h-f290lp_s3d.fits"
REAL_REGION = {"x": 18.0, "y": 32.0, "r": 3.0, "weights": "photutils CircularAperture, method exact"}
REAL_MOMENT_RANGE = (1769, 1786)


def exact_circle(shape, x0, y0, radius):
    from photutils.aperture import CircularAperture

    return numpy.asarray(CircularAperture((x0, y0), r=radius).to_mask(method="exact").to_image(shape), dtype=numpy.float64)


def real_cube(folder):
    from astropy.io import fits

    path = Path(folder) / REAL_CUBE
    if not path.exists():
        return None
    with fits.open(path) as hdul:
        sci = numpy.asarray(hdul["SCI"].data, dtype=numpy.float64)
        dq = numpy.asarray(hdul["DQ"].data).astype(numpy.int64)
    valid = numpy.isfinite(sci) & ((dq & 1) == 0)
    weights = exact_circle(sci.shape[1:], REAL_REGION["x"], REAL_REGION["y"], REAL_REGION["r"])
    inside = weights > 0
    spaxel_valid = valid[:, inside]
    spaxel_weight = weights[inside]
    used = (spaxel_valid * spaxel_weight[None, :]).sum(axis=1)
    never = ~spaxel_valid.any(axis=0)
    incomplete = (~spaxel_valid[:, ~never]).any(axis=1)
    kept = float(spaxel_weight[~never].sum())
    blanked = incomplete & (used > 0)
    worst = int(numpy.argmin(numpy.where(blanked, used, numpy.inf)))
    z0, z1 = REAL_MOMENT_RANGE
    count = valid[z0 : z1 + 1].sum(axis=0)
    channels = z1 - z0 + 1
    return {
        "file": REAL_CUBE,
        "shape_zyx": list(sci.shape),
        "region": REAL_REGION,
        "aperture_spaxels": int(inside.sum()),
        "aperture_weight": float(weights.sum()),
        "spaxels_without_data_in_any_channel": int(never.sum()),
        "kept_weight": kept,
        "channels_with_incomplete_aperture_and_a_finite_sum": int(blanked.sum()),
        "channels_without_any_aperture_spaxel": int((used == 0).sum()),
        "least_covered_channel": worst,
        "least_covered_channel_coverage": float(used[worst] / kept),
        "coverage_of_channel_1887": float(used[1887] / kept),
        "moment_range": list(REAL_MOMENT_RANGE),
        "moment_pixels_with_data": int((count > 0).sum()),
        "moment_pixels_with_every_channel": int((count == channels).sum()),
        "moment_pixels_with_partial_coverage": int(((count > 0) & (count < channels)).sum()),
        "moment_fewest_channels": int(count[count > 0].min()),
    }


def main(output, real_folder):
    goldens = {
        "generator": "scripts/oracle/cube_review_goldens.py",
        "python": platform.python_version(),
        "astropy": astropy.__version__,
        "photutils": photutils.__version__,
        "numpy": numpy.__version__,
        "scipy": scipy.__version__,
        "seed": SEED,
        "integrated_units": unit_rows(),
        "legacy_units": legacy_rows(),
        "moment_two_bias": [
            moment_bias(30.0, 25, 20, 4000, 40.0, 1.0, 2.0),
            moment_bias(30.0, 11, 20, 4000, 40.0, 1.0, 2.0),
        ],
        "line_fit_coverage_without_err": coverage_without_err(4000, 5, 25, 30.0),
        "float32_wave_tab": [float32_table(4.9, 0.0008, 300), float32_table(4.9, 0.0008, 60)],
        "lsf_widths": [lsf_widths(60.0, 100.0, 0.5), lsf_widths(120.0, 100.0, 0.5), lsf_widths(47.152, 92.862, 3.5)],
        "redshifted_moment": {
            "rest_um": 0.656461,
            "redshift": 0.5,
            "source_sigma_kms": 100.0,
            "sigma_um": 0.656461 * 1.5 * 100.0 / C_KMS,
            "m2_source_frame_kms": 100.0,
            "m2_against_the_laboratory_rest_kms": 150.0,
            "m1_against_the_laboratory_rest_kms": C_KMS * 0.5,
        },
    }
    previous = json.loads(output.read_text(encoding="utf-8")).get("real_g395h_cube") if output.exists() else None
    goldens["real_g395h_cube"] = (real_cube(real_folder) if real_folder else None) or previous
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(goldens, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(goldens, indent=2))


if __name__ == "__main__":
    arguments = sys.argv[1:]
    real = arguments[arguments.index("--real-cubes") + 1] if "--real-cubes" in arguments else None
    positional = [a for a in arguments if a != "--real-cubes" and a != real]
    main(Path(positional[0]) if positional else DEFAULT_OUTPUT, real)
