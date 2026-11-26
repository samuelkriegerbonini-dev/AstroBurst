import json
import struct
import sys
from pathlib import Path

import astropy
from astropy.io import fits

ROOT = Path(__file__).resolve().parents[2]
GOLDEN = ROOT / "src-tauri/tests/fixtures/fitsio/astropy_fitsio_goldens.json"


def bits(value):
    return struct.pack(">d", float(value)).hex()


def main(folder):
    rows = json.loads(GOLDEN.read_text(encoding="ascii"))["float_cards"]
    path = Path(folder) / "float_cards.fits"
    with fits.open(path) as hdus:
        hdus.verify("exception")
        header = hdus[0].header
        failures = []
        for index, row in enumerate(rows):
            key = f"ABFLT{index:03d}"
            card = header.cards[key]
            read = bits(card.value)
            layout = "fixed" if len(card.image[10:30].strip()) == len(str(card.image[10:]).split("/")[0].strip()) else "free"
            print(f"{key} {card.image.rstrip()!r:60} {layout:5} astropy reads {read} expected {row['bits']}")
            if read != row["bits"] or not isinstance(card.value, float):
                failures.append(key)
    print(f"astropy {astropy.__version__}: {len(rows) - len(failures)} of {len(rows)} cards read back bit for bit; verify('exception') passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
