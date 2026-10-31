import argparse
import hashlib
import importlib.metadata
import json
import math
import platform
import sys
from pathlib import Path

import numpy as np
from photutils.psf_matching import CosineBellWindow, HanningWindow, SplitCosineBellWindow, TopHatWindow, TukeyWindow, make_kernel
from scipy import fft as scipy_fft
from scipy.signal import windows

MANIFEST_VERSION = 1
CREATED_BY = "scripts/oracle/kernel_goldens.py"
GOLDEN = "kernel_goldens.json"
FOLDER = "kernels"
FITS_BLOCK = 2880
CARD = 80
TARGET = "d1/psf/d1_psf_JWST_F444W_40mas.fits"
SOURCES = {
    "F814W": "d1/psf/d1_psf_HST_F814W_40mas.fits",
    "F150W": "d1/psf/d1_psf_JWST_F150W_40mas.fits",
    "F200W": "d1/psf/d1_psf_JWST_F200W_40mas.fits",
    "F277W": "d1/psf/d1_psf_JWST_F277W_40mas.fits",
    "F356W": "d1/psf/d1_psf_JWST_F356W_40mas.fits",
}
TUTORIAL_EPS = 5e-2
WIENER_EXTRA = [
    {"source": "F150W", "shape": [31, 31], "eps": 1e-3, "source_factor": 1.0, "target_factor": 1.0},
    {"source": "F277W", "shape": [23, 27], "eps": 1e-2, "source_factor": 1.0, "target_factor": 1.0},
    {"source": "F814W", "shape": [21, 21], "eps": TUTORIAL_EPS, "source_factor": 0.97, "target_factor": 0.99},
]
REGULARIZATION = 1e-4
WINDOW_CASES = [
    {"source": "F150W", "window": "cosine_bell", "alpha": 0.7, "beta": 0.0},
    {"source": "F277W", "window": "cosine_bell", "alpha": 0.7, "beta": 0.0},
    {"source": "F356W", "window": "split_cosine_bell", "alpha": 0.3, "beta": 0.1},
    {"source": "F200W", "window": "tukey", "alpha": 0.5, "beta": 0.5},
    {"source": "F814W", "window": "top_hat", "alpha": 0.0, "beta": 0.6},
    {"source": "F814W", "window": "hanning", "alpha": 1.0, "beta": 0.0},
]


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
    return np.frombuffer(raw, dtype=">f8", count=rows * cols, offset=start).reshape(rows, cols).astype(np.float64)


def lightstack_build_kernel(psf_source, psf_ref, shape, eps):
    ny, nx = shape
    psf_s = np.zeros((ny, nx))
    psf_t = np.zeros((ny, nx))
    ys, xs = psf_source.shape
    yt, xt = psf_ref.shape
    psf_s[ny // 2 - ys // 2:ny // 2 - ys // 2 + ys, nx // 2 - xs // 2:nx // 2 - xs // 2 + xs] = psf_source
    psf_t[ny // 2 - yt // 2:ny // 2 - yt // 2 + yt, nx // 2 - xt // 2:nx // 2 - xt // 2 + xt] = psf_ref
    f_s = scipy_fft.fft2(scipy_fft.ifftshift(psf_s))
    f_t = scipy_fft.fft2(scipy_fft.ifftshift(psf_t))
    kernel = scipy_fft.fftshift(np.real(scipy_fft.ifft2(f_t * np.conj(f_s) / (np.abs(f_s) ** 2 + eps))))
    return kernel / np.sum(kernel)


PHOTUTILS_WINDOWS = {
    "split_cosine_bell": lambda case: SplitCosineBellWindow(alpha=case["alpha"], beta=case["beta"]),
    "cosine_bell": lambda case: CosineBellWindow(alpha=case["alpha"]),
    "tukey": lambda case: TukeyWindow(alpha=case["alpha"]),
    "top_hat": lambda case: TopHatWindow(beta=case["beta"]),
    "hanning": lambda case: HanningWindow(),
}


def window_kernel(source, target, case):
    window = PHOTUTILS_WINDOWS[case["window"]](case)
    return make_kernel(source, target, regularization=case["regularization"], window=window)


def build(fixtures):
    target = read_primary_f64(fixtures / TARGET)
    psfs = {name: read_primary_f64(fixtures / path) for name, path in SOURCES.items()}
    wiener = [
        {"source": name, "shape": list(target.shape), "eps": TUTORIAL_EPS, "source_factor": 1.0, "target_factor": 1.0}
        for name in SOURCES
    ] + WIENER_EXTRA
    for case in wiener:
        source = psfs[case["source"]] * case["source_factor"]
        case["kernel"] = lightstack_build_kernel(source, target * case["target_factor"], tuple(case["shape"]), case["eps"])
    window = [dict(case, regularization=REGULARIZATION) for case in WINDOW_CASES]
    for case in window:
        case["kernel"] = window_kernel(psfs[case["source"]], target, case)
    return {
        "target": TARGET,
        "sources": SOURCES,
        "wiener": {
            "description": "LIGHTSTACK 0.4.0 build_kernel (MIT, Copyright (c) 2026 Andressa Wille, Thallis Pessi) restated with scipy.fft: both PSFs times their factor, zero-padded to shape at N//2 - n//2, ifftshift, fft2, F_t conj(F_s) / (|F_s|^2 + eps), real ifft2, fftshift, divided by the sum",
            "cases": wiener,
        },
        "window": {
            "description": "a photutils run: photutils.psf_matching.make_kernel(source, target, regularization, window) with the window class named by each case (SplitCosineBellWindow, CosineBellWindow, TukeyWindow, TopHatWindow, HanningWindow)",
            "cases": window,
        },
        "windows": {
            "hanning_101": np.hanning(101),
            "tukey_101_0.4": windows.tukey(101, 0.4),
            "tukey_51_0.4": windows.tukey(51, 0.4),
            "tukey_11_0.5": windows.tukey(11, 0.5),
        },
    }


def versions():
    return {
        "python": platform.python_version(),
        "numpy": importlib.metadata.version("numpy"),
        "scipy": importlib.metadata.version("scipy"),
        "photutils": importlib.metadata.version("photutils"),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="kernel_goldens",
        description="Write reference matching kernels for the AstroBurst kernel builder: LIGHTSTACK's build_kernel restated with scipy.fft and photutils' make_kernel run with its window classes, on the D1 PSFs, plus numpy and scipy window values.",
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
        "regenerate": "docs\\notCommit\\oracle\\.venv313\\Scripts\\python.exe -I scripts\\oracle\\kernel_goldens.py",
        "versions": versions(),
        "platform": platform.platform(),
        "inputs": [file_entry(fixtures / path, fixtures) for path in [TARGET] + list(SOURCES.values())],
        "files": [file_entry(golden, out_dir)],
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2, allow_nan=False) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {golden} ({golden.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
