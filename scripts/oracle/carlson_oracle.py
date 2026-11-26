import scipy
from scipy import special

CASES = [
    ("RF(0, 0.5, 1)", special.elliprf, (0.0, 0.5, 1.0)),
    ("RD(0, 0.5, 1)", special.elliprd, (0.0, 0.5, 1.0)),
    ("RJ(0, 0.5, 1, 2)", special.elliprj, (0.0, 0.5, 1.0, 2.0)),
    ("RJ(2, 3, 4, 5)", special.elliprj, (2.0, 3.0, 4.0, 5.0)),
    ("RF(1, 2, 4)", special.elliprf, (1.0, 2.0, 4.0)),
    ("RD(0, 2, 1)", special.elliprd, (0.0, 2.0, 1.0)),
    ("RJ(0, 1e-6, 1, 3.5)", special.elliprj, (0.0, 1e-6, 1.0, 3.5)),
    ("RJ(0, 0.9, 1, 1.0001)", special.elliprj, (0.0, 0.9, 1.0, 1.0001)),
]


def main():
    print("scipy", scipy.__version__, "(scipy.special.elliprf, elliprd, elliprj)")
    for label, function, arguments in CASES:
        print(label, repr(float(function(*arguments))))


main()
