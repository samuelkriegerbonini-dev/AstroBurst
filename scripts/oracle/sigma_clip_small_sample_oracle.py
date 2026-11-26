import json
import platform
import sys
from pathlib import Path

import numpy
import scipy
from scipy import stats

DEFAULT_OUTPUT = Path(__file__).resolve().parents[2] / "src-tauri/tests/fixtures/stacking/student_thresholds.json"
SIGMAS = [1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0]
DEGREES_OF_FREEDOM = [1, 2, 3]
TRIALS = 400_000
SEED = 20261125


def threshold(sigma, dof):
    return float(stats.t.isf(stats.norm.sf(sigma), dof))


def deleted_residual_rejections(samples, sigma_low, sigma_high):
    kept = numpy.ones(samples.shape, dtype=bool)
    rows = numpy.arange(samples.shape[0])
    for _ in range(samples.shape[1] - 2):
        count = kept.sum(axis=1)
        active = count >= 3
        dof = numpy.maximum(count - 2, 1)
        total = numpy.where(kept, samples, 0.0).sum(axis=1)
        others_mean = (total[:, None] - samples) / numpy.maximum(count - 1, 1)[:, None]
        centred = numpy.where(kept, samples, numpy.nan)[:, None, :] - others_mean[:, :, None]
        own = numpy.eye(samples.shape[1], dtype=bool)[None, :, :]
        squares = numpy.nansum(numpy.where(own, 0.0, centred) ** 2, axis=2)
        others_sigma = numpy.sqrt(squares / numpy.maximum(count - 2, 1)[:, None])
        scale = others_sigma * numpy.sqrt(count / numpy.maximum(count - 1, 1))[:, None]
        statistic = (samples - others_mean) / scale
        high = stats.t.isf(stats.norm.sf(sigma_high), dof)[:, None]
        low = stats.t.isf(stats.norm.sf(sigma_low), dof)[:, None]
        excess = numpy.where(kept, numpy.maximum(statistic / high, -statistic / low), -numpy.inf)
        worst = excess.argmax(axis=1)
        reject = active & (excess[rows, worst] > 1.0)
        if not reject.any():
            break
        kept[rows[reject], worst[reject]] = False
    return kept


def false_rejection(frames, sigma_low, sigma_high, rng):
    samples = rng.standard_normal((TRIALS, frames))
    kept = deleted_residual_rejections(samples, sigma_low, sigma_high)
    combined = numpy.where(kept, samples, 0.0).sum(axis=1) / kept.sum(axis=1)
    return {
        "frames": frames,
        "sigma_low": sigma_low,
        "sigma_high": sigma_high,
        "rejected_fraction": float(1.0 - kept.mean()),
        "gaussian_tail_fraction": float(stats.norm.sf(sigma_low) + stats.norm.sf(sigma_high)),
        "combined_sigma": float(combined.std()),
        "ideal_sigma": float(frames ** -0.5),
    }


def main():
    output = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUTPUT
    rng = numpy.random.default_rng(SEED)
    document = {
        "generator": "scripts/oracle/sigma_clip_small_sample_oracle.py",
        "tool": f"scipy {scipy.__version__}, numpy {numpy.__version__}, python {platform.python_version()}",
        "definition": "threshold = scipy.stats.t.isf(scipy.stats.norm.sf(sigma), dof): the Student t quantile whose upper tail equals the Gaussian upper tail at sigma",
        "thresholds": [
            {"sigma": sigma, "dof": dof, "threshold": threshold(sigma, dof)}
            for dof in DEGREES_OF_FREEDOM
            for sigma in SIGMAS
        ],
        "monte_carlo": {
            "trials": TRIALS,
            "seed": SEED,
            "cases": [
                false_rejection(frames, sigma_low, sigma_high, rng)
                for sigma_low, sigma_high in ((4.0, 3.0), (2.5, 3.0), (3.0, 3.0))
                for frames in (3, 4, 5)
            ],
        },
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {output}")


if __name__ == "__main__":
    main()
