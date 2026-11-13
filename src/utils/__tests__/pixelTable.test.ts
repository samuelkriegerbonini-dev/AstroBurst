import { describe, it, expect } from "vitest";
import {
  DEFAULT_PIXEL_TABLE_SIZE,
  PIXEL_TABLE_PLANES,
  PIXEL_TABLE_KEY,
  PIXEL_TABLE_SIZES,
  DQ_CELL_GLYPH,
  cellMark,
  cellTone,
  centreSummary,
  columnIndices,
  displayedPlane,
  errorOriginLine,
  formatCell,
  formatPlaneCell,
  layerNotes,
  pixelCellTitle,
  pixelTableCsv,
  planeAvailable,
  rowIndices,
} from "../pixelTable";
import { CSV_LINE_END } from "../catalogCsv";
import { APP_VERSION } from "../constants";
import type { PixelTableResult, PixelTableStats } from "../../shared/types/analysis";
import type { PixelTableLayers } from "../../shared/types/layers";

const STATS: PixelTableStats = { min: 1, max: 9, mean: 4.5, median: 4, n_finite: 6, n_nan: 0 };

function result(overrides: Partial<PixelTableResult & PixelTableLayers> = {}): PixelTableResult & PixelTableLayers {
  return {
    x: 0,
    y: 1,
    size: 3,
    x0: -1,
    y0: 0,
    values: [
      [null, 1, 2],
      [null, 4, 5],
      [null, 8, 9],
    ],
    err: null,
    dq: null,
    dq_names: null,
    dq_table: null,
    unit: null,
    stats: STATS,
    err_stats: null,
    wavelength: null,
    wavelength_stats: null,
    wavelength_unit: null,
    elapsed_ms: 1,
    ...overrides,
  };
}

describe("pixelTableCsv", () => {
  it("writes the column x indices as the header and the y index first on every row, blanks for null cells", () => {
    const lines = pixelTableCsv(result()).split(CSV_LINE_END);
    expect(lines[0]).toBe("y\\x,-1,0,1");
    expect(lines[1]).toBe("0,,1,2");
    expect(lines[2]).toBe("1,,4,5");
    expect(lines[3]).toBe("2,,8,9");
    expect(lines[4]).toBe("");
    expect(lines).toHaveLength(5);
  });

  it("can export a different grid of the same shape, such as the ERR plane", () => {
    const err = [
      [null, 0.5, 1],
      [null, 2, 2.5],
      [null, 4, 4.5],
    ];
    const lines = pixelTableCsv(result({ err }), err).split(CSV_LINE_END);
    expect(lines[1]).toBe("0,,0.5,1");
    expect(lines[3]).toBe("2,,4,4.5");
  });

  it("derives the axis indices from the origin and size", () => {
    expect(columnIndices({ x0: 10, size: 5 })).toEqual([10, 11, 12, 13, 14]);
    expect(rowIndices({ y0: -2, size: 3 })).toEqual([-2, -1, 0]);
  });

  it("labels the corner row axis before the backslash and column axis after it, matching the rows and header it writes", () => {
    const r = result();
    const lines = pixelTableCsv(r).split(CSV_LINE_END);
    const [corner, ...headers] = lines[0].split(",");
    const [rowAxis, colAxis] = corner.split("\\");
    expect(rowAxis).toBe("y");
    expect(colAxis).toBe("x");
    expect(headers).toEqual(columnIndices(r).map(String));
    expect(lines.slice(1, 1 + r.size).map((l) => l.split(",")[0])).toEqual(rowIndices(r).map(String));
  });

  const EXPORTED = "2026-11-12T10:00:00.000Z";
  const ERR_GRID = [
    [null, 0.5, 1],
    [null, 2, 2.5],
    [null, 4, 4.5],
  ];
  const provenanceOf = (csv: string) => csv.split(CSV_LINE_END).filter((line) => line.startsWith("#"));
  const bodyOf = (csv: string) => csv.split(CSV_LINE_END).filter((line) => !line.startsWith("#") && line !== "");

  it("the copied table says which file, plane, unit and pixel it is", () => {
    const csv = pixelTableCsv(
      result({ unit: "MJy/sr", err: ERR_GRID, err_source: "EXTNAME VARIANCE, HDU 3: square root", dq_table: "jwst" }),
      ERR_GRID,
      { file: "a.fits", plane: "ERR", exportedUtc: EXPORTED },
    );
    expect(provenanceOf(csv)).toEqual([
      `# software: AstroBurst ${APP_VERSION}`,
      "# exported_utc: 2026-11-12T10:00:00.000Z",
      "# file: a.fits",
      "# plane: ERR",
      "# centre_x: 0",
      "# centre_y: 1",
      "# pixel_coordinates: 0-based, integer pixel centres; rows are y, columns are x",
      "# err_source: EXTNAME VARIANCE, HDU 3: square root",
      "# dq_table: jwst",
      "# unit -1: MJy/sr",
      "# unit 0: MJy/sr",
      "# unit 1: MJy/sr",
    ]);
    expect(bodyOf(csv)).toEqual(["y\\x,-1,0,1","0,,0.5,1", "1,,2,2.5", "2,,4,4.5"]);
  });

  it("leaves out what is not known and keeps the bare grid without a context", () => {
    const bare = provenanceOf(pixelTableCsv(result(), undefined, { file: null, plane: "SCI", exportedUtc: EXPORTED }));
    expect(bare[2]).toBe("# file:");
    expect(bare.some((line) => line.startsWith("# err_source") || line.startsWith("# dq_table") || line.startsWith("# unit"))).toBe(false);
    const sci = provenanceOf(pixelTableCsv(result({ err_source: "EXTNAME ERR, HDU 2: as is" }), undefined, { file: "a.fits", plane: "SCI", exportedUtc: EXPORTED }));
    expect(sci.some((line) => line.startsWith("# err_source"))).toBe(false);
    const wave = [
      [null, 1.5, 1.6],
      [null, 1.5, 1.6],
      [null, 1.5, 1.6],
    ];
    const lambda = provenanceOf(
      pixelTableCsv(result({ unit: "MJy/sr", wavelength: wave, wavelength_unit: "um" }), wave, { file: "a.fits", plane: "WAVELENGTH", exportedUtc: EXPORTED }),
    );
    expect(lambda.filter((line) => line.startsWith("# unit "))).toEqual(["# unit -1: um", "# unit 0: um", "# unit 1: um"]);
  });
});

describe("PIXEL_TABLE_SIZES", () => {
  it("offers every odd grid size the pixel_table_cmd boundary accepts (3 to 15)", () => {
    const backendAccepted = Array.from({ length: 7 }, (_, i) => 3 + 2 * i);
    expect([...PIXEL_TABLE_SIZES]).toEqual(backendAccepted);
    expect(PIXEL_TABLE_SIZES).toContain(DEFAULT_PIXEL_TABLE_SIZE);
  });
});

describe("displayedPlane", () => {
  const ERR_STATS: PixelTableStats = { min: 0, max: 4.5, mean: 2.25, median: 2.25, n_finite: 6, n_nan: 0 };
  const ERR = [
    [null, 0, 0.5],
    [null, 2, 2.5],
    [null, 4, 4.5],
  ];

  const WAVE_STATS: PixelTableStats = { min: 1.654, max: 1.66, mean: 1.657, median: 1.657, n_finite: 6, n_nan: 0 };
  const WAVE = [
    [null, 1.654, 1.655],
    [null, 1.656, 1.657],
    [null, 1.659, 1.66],
  ];

  it("shows the ERR grid with the ERR plane's own stats when ERR is requested and present", () => {
    const r = result({ err: ERR, err_stats: ERR_STATS });
    expect(displayedPlane(r, "ERR")).toEqual({ plane: "ERR", grid: ERR, stats: ERR_STATS });
  });

  it("shows the science grid and science stats when SCI is requested or the image has no ERR plane", () => {
    const r = result({ err: ERR, err_stats: ERR_STATS });
    expect(displayedPlane(r, "SCI")).toEqual({ plane: "SCI", grid: r.values, stats: STATS });
    const plain = result();
    expect(displayedPlane(plain, "ERR")).toEqual({ plane: "SCI", grid: plain.values, stats: STATS });
  });

  it("shows the WAVELENGTH grid with its own stats when requested and present", () => {
    const r = result({ err: ERR, err_stats: ERR_STATS, wavelength: WAVE, wavelength_stats: WAVE_STATS, wavelength_unit: "um" });
    expect(displayedPlane(r, "WAVELENGTH")).toEqual({ plane: "WAVELENGTH", grid: WAVE, stats: WAVE_STATS });
  });

  it("falls back to the science grid when the WAVELENGTH plane is missing", () => {
    const plain = result({ err: ERR, err_stats: ERR_STATS });
    expect(displayedPlane(plain, "WAVELENGTH")).toEqual({ plane: "SCI", grid: plain.values, stats: STATS });
  });
});

describe("planeAvailable", () => {
  it("offers SCI always, ERR and WAVELENGTH only when their grid and stats came back", () => {
    const plain = result();
    expect(planeAvailable(plain, "SCI")).toBe(true);
    expect(planeAvailable(plain, "ERR")).toBe(false);
    expect(planeAvailable(plain, "WAVELENGTH")).toBe(false);
    const full = result({
      err: [[1]],
      err_stats: STATS,
      wavelength: [[2]],
      wavelength_stats: STATS,
      wavelength_unit: "um",
    });
    expect(PIXEL_TABLE_PLANES.map((p) => planeAvailable(full, p))).toEqual([true, true, true]);
    expect(planeAvailable(result({ wavelength: [[2]], wavelength_stats: null }), "WAVELENGTH")).toBe(false);
  });

  it("lists the planes in the order of the plane selector", () => {
    expect([...PIXEL_TABLE_PLANES]).toEqual(["SCI", "ERR", "WAVELENGTH"]);
  });
});

describe("pixelCellTitle", () => {
  const units = { unit: "MJy/sr", wavelength_unit: "um" };

  it("adds a λ line with the wavelength unit when the WAVELENGTH plane came back", () => {
    expect(pixelCellTitle(3, 4, { value: 2.5, err: 0.1, wavelength: 1.65432, dqNames: "SATURATED" }, units, 4)).toBe(
      "(3, 4): 2.5000 MJy/sr\nERR 0.1000\nλ 1.65432 um\nDQ SATURATED",
    );
  });

  it("writes λ -- on a NaN wavelength pixel and leaves the line out without the plane", () => {
    expect(pixelCellTitle(0, 0, { value: 1, err: undefined, wavelength: null, dqNames: null }, units, 4)).toBe("(0, 0): 1.0000 MJy/sr\nλ --");
    expect(pixelCellTitle(0, 0, { value: 1, err: undefined, wavelength: undefined, dqNames: null }, { unit: null, wavelength_unit: null }, 4)).toBe(
      "(0, 0): 1.0000",
    );
  });
});

describe("formatCell", () => {
  it("uses fixed notation inside [1e-3, 1e5) and exponential outside", () => {
    expect(formatCell(12.3456, 2)).toBe("12.35");
    expect(formatCell(0.001, 3)).toBe("0.00100");
    expect(formatCell(99999.5, 1)).toBe("99999.5");
    expect(formatCell(1e5, 2)).toBe("1.0e+5");
    expect(formatCell(0.0005, 2)).toBe("5.0e-4");
    expect(formatCell(-2.5e7, 3)).toBe("-2.50e+7");
    expect(formatCell(-0.5, 2)).toBe("-0.50");
  });

  it("keeps at least the requested significant figures inside the fixed range, matching the exponential branch below 1e-3", () => {
    expect(formatCell(0.00149, 3)).toBe("0.00149");
    expect(formatCell(0.00101, 3)).toBe("0.00101");
    expect(formatCell(0.00449, 3)).toBe("0.00449");
    expect(formatCell(0.0123, 3)).toBe("0.0123");
    expect(formatCell(-0.00149, 3)).toBe("-0.00149");
    expect(formatCell(0.00099, 3)).toBe("9.90e-4");
    expect(formatCell(12.3456, 3)).toBe("12.346");
    expect(formatCell(0.00149, 4)).toBe("0.001490");
  });

  it("prints zero in fixed notation and dashes for null or non-finite values", () => {
    expect(formatCell(0, 2)).toBe("0.00");
    expect(formatCell(null, 2)).toBe("--");
    expect(formatCell(Number.NaN, 2)).toBe("--");
    expect(formatCell(Number.POSITIVE_INFINITY, 2)).toBe("--");
  });
});

describe("formatPlaneCell", () => {
  it("prints WAVELENGTH cells with six significant digits so adjacent pixels differ in every NIRSpec grating", () => {
    const g140h = [0.97012, 0.97035, 0.97058].map((v) => formatPlaneCell(v, "WAVELENGTH"));
    expect(g140h).toEqual(["0.970120", "0.970350", "0.970580"]);
    const g235h = [1.654, 1.65438, 1.65476, 1.65514].map((v) => formatPlaneCell(v, "WAVELENGTH"));
    expect(g235h).toEqual(["1.65400", "1.65438", "1.65476", "1.65514"]);
    const fine = [1.654, 1.65405, 1.6541].map((v) => formatPlaneCell(v, "WAVELENGTH"));
    expect(new Set(fine).size).toBe(3);
    expect(formatPlaneCell(12.34567, "WAVELENGTH")).toBe("12.3457");
    expect(formatPlaneCell(null, "WAVELENGTH")).toBe("--");
    expect(formatPlaneCell(Number.NaN, "WAVELENGTH")).toBe("--");
  });

  it("keeps the per-pixel step readable whatever unit the WAVELENGTH plane is in", () => {
    expect([1654.38, 1654.76].map((v) => formatPlaneCell(v, "WAVELENGTH"))).toEqual(["1654.38", "1654.76"]);
    expect([16543.8, 16547.6].map((v) => formatPlaneCell(v, "WAVELENGTH"))).toEqual(["16543.8", "16547.6"]);
    expect([1.65438e-6, 1.65476e-6].map((v) => formatPlaneCell(v, "WAVELENGTH"))).toEqual(["1.65438e-6", "1.65476e-6"]);
  });

  it("keeps three decimals for SCI and ERR cells and takes the summary digits only for those planes", () => {
    expect(formatPlaneCell(1.65438, "SCI")).toBe("1.654");
    expect(formatPlaneCell(1.65438, "ERR")).toBe("1.654");
    expect(formatPlaneCell(1.65438, "SCI", 4)).toBe("1.6544");
    expect(formatPlaneCell(1.65438, "WAVELENGTH", 4)).toBe("1.65438");
  });
});

describe("cellTone", () => {
  it("flags DQ before anything else, then null, max and min", () => {
    expect(cellTone(9, STATS, "SATURATED")).toBe("dq");
    expect(cellTone(null, STATS, "DO_NOT_USE")).toBe("dq");
    expect(cellTone(null, STATS, null)).toBe("nan");
    expect(cellTone(9, STATS, null)).toBe("max");
    expect(cellTone(1, STATS, null)).toBe("min");
    expect(cellTone(4, STATS, null)).toBe("normal");
  });

  it("treats a constant grid as all max and an empty grid as plain", () => {
    const flat: PixelTableStats = { ...STATS, min: 3, max: 3 };
    expect(cellTone(3, flat, null)).toBe("max");
    const empty: PixelTableStats = { min: null, max: null, mean: null, median: null, n_finite: 0, n_nan: 9 };
    expect(cellTone(2, empty, null)).toBe("normal");
  });
});

describe("errorOriginLine", () => {
  it("errorOriginLine hides the untouched error plane of a pipeline product and shows every other origin", () => {
    for (const hidden of [null, undefined, "", "EXTNAME ERR, HDU 2: as is", "ASDF array err, roman.err: as is"]) {
      expect(errorOriginLine(hidden), String(hidden)).toBeNull();
    }
    expect(errorOriginLine("EXTNAME VARIANCE, HDU 3: square root")).toBe("± from EXTNAME VARIANCE, HDU 3: square root");
    for (const shown of [
      "EXTNAME RMS, HDU 2: as is",
      "EXTNAME WGT, HDU 3: 1 / square root, assumed inverse variance",
      "file name -RMS, EUC_MER_MOSAIC-VIS-RMS_TILE1-BBB_t2_00.00.fits: as is",
      "user assignment, wgt.fits: 1 / square root",
      "SDSS frame recipe (gain 4.76 e-/DN, dark variance 1.3225 DN^2, typed by the user): rebuilt from the calibration vector and the sky table",
    ]) {
      expect(errorOriginLine(shown)).toBe(`± from ${shown}`);
    }
  });
});

describe("cellMark", () => {
  it("cellMark gives a flagged cell and the clicked cell a mark that is not a colour", () => {
    expect(cellMark("dq", "SATURATED | JUMP_DET", false)).toEqual({ glyph: "*", srText: "DQ: SATURATED | JUMP_DET" });
    expect(cellMark("dq", "SATURATED | JUMP_DET", true)).toEqual({ glyph: "*", srText: "DQ: SATURATED | JUMP_DET, clicked pixel" });
    expect(cellMark("normal", null, true)).toEqual({ glyph: null, srText: "clicked pixel" });
    for (const tone of ["max", "min", "nan", "normal"] as const) {
      expect(cellMark(tone, null, false), tone).toEqual({ glyph: null, srText: null });
    }
    expect(DQ_CELL_GLYPH).toBe("*");
  });
});

describe("PIXEL_TABLE_KEY", () => {
  it("the key names the four tints in the order of their priority", () => {
    expect(PIXEL_TABLE_KEY).toEqual([
      { tone: "dq", label: "DQ flagged *" },
      { tone: "max", label: "max" },
      { tone: "min", label: "min" },
      { tone: "nan", label: "no value" },
    ]);
  });
});

describe("centreSummary", () => {
  it("centreSummary names the clicked pixel and its flags", () => {
    expect(centreSummary(result())).toBe("clicked (0, 1)");
    const dq_names = [
      [null, null, null],
      [null, "SATURATED", null],
      [null, null, null],
    ];
    expect(centreSummary(result({ dq_names }))).toBe("clicked (0, 1): DQ SATURATED");
  });
});

describe("layerNotes", () => {
  it("layerNotes says where the error comes from and which layer was not used", () => {
    expect(layerNotes({ err_source: "EXTNAME VARIANCE, HDU 3: square root", dq_source: "MP_ cards (afw mask planes)" })).toEqual({
      origin: "ERR EXTNAME VARIANCE, HDU 3: square root",
      gaps: [],
    });
    expect(
      layerNotes({
        err_unavailable: "error layer HDU 2 (VARIANCE) is 2x2 and does not match the image 4x4: not used",
        dq_unavailable: "mask layer HDU 2 is a floating-point plane (BITPIX -32): flag bits cannot be read exactly",
      }),
    ).toEqual({
      origin: null,
      gaps: [
        "ERR not used: error layer HDU 2 (VARIANCE) is 2x2 and does not match the image 4x4: not used",
        "DQ not used: mask layer HDU 2 is a floating-point plane (BITPIX -32): flag bits cannot be read exactly",
      ],
    });
    expect(layerNotes({})).toEqual({ origin: null, gaps: [] });
  });
});
