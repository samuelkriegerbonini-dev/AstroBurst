import { describe, it, expect } from "vitest";
import { formatLat, formatLon, frameAxisLabels, frameLonInHours } from "../coordFormat";

const GC_RA = 266.4049882865447;
const GC_DEC = -28.936177761791473;

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
