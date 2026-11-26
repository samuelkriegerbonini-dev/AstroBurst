import math
import sys

import astropy
import numpy as np
from astropy.timeseries import LombScargle
from astropy.timeseries.periodograms.lombscargle import _statistics

SAMPLES_PER_PEAK = 5.0
NYQUIST_FACTOR = 5.0
MAX_FREQUENCIES = 200000
MAX_WORK = 2000000000


def uniform_deviates(count):
    state = 12345
    out = []
    for _ in range(count):
        state = (1103515245 * state + 12345) % 2147483648
        out.append(state / 2147483648.0)
    return out


def series_a():
    time = np.array([0.37 * i + 0.11 * math.sin(1.3 * i) for i in range(40)])
    value = np.array([1.5 + 0.8 * math.sin(2.0 * math.pi * t / 2.9 + 0.4) + 0.3 * math.cos(2.0 * math.pi * t / 1.1) for t in time])
    err = np.array([0.1 + 0.0125 * ((7 * i) % 5) for i in range(40)])
    return time, value, err


def series_b():
    u = uniform_deviates(48)
    time = np.array([0.5 * i + 0.4 * u[i] for i in range(24)])
    value = np.array([10.0 + 0.3 * math.sin(2.0 * math.pi * time[i] / 3.3) + (u[i + 24] - 0.5) for i in range(24)])
    return time, value


def trend_series():
    u = uniform_deviates(200)
    time = np.array([10.0 * i / 199.0 for i in range(200)])
    value = np.array([5.0 + 0.3 * time[i] + 0.02 * (u[i] - 0.5) for i in range(200)])
    return time, value


def scargle(time, value, err=None):
    return LombScargle(time, value, err, fit_mean=True, center_data=True, normalization="standard")


def power_at(time, value, err, frequency):
    return scargle(time, value, err).power(np.asarray(frequency, "f8"), method="cython")


def baluev(time, err, power, maximum_frequency):
    return float(_statistics.fap_baluev(power, maximum_frequency, time - time.min(), None, err, "standard"))


def default_grid(time, last=None):
    baseline = float(time.max() - time.min())
    step = 1.0 / (SAMPLES_PER_PEAK * baseline)
    wanted_last = NYQUIST_FACTOR * 0.5 * len(time) / baseline if last is None else last
    count = int(math.floor((wanted_last - step) / step + 1e-9)) + 1
    return step + step * np.arange(count), baseline, step


def show(label, values):
    print(label, [repr(float(v)) for v in np.atleast_1d(values)])


def search(label, time, value, err, last=None):
    frequency, baseline, step = default_grid(time, last)
    power = power_at(time, value, err, frequency)
    best = int(np.argmax(power))
    fap = baluev(time, err, power[best], frequency[-1])
    print(label, "baseline", repr(baseline), "step", repr(step), "count", len(frequency), "last", repr(float(frequency[-1])))
    print("   best index", best, "frequency", repr(float(frequency[best])), "period", repr(float(1.0 / frequency[best])), "power", repr(float(power[best])), "baluev FAP", repr(float(fap)))
    print("   power[0]", repr(float(power[0])), "power[last]", repr(float(power[-1])))
    return frequency, power, best, baseline


def cmd_constants():
    time_a, value_a, err_a = series_a()
    time_b, value_b = series_b()
    six = [1.0 / 2.9, 1.0 / 1.1, 0.05, 0.5, 1.0, 1.3]
    four = [1.0 / 3.3, 0.1, 0.7, 1.5]
    print("series A checkpoints t[1], t[39], y[0], y[39]", repr(float(time_a[1])), repr(float(time_a[39])), repr(float(value_a[0])), repr(float(value_a[39])))
    print("series B checkpoints t[0], t[23], y[0], y[23]", repr(float(time_b[0])), repr(float(time_b[23])), repr(float(value_b[0])), repr(float(value_b[23])))
    show("ASTROPY_WEIGHTED_A", power_at(time_a, value_a, err_a, six))
    show("ASTROPY_UNWEIGHTED_A", power_at(time_a, value_a, None, six))
    show("ASTROPY_UNWEIGHTED_B", power_at(time_b, value_b, None, four))
    search("default search, series A weighted:", time_a, value_a, err_a)
    search("default search, series A unweighted:", time_a, value_a, None)
    search("default search, series B unweighted:", time_b, value_b, None)
    last_a = default_grid(time_a)[0][-1]
    fap = baluev(time_a, None, 0.3, last_a)
    print("baluev FAP at power 0.3, series A unweighted, maximum frequency", repr(float(last_a)), ":", repr(float(fap)))
    user = [0.2 + 0.01 * k for k in range(31)]
    user_power = power_at(time_a, value_a, err_a, user)
    print("user grid 0.2 + 0.01 k, series A weighted: highest index", int(np.argmax(user_power)), "power", repr(float(user_power.max())))
    sinusoid = np.array([3.0 + 2.0 * math.sin(2.0 * math.pi * 0.4 * t + 1.0) for t in time_a])
    show("pure sinusoid of 0.4 cycles per day on the times of series A, power at 0.4 and 0.2", power_at(time_a, sinusoid, None, [0.4, 0.2]))
    even = np.arange(16.0)
    alternating = np.array([4.0 if k % 2 == 0 else 2.0 for k in range(16)])
    show("even sampling, alternating values, power at 0.1", power_at(even, alternating, None, [0.1]))


def cmd_trend():
    time, value = trend_series()
    cadence = 10.0 / 199.0
    frequency, power, best, baseline = search("trend series (200 points over 10 d, 5 + 0.3 t + 0.02 (u - 0.5)), grid up to the Nyquist frequency of the cadence:", time, value, None, last=0.5 / cadence)
    print("   best period over baseline", repr(float(1.0 / frequency[best] / baseline)))
    slope, level = np.polyfit(time, value, 1)
    detrended = value - (level + slope * time)
    detrended_power = power_at(time, detrended, None, frequency)
    print("   after removing the fitted line: highest power", repr(float(detrended_power.max())), "at period", repr(float(1.0 / frequency[int(np.argmax(detrended_power))])))


def cmd_capped():
    points, baseline = 100000, 100.0
    step = 1.0 / (SAMPLES_PER_PEAK * baseline)
    cadence = baseline / (points - 1)
    last = 0.5 / cadence
    wanted = int(math.floor((last - step) / step + 1e-9)) + 1
    limit = min(MAX_FREQUENCIES, MAX_WORK // points)
    widened = (last - step) / (limit - 1)
    print("capped grid:", points, "points on a regular cadence over", baseline, "d: wanted", wanted, "frequencies, limit", limit, "widened step", repr(widened), "samples per peak", repr(1.0 / (baseline * widened)))
    half_step_loss = (math.sin(math.pi * baseline * widened / 2.0) / (math.pi * baseline * widened / 2.0)) ** 2
    print("   power kept half a widened step from a peak (sinc^2 of pi T step / 2):", repr(half_step_loss))
    rng = np.random.default_rng(20261125)
    time = cadence * np.arange(points)
    noise = rng.normal(0.0, 0.002, points)
    target = step + 1000.5 * widened
    grid = step + widened * np.arange(limit)
    for amplitude in (0.01, 0.0002):
        value = 1.0 + amplitude * np.sin(2.0 * math.pi * target * time) + noise
        true_power = float(power_at(time, value, None, [target])[0])
        near = power_at(time, value, None, grid[995:1006])
        coarse = scargle(time, value).power(grid, method="fast")
        best = int(np.argmax(coarse))
        print("   amplitude", amplitude, "at", repr(target), "cycles per day: true power", repr(true_power), "highest power on the widened grid near it", repr(float(near.max())), "| best of the whole widened grid", repr(float(grid[best])), "power", repr(float(coarse[best])))


def main():
    print("astropy", astropy.__version__, "numpy", np.__version__)
    what = sys.argv[1] if len(sys.argv) > 1 else "constants"
    if what == "constants":
        cmd_constants()
    elif what == "trend":
        cmd_trend()
    elif what == "capped":
        cmd_capped()
    else:
        print("usage: periodogram_oracle.py constants|trend|capped")


main()
