import json
import math
import random
import sys
from decimal import ROUND_HALF_UP, Decimal, getcontext

getcontext().prec = 1200
PLUS_MINUS = chr(0xB1)
NAN = float("nan")
INF = float("inf")
MAX_SPAN = 12


def to_fixed(x, digits):
    exact = Decimal(x)
    text = format(exact.copy_abs().quantize(Decimal(1).scaleb(-digits), rounding=ROUND_HALF_UP), "f")
    return ("-" if exact < 0 and any(c in "123456789" for c in text) else "") + text


def to_exponential(x, digits):
    exact = Decimal(x)
    magnitude = exact.copy_abs()
    exponent = magnitude.adjusted()
    rounded = magnitude.quantize(Decimal(1).scaleb(exponent - digits), rounding=ROUND_HALF_UP)
    if rounded.adjusted() > exponent:
        exponent += 1
        rounded = magnitude.quantize(Decimal(1).scaleb(exponent - digits), rounding=ROUND_HALF_UP)
    mantissa = format(rounded.scaleb(-exponent).quantize(Decimal(1).scaleb(-digits)), "f")
    return ("-" if exact < 0 else "") + mantissa + "e" + ("+" if exponent >= 0 else "-") + str(abs(exponent))


def exponent_of(x, digits):
    return int(to_exponential(x, digits).split("e")[1])


def shortest_exponent(x):
    return Decimal(repr(abs(x))).adjusted()


def shift(x, places):
    if x == 0:
        return 0.0
    mantissa, exponent = to_exponential(x, 16).split("e")
    return float(mantissa + "e" + str(int(exponent) + places))


def four_significant(value):
    if value == 0:
        return "0"
    magnitude = abs(value)
    if 1e-3 <= magnitude < 1e3:
        return to_fixed(value, 3 - exponent_of(value, 3))
    if 1e3 <= magnitude < 1e5:
        return to_fixed(value, 0)
    return to_exponential(value, 3).replace("e+", "e")


def usable(err):
    return err is not None and math.isfinite(err) and err > 0


def with_uncertainty(value, err, unit=None, err_digits=2):
    if value is None or not math.isfinite(value):
        return "--"
    digits = 1 if err_digits == 1 else 2
    if not usable(err):
        body = four_significant(value)
    else:
        err_exponent = exponent_of(err, digits - 1)
        decimals = digits - 1 - err_exponent
        largest = max(abs(value), err)
        top_exponent = max(shortest_exponent(largest), err_exponent)
        if top_exponent - err_exponent > MAX_SPAN:
            body = four_significant(value) + " " + PLUS_MINUS + " " + to_exponential(err, digits - 1).replace("e+", "e")
        elif 1e-3 <= largest < 1e5:
            if decimals >= 0:
                body = to_fixed(value, decimals) + " " + PLUS_MINUS + " " + to_fixed(err, decimals)
            else:
                def coarse(x):
                    text = to_fixed(shift(x, decimals), 0)
                    return text if text == "0" else text + "0" * (-decimals)
                body = coarse(value) + " " + PLUS_MINUS + " " + coarse(err)
        else:
            places = decimals + top_exponent
            body = "(" + to_fixed(shift(value, -top_exponent), places) + " " + PLUS_MINUS + " " + to_fixed(shift(err, -top_exponent), places) + ")e" + str(top_exponent)
    return body + (" " + unit if unit else "")


def flux_density_jy(value, err):
    if value is None or not math.isfinite(value):
        return "--"
    reference = abs(value)
    if reference == 0 and usable(err):
        reference = err
    if reference >= 1 or reference == 0:
        places, unit = 0, "Jy"
    elif reference >= 1e-3:
        places, unit = 3, "mJy"
    elif reference >= 1e-6:
        places, unit = 6, "uJy"
    else:
        places, unit = 9, "nJy"
    return with_uncertainty(shift(value, places), shift(err, places) if usable(err) else None, unit)


WITH_UNCERTAINTY = [
    (1.23456, 0.0123, None, 2), (1.23456, 0.0123, "mag", 2), (1.23456, 0.0123, None, 1), (12.3456, 0.5, None, 2),
    (1234.56, 12.3, "DN", 2), (56789.0, 1234.0, None, 2), (56789.0, 1234.0, None, 1), (12.0, 1234.0, None, 2), (0.5, 120.0, None, 2),
    (-56789.0, 1234.0, None, 2), (-12.0, 1234.0, None, 2),
    (1.2341e-17, 1.2e-19, None, 2), (1.2341e-17, 1.2e-19, "erg s-1 cm-2", 2), (-1.2341e-17, 1.2e-19, None, 2), (2e-19, 1.5e-17, None, 2), (1234567.0, 2345.0, None, 2),
    (123456.7, 0.5, None, 2), (99999.0, 0.6, None, 2), (0.0996, 0.0996, None, 2), (5.0, 0.0996, None, 2), (5.0, 0.0996, None, 1),
    (5.0, 0.96, None, 1), (-0.0004, 0.01, None, 2), (-3.14159, 0.002, None, 2), (0.0, 0.05, None, 2), (0.125, 0.01, None, 1),
    (0.000123, 0.000045, None, 2), (0.00123, 0.00045, None, 2), (20.5, 0.25, None, 2), (1e-3, 1e-4, None, 2), (9.9996e4, 12.0, None, 2),
    (100000.0, 12.0, None, 2), (99999.996, 0.6, None, 2), (999999.9999, 1200.0, None, 2), (3.0e4, 9.6e5, None, 1), (3.0e4, 9.96e5, None, 2),
    (24.51234, 0.2512, None, 2), (1234.5678, 12.3, None, 2), (10.25, 0.1414, None, 2), (3.0, 0.3, None, 2),
    (1.0, 1e-150, None, 2), (1.0, 5e-324, None, 2), (123456.7, 1.5e-8, "Jy", 2), (1.0, 1e-12, None, 2), (1.0, 1e-13, None, 2), (5e-324, 5e-324, None, 2), (1.7e308, 1.2e306, None, 2),
    (3.0, None, None, 2), (12.3456, None, None, 2), (12.3, 0.0, None, 2), (0.0012345, NAN, None, 2),
    (1234.5, None, None, 2), (12345.6, -1.0, None, 2), (123456.7, None, "Jy", 2), (1.2e-17, None, None, 2), (0.0, None, None, 2),
    (-0.00012345, None, None, 2), (999.96, None, None, 2), (-2048.5, INF, None, 2), (0.00099996, None, None, 2), (99999.5, None, None, 2), (-0.0, None, None, 2),
    (None, 0.1, None, 2), (NAN, 0.1, "Jy", 2), (INF, 0.1, None, 2), (-INF, None, "Jy", 2),
]
FLUX_DENSITY_JY = [
    (1.5, 0.02), (0.0123, 0.00099), (1.23e-5, 9.9e-7), (1.23e-5, 9.9e-10), (4.2e-9, 1.1e-9), (3.3e-13, 2.0e-14), (1500.0, 20.0),
    (2.5e6, 1.2e4), (0.0, 2e-6), (-3e-6, 2e-6), (0.00123, None), (None, 1e-3), (1e-3, 1e-5), (0.999e-3, 1e-5), (1.0, 0.01), (0.0, None),
    (1e-6, 1e-8), (0.999e-6, 1e-8), (-0.0123, 0.00099), (NAN, None), (2.0e-5, NAN), (0.0, 0.0),
]


def shown(x):
    if x is None:
        return None
    if isinstance(x, float) and not math.isfinite(x):
        return "NaN" if math.isnan(x) else ("Infinity" if x > 0 else "-Infinity")
    if isinstance(x, float) and x == 0 and math.copysign(1.0, x) < 0:
        return "-0"
    return x


def random_number(rng):
    kind = rng.random()
    if kind < 0.05:
        return 0.0
    mantissa = rng.choice([rng.uniform(1, 10), float(rng.randint(1, 9999)) / rng.choice([1, 10, 100, 1000]), rng.choice([0.5, 0.25, 0.125, 9.5, 9.96, 9.996, 99.5, 1.5, 2.5])])
    exponent = rng.randint(-30, 30) if kind < 0.3 else rng.randint(-6, 7)
    sign = -1.0 if rng.random() < 0.2 else 1.0
    return sign * float("%.17ge%d" % (mantissa, exponent))


def fuzz(count, seed):
    rng = random.Random(seed)
    for _ in range(count):
        value = random_number(rng)
        err = abs(random_number(rng)) if rng.random() < 0.9 else None
        digits = 1 if rng.random() < 0.3 else 2
        print(json.dumps(["u", value, err, digits, with_uncertainty(value, err, None, digits)], ensure_ascii=True))
        print(json.dumps(["j", value, err, flux_density_jy(value, err)], ensure_ascii=True))


if len(sys.argv) > 2 and sys.argv[1] == "fuzz":
    fuzz(int(sys.argv[2]), int(sys.argv[3]) if len(sys.argv) > 3 else 1)
else:
    for value, err, unit, digits in WITH_UNCERTAINTY:
        print(json.dumps([shown(value), shown(err), unit, digits, with_uncertainty(value, err, unit, digits)], ensure_ascii=True))
    for value, err in FLUX_DENSITY_JY:
        print(json.dumps([shown(value), shown(err), flux_density_jy(value, err)], ensure_ascii=True))
