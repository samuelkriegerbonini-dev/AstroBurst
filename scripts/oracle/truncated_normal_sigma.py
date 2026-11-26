import importlib.metadata

from scipy.stats import truncnorm

CREATED_BY = "scripts/oracle/truncated_normal_sigma.py"
CLIPS = [2.0, 2.5, 3.0, 4.0]
K_SIGMA = 3.0


def truncated_sigma(clip):
    return float(truncnorm(-clip, clip).std())


def clipping_fixed_point(k):
    sigma = 1.0
    for _ in range(500):
        following = truncated_sigma(k * sigma)
        if abs(following - sigma) < 1e-14:
            return following
        sigma = following
    return sigma


def main():
    print(f"{CREATED_BY}, scipy {importlib.metadata.version('scipy')}")
    for clip in CLIPS:
        print(f"standard deviation of a unit normal truncated at +-{clip}: {truncated_sigma(clip):.9f}")
    print(f"fixed point of repeated clipping at {K_SIGMA} sigma: {clipping_fixed_point(K_SIGMA):.9f}")


if __name__ == "__main__":
    main()
