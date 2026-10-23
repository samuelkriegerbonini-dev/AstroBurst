import { describe, it, expect } from "vitest";
import {
  fieldOfViewText,
  formatLat,
  formatLon,
  frameAxisLabels,
  frameLonInHours,
  pixelScaleText,
  skyPositionLine,
  solveCentreText,
  solveSummaryLine,
} from "../coordFormat";
import { fieldText } from "../../components/archive/logic/quickWcs";
import type { MastQuickWcs } from "../../shared/types/mast";

const GC_RA = 266.4049882865447;
const GC_DEC = -28.936177761791473;
const EDGE_RA = 15 * (5 + 35 / 60 + 59.9996 / 3600);
const EDGE_DEC = -(5 + 23 / 60 + 59.96 / 3600);

describe("formatLon", () => {
  it("formats hours sexagesimal", () => {
    expect(formatLon(GC_RA, { hours: true, format: "sexagesimal" })).toBe("17h45m37.20s");
  });
  it("formats degrees sexagesimal", () => {
    expect(formatLon(GC_RA, { hours: false, format: "sexagesimal" })).toBe("266°24'18.0\"");
  });
  it("formats decimal degrees", () => {
    expect(formatLon(GC_RA, { hours: false, format: "decimal" })).toBe("266.404988°");
    expect(formatLon(GC_RA, { hours: true, format: "decimal" })).toBe("266.404988°");
  });
  it("wraps longitude into [0,360)", () => {
    expect(formatLon(-90, { hours: true, format: "sexagesimal" })).toBe("18h00m00.00s");
    expect(formatLon(370, { hours: false, format: "decimal" })).toBe("10.000000°");
  });
  it("carries seconds that round to 60 instead of printing 60.00s", () => {
    const deg = (1 + 59 / 60 + 59.999 / 3600) * 15;
    expect(formatLon(deg, { hours: true, format: "sexagesimal" })).toBe("02h00m00.00s");
    const degDeg = 45 + 59 / 60 + 59.99 / 3600;
    expect(formatLon(degDeg, { hours: false, format: "sexagesimal" })).toBe("46°00'00.0\"");
  });
  it("honours precision", () => {
    expect(formatLon(GC_RA, { hours: true, format: "sexagesimal", precision: 0 })).toBe("17h45m37s");
    expect(formatLon(GC_RA, { hours: false, format: "decimal", precision: 2 })).toBe("266.40°");
  });
  it("returns a dash for non-finite input", () => {
    expect(formatLon(NaN, { hours: true, format: "sexagesimal" })).toBe("—");
  });
});

describe("formatLat", () => {
  it("formats negative sexagesimal", () => {
    expect(formatLat(GC_DEC, { format: "sexagesimal" })).toBe("-28°56'10.2\"");
  });
  it("formats positive sexagesimal with a plus sign and zero padding", () => {
    expect(formatLat(27.12825, { format: "sexagesimal" })).toBe("+27°07'41.7\"");
  });
  it("formats decimal", () => {
    expect(formatLat(GC_DEC, { format: "decimal" })).toBe("-28.936178°");
    expect(formatLat(27.12825, { format: "decimal" })).toBe("+27.128250°");
  });
  it("keeps the sign of a tiny negative declination", () => {
    expect(formatLat(-0.0001, { format: "sexagesimal" }).startsWith("-")).toBe(true);
    expect(formatLat(-0.0001, { format: "decimal" }).startsWith("-")).toBe(true);
  });
  it("carries arcseconds that round to 60", () => {
    expect(formatLat(29 + 59 / 60 + 59.99 / 3600, { format: "sexagesimal" })).toBe("+30°00'00.0\"");
  });
  it("returns a dash for non-finite input", () => {
    expect(formatLat(Infinity, { format: "decimal" })).toBe("—");
  });
});

describe("solveCentreText", () => {
  it("carries a centre at 59.9996 s and 59.96 arcsec into the next minute instead of printing 60", () => {
    const text = solveCentreText(EDGE_RA, EDGE_DEC);
    expect(text.ra).toBe("05h36m00.00s");
    expect(text.dec).toBe("-05°24'00.0\"");
    expect(Object.values(text).filter((s) => s.includes("60.0"))).toEqual([]);
  });

  it("prints the same sexagesimal strings as the WCS readout's default precision", () => {
    const text = solveCentreText(GC_RA, GC_DEC);
    expect(text.ra).toBe(formatLon(GC_RA, { hours: true, format: "sexagesimal" }));
    expect(text.dec).toBe(formatLat(GC_DEC, { format: "sexagesimal" }));
  });

  it("adds decimal degrees with six decimals", () => {
    expect(solveCentreText(GC_RA, GC_DEC)).toMatchObject({ raDeg: "266.404988°", decDeg: "-28.936178°" });
    expect(solveCentreText(EDGE_RA, EDGE_DEC)).toMatchObject({ raDeg: "83.999998°", decDeg: "-5.399989°" });
  });
});

const SUB64P_SIDE_ARCMIN = (64 * 0.0311) / 60;

describe("solveSummaryLine", () => {
  it("formats the centre in decimal degrees with its unit and frame and in sexagesimal, then the pixel scale and the FOV", () => {
    const line = solveSummaryLine({
      center_ra: EDGE_RA,
      center_dec: EDGE_DEC,
      pixel_scale_arcsec: 0.0629,
      field_of_view_w_arcmin: 2.24,
      field_of_view_h_arcmin: 2.31,
    });
    expect(line).toBe("83.999998 -5.399989 deg ICRS | 05h36m00.00s -05°24'00.0\" | 0.06290\"/px | FOV 2.2' x 2.3'");
  });

  it("leaves out a pixel scale or FOV the WCS does not give", () => {
    expect(solveSummaryLine({ center_ra: GC_RA, center_dec: GC_DEC, pixel_scale_arcsec: null, field_of_view_w_arcmin: 3 })).toBe(
      "266.404988 -28.936178 deg ICRS | 17h45m37.20s -28°56'10.2\"",
    );
  });

  it("copies a 2 arcsec subarray with a four-figure scale and its FOV in arcsec", () => {
    const line = solveSummaryLine({
      center_ra: 83.8,
      center_dec: -5.4,
      pixel_scale_arcsec: 0.0311,
      field_of_view_w_arcmin: SUB64P_SIDE_ARCMIN,
      field_of_view_h_arcmin: SUB64P_SIDE_ARCMIN,
    });
    expect(line).toBe("83.800000 -5.400000 deg ICRS | 05h35m12.00s -05°24'00.0\" | 0.03110\"/px | FOV 2\" x 2\"");
  });
});

describe("pixelScaleText", () => {
  it("keeps four significant figures at any scale", () => {
    expect(pixelScaleText(0.03962)).toBe('0.03962"/px');
    expect(pixelScaleText(0.0629)).toBe('0.06290"/px');
    expect(pixelScaleText(0.11)).toBe('0.1100"/px');
    expect(pixelScaleText(1.254)).toBe('1.254"/px');
    expect(pixelScaleText(21.04)).toBe('21.04"/px');
    expect(pixelScaleText(206.3)).toBe('206.3"/px');
  });

  it("gives no scale for a missing, non-finite or non-positive value", () => {
    expect(pixelScaleText(null)).toBeNull();
    expect(pixelScaleText(undefined)).toBeNull();
    expect(pixelScaleText(NaN)).toBeNull();
    expect(pixelScaleText(0)).toBeNull();
  });
});

describe("fieldOfViewText", () => {
  it.each([
    [7.41, 4.46],
    [2.14, 2.14],
    [0.75, 0.5],
    [SUB64P_SIDE_ARCMIN, SUB64P_SIDE_ARCMIN],
  ])("prints a %f' x %f' field like the Archive header peek", (w, h) => {
    const peek = fieldText({ fieldArcmin: [w, h] } as MastQuickWcs);
    expect(fieldOfViewText(w, h)).toBe(peek.replace(/^field /, ""));
  });

  it("switches to arcsec below one arcminute", () => {
    expect(fieldOfViewText(SUB64P_SIDE_ARCMIN, SUB64P_SIDE_ARCMIN)).toBe('2" x 2"');
    expect(fieldOfViewText(0.75, 0.5)).toBe('45" x 30"');
    expect(fieldOfViewText(2.24, 2.31)).toBe("2.2' x 2.3'");
  });
});

describe("skyPositionLine", () => {
  it("formats a sky position in decimal degrees with its unit and frame and in sexagesimal", () => {
    expect(skyPositionLine(83.82208, -5.39111)).toBe("83.822080 -5.391110 deg ICRS | 05h35m17.30s -05°23'28.0\"");
    expect(skyPositionLine(27.5, 27.12825)).toBe("27.500000 +27.128250 deg ICRS | 01h50m00.00s +27°07'41.7\"");
  });
});

describe("frame helpers", () => {
  it("labels the axes per frame", () => {
    expect(frameAxisLabels("icrs")).toEqual(["RA", "Dec"]);
    expect(frameAxisLabels("fk5")).toEqual(["RA", "Dec"]);
    expect(frameAxisLabels("galactic")).toEqual(["l", "b"]);
    expect(frameAxisLabels("ecliptic")).toEqual(["λ", "β"]);
  });
  it("uses hours only for equatorial frames", () => {
    expect(frameLonInHours("icrs")).toBe(true);
    expect(frameLonInHours("fk5")).toBe(true);
    expect(frameLonInHours("galactic")).toBe(false);
    expect(frameLonInHours("ecliptic")).toBe(false);
  });
});
