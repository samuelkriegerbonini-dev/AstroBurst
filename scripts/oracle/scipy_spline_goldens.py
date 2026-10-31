import argparse
import hashlib
import importlib.metadata
import json
import math
import platform
import sys
from pathlib import Path

import numpy as np
from scipy import ndimage
from scipy.optimize import curve_fit

MANIFEST_VERSION = 1
CREATED_BY = "scripts/oracle/scipy_spline_goldens.py"
GOLDEN = "scipy_spline.json"
FOLDER = "scipy"
D1_PSF = "d1/psf/d1_psf_JWST_F150W_20mas.fits"
FITS_BLOCK = 2880
CARD = 80
GRID_ROWS = 7
GRID_COLS = 9
MOFFAT_SIZE = 61
MOFFAT_ALPHA2 = 16.0
ROTATION_DEG = 20.0
GAUSSIAN_SIGMA_TO_FWHM = 2.0 * math.sqrt(2.0 * math.log(2.0))
FWHM_PSFS = [
    "d1/psf/d1_psf_JWST_F150W_20mas.fits",
    "d1/psf/d1_psf_JWST_F200W_20mas.fits",
    "d1/psf/d1_psf_HST_F814W_40mas.fits",
    "d1/psf/d1_psf_JWST_F150W_40mas.fits",
    "d1/psf/d1_psf_JWST_F277W_40mas.fits",
]
EDGE_POINTS = [
    [0.0, 0.0],
    [6.0, 8.0],
    [-1e-9, 3.0],
    [6.0 + 1e-9, 3.0],
    [3.5, -0.5],
    [2.25, 8.0],
    [-0.5, -0.5],
    [3.3, 4.7],
    [-2.0, 4.0],
    [8.5, 1.25],
]
LINE_POINTS = [-13.0, -0.5, -1e-9, 0.0, 0.25, 3.5, 7.75, 8.0, 8.0 + 1e-9, 8.6, 21.0]


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def file_entry(path, root):
    path = Path(path)
    return {"path": path.relative_to(root).as_posix(), "bytes": path.stat().st_size, "sha256": sha256(path)}


def plain(value):
    if isinstance(value, dict):
        return {key: plain(item) for key, item in value.items()}
    if isinstance(value, np.ndarray):
        return plain(value.tolist())
    if isinstance(value, (list, tuple)):
        return [plain(item) for item in value]
    if isinstance(value, (float, np.floating)):
        value = float(value)
        return value if math.isfinite(value) else None
    if isinstance(value, np.integer):
        return int(value)
    return value


def read_primary_f64(path):
    raw = Path(path).read_bytes()
    cards = {}
    offset = 0
    while True:
        card = raw[offset:offset + CARD].decode("ascii")
        offset += CARD
        key = card[:8].strip()
        if key == "END":
            break
        if card[8:10] == "= ":
            cards[key] = card[10:].split("/")[0].strip()
    start = -(-offset // FITS_BLOCK) * FITS_BLOCK
    if int(cards["BITPIX"]) != -64 or int(cards["NAXIS"]) != 2:
        raise SystemExit(f"{path}: expected a 2D float64 primary image")
    cols, rows = int(cards["NAXIS1"]), int(cards["NAXIS2"])
    data = np.frombuffer(raw, dtype=">f8", count=rows * cols, offset=start).reshape(rows, cols).astype(np.float64)
    return data, cards


def grid():
    rows, cols = np.mgrid[0:GRID_ROWS, 0:GRID_COLS]
    return (((rows * GRID_COLS + cols) * 37) % 23 - 11).astype(np.float64) + 0.25 * rows


def moffat():
    centre = (MOFFAT_SIZE - 1) / 2
    y, x = np.mgrid[0:MOFFAT_SIZE, 0:MOFFAT_SIZE].astype(np.float64)
    q = 1.0 + ((x - centre) * (x - centre) + (y - centre) * (y - centre)) / MOFFAT_ALPHA2
    return 1.0 / (q * q)


def lightstack_geometry(psf, source_scale, target_scale, target_shape):
    ny, nx = psf.shape
    required_nx = int(np.ceil(target_shape[1] * target_scale / source_scale))
    required_ny = int(np.ceil(target_shape[0] * target_scale / source_scale))
    required_nx += 1 if required_nx % 2 == 0 else 0
    required_ny += 1 if required_ny % 2 == 0 else 0
    padded_nx = max(required_nx, nx)
    padded_ny = max(required_ny, ny)
    padded_nx += 1 if padded_nx % 2 == 0 else 0
    padded_ny += 1 if padded_ny % 2 == 0 else 0
    padded = np.zeros((padded_ny, padded_nx), dtype=float)
    y0 = padded_ny // 2 - ny // 2
    x0 = padded_nx // 2 - nx // 2
    padded[y0:y0 + ny, x0:x0 + nx] = psf
    padded /= np.nansum(padded)
    scale = source_scale / target_scale
    matrix = np.array([[1.0 / scale, 0.0], [0.0, 1.0 / scale]])
    input_center = ((padded.shape[0] - 1) / 2, (padded.shape[1] - 1) / 2)
    output_center = ((target_shape[0] - 1) / 2, (target_shape[1] - 1) / 2)
    offset = np.array(input_center) - matrix @ np.array(output_center)
    raw = ndimage.affine_transform(padded, matrix=matrix, offset=offset, output_shape=target_shape, order=3, mode="constant", cval=0.0)
    final = raw.copy()
    final[final < 0] = 0.0
    final /= np.nansum(final)
    return {
        "source_shape": list(psf.shape),
        "source_pixel_scale_arcsec": source_scale,
        "target_pixel_scale_arcsec": target_scale,
        "target_shape": list(target_shape),
        "padded_shape": list(padded.shape),
        "matrix_diagonal": float(matrix[0, 0]),
        "offset": offset,
        "affine_output": raw,
        "resampled": final,
    }


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


def gaussian2d(xy, amplitude, x_mean, y_mean, x_stddev, y_stddev, theta):
    x, y = xy
    cost2 = np.cos(theta) ** 2
    sint2 = np.sin(theta) ** 2
    sin2t = np.sin(2.0 * theta)
    xstd2 = x_stddev ** 2
    ystd2 = y_stddev ** 2
    xdiff = x - x_mean
    ydiff = y - y_mean
    a = 0.5 * (cost2 / xstd2 + sint2 / ystd2)
    b = 0.5 * (sin2t / xstd2 - sin2t / ystd2)
    c = 0.5 * (sint2 / xstd2 + cost2 / ystd2)
    return amplitude * np.exp(-(a * xdiff ** 2 + b * xdiff * ydiff + c * ydiff ** 2))


def minpack_fwhm(psf):
    data = np.nan_to_num(np.asarray(psf, dtype=float))
    yc, xc = np.unravel_index(np.argmax(data), data.shape)
    fwhm0 = 2 * np.sqrt(np.sum(data > data[yc, xc] / 2) / np.pi)
    half = int(np.ceil(max(5, 3 * fwhm0)))
    if yc + half >= data.shape[0] or xc + half >= data.shape[1]:
        raise SystemExit("the measure_psf_fwhm box leaves the array")
    y, x = np.mgrid[max(yc - half, 0):yc + half + 1, max(xc - half, 0):xc + half + 1]
    sigma0 = fwhm0 / GAUSSIAN_SIGMA_TO_FWHM
    start = [data[yc, xc], xc, yc, sigma0, sigma0, 0.0]
    popt, _ = curve_fit(gaussian2d, (x.ravel().astype(float), y.ravel().astype(float)), data[y, x].ravel(), p0=start, method="lm", maxfev=20000)
    return (abs(popt[3]) + abs(popt[4])) / 2 * GAUSSIAN_SIGMA_TO_FWHM


def rotated_gaussian():
    y, x = np.mgrid[0:31, 0:31].astype(float)
    return gaussian2d((x, y), 1.0, 15.3, 14.6, 2.5, 1.5, 0.6)


def build(fixtures):
    values = grid()
    angle = math.radians(ROTATION_DEG)
    rotation = np.array([[math.cos(angle), -math.sin(angle)], [math.sin(angle), math.cos(angle)]])
    rotation_offset = np.array([1.2, -0.7])
    points = np.array(EDGE_POINTS).T
    d1_psf, d1_cards = read_primary_f64(fixtures / D1_PSF)
    d1_source_scale = math.hypot(float(d1_cards["CD1_1"]), float(d1_cards["CD2_1"])) * 3600.0
    binned = cumulative_rebin(cumulative_rebin(d1_psf, 21, 2.0, 0), 21, 2.0, 1)
    return {
        "grid": values,
        "spline_filter1d": {
            "line": values[3],
            "mirror": ndimage.spline_filter1d(values[3], order=3, mode="mirror"),
            "reflect": ndimage.spline_filter1d(values[3], order=3, mode="reflect"),
        },
        "spline_filter_mirror": ndimage.spline_filter(values, order=3, mode="mirror"),
        "shift": [
            {"shift": [0.3, -1.7], "mode": "constant", "cval": 0.0, "output": ndimage.shift(values, (0.3, -1.7), order=3, mode="constant", cval=0.0)},
            {"shift": [-0.45, 0.8], "mode": "constant", "cval": None, "output": ndimage.shift(values, (-0.45, 0.8), order=3, mode="constant", cval=np.nan)},
            {"shift": [0.3, -1.7], "mode": "nearest", "cval": 0.0, "output": ndimage.shift(values, (0.3, -1.7), order=3, mode="nearest")},
        ],
        "map_coordinates": {
            "points": EDGE_POINTS,
            "constant": {"cval": -7.0, "values": ndimage.map_coordinates(values, points, order=3, mode="constant", cval=-7.0)},
            "nearest": ndimage.map_coordinates(values, points, order=3, mode="nearest"),
        },
        "affine_rotation": {
            "matrix": rotation,
            "offset": rotation_offset,
            "output_shape": [8, 10],
            "output": ndimage.affine_transform(values, rotation, offset=rotation_offset, output_shape=(8, 10), order=3, mode="constant", cval=0.0),
        },
        "zoom": [
            {"factor": 2.0, "mode": "nearest", "output": ndimage.zoom(values, 2, order=3, mode="nearest", prefilter=True)},
            {"factor": 0.5, "mode": "constant", "output": ndimage.zoom(values, 0.5, order=3)},
        ],
        "line": {
            "values": values[2],
            "coordinates": LINE_POINTS,
            "nearest": ndimage.map_coordinates(values[2], [LINE_POINTS], order=3, mode="nearest"),
            "constant": {"cval": -3.0, "values": ndimage.map_coordinates(values[2], [LINE_POINTS], order=3, mode="constant", cval=-3.0)},
        },
        "gaussian_fwhm": {
            "description": "scipy curve_fit (MINPACK lm) of the astropy Gaussian2D model on the box and first guess of LIGHTSTACK 0.4.0 measure_psf_fwhm; FWHM in pixels",
            "psfs": [{"input": path, "fwhm_px": minpack_fwhm(read_primary_f64(fixtures / path)[0])} for path in FWHM_PSFS],
            "rotated": {"sigma": [2.5, 1.5], "theta": 0.6, "centre": [15.3, 14.6], "shape": [31, 31], "fwhm_px": minpack_fwhm(rotated_gaussian())},
        },
        "cumulative_rebin": {
            "input": D1_PSF,
            "description": "per axis, rows first: cubic spline (mode nearest) of the cumulative flux at pixel edges, differenced at the output pixel edges; output centre on the input centre; then divided by the sum",
            "factor": 2.0,
            "output_shape": [21, 21],
            "output": binned / binned.sum(),
        },
        "lightstack_resample": {
            "d1_f150w": {"input": D1_PSF, **lightstack_geometry(d1_psf, d1_source_scale, 0.04, (21, 21))},
            "moffat_tutorial2": {
                "input": f"moffat beta 2, alpha^2 {MOFFAT_ALPHA2} px^2, {MOFFAT_SIZE}x{MOFFAT_SIZE}, centre pixel, 1/(q*q) with q = 1 + r^2/alpha^2",
                **lightstack_geometry(moffat(), 0.05, 1.004, (21, 21)),
            },
        },
    }


def versions():
    return {"python": platform.python_version(), "numpy": importlib.metadata.version("numpy"), "scipy": importlib.metadata.version("scipy")}


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="scipy_spline_goldens",
        description="Write scipy.ndimage cubic-spline reference values for the AstroBurst B-spline interpolator and the LIGHTSTACK PSF resampling geometry.",
    )
    default = Path(__file__).resolve().parents[2] / "src-tauri" / "tests" / "fixtures" / "lightstack"
    parser.add_argument("--fixtures", type=Path, default=default, help="the tests/fixtures/lightstack folder")
    args = parser.parse_args(argv)
    fixtures = args.fixtures.resolve()
    out_dir = fixtures / FOLDER
    out_dir.mkdir(parents=True, exist_ok=True)
    golden = out_dir / GOLDEN
    golden.write_text(json.dumps(plain(build(fixtures)), indent=1, allow_nan=False) + "\n", encoding="utf-8", newline="\n")
    manifest = {
        "manifest_version": MANIFEST_VERSION,
        "kind": "scipy_reference",
        "created_by": CREATED_BY,
        "regenerate": "docs\\notCommit\\oracle\\.venv\\Scripts\\python.exe -I scripts\\oracle\\scipy_spline_goldens.py",
        "versions": versions(),
        "platform": platform.platform(),
        "inputs": [file_entry(fixtures / path, fixtures) for path in dict.fromkeys([D1_PSF] + FWHM_PSFS)],
        "files": [file_entry(golden, out_dir)],
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2, allow_nan=False) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {golden} ({golden.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
