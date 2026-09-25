import { describe, it, expect } from "vitest";
import { MAX_TARGETS, detectSkyCsvHeader, parseAngle, parseSkyCsv, parseSkyList } from "../skyList";

const ETA_RA_DEG = (10 + 45 / 60 + 3.6 / 3600) * 15;
const ETA_DEC_DEG = -(59 + 41 / 60 + 4 / 3600);

describe("parseAngle", () => {
  it("reads colon separated RA as hours and Dec as degrees", () => {
    expect(parseAngle("10:45:03.6", "lon").value).toBeCloseTo(ETA_RA_DEG, 9);
    expect(parseAngle("-59:41:04", "lat").value).toBeCloseTo(ETA_DEC_DEG, 9);
  });

  it("reads hms and dms letter forms", () => {
    expect(parseAngle("10h45m03.6s", "lon").value).toBeCloseTo(ETA_RA_DEG, 9);
    expect(parseAngle("-59d41m04s", "lat").value).toBeCloseTo(ETA_DEC_DEG, 9);
    expect(parseAngle("161d15m54s", "lon").value).toBeCloseTo(161 + 15 / 60 + 54 / 3600, 9);
  });

  it("reads decimal RA as degrees and wraps it into 0..360", () => {
    expect(parseAngle("161.265", "lon").value).toBeCloseTo(161.265, 9);
    expect(parseAngle("-10", "lon").value).toBeCloseTo(350, 9);
    expect(parseAngle("-01:00:00", "lon").value).toBeCloseTo(345, 9);
    expect(parseAngle("24:00:00", "lon").value).toBeCloseTo(0, 9);
  });

  it("rejects RA above 24 hours, Dec beyond 90 degrees, bad fields and non-finite values", () => {
    expect(parseAngle("25:00:00", "lon").error).toMatch(/exceeds 24 hours/);
    expect(parseAngle("10:61:00", "lon").error).toMatch(/60 or more/);
    expect(parseAngle("95.5", "lat").error).toMatch(/outside -90..90/);
    expect(parseAngle("1e999", "lon").error).toMatch(/not a finite number/);
    expect(parseAngle("NaN", "lon").error).toMatch(/neither decimal degrees nor sexagesimal/);
    expect(parseAngle("", "lat").error).toMatch(/Dec is empty/);
  });
});

describe("parseSkyList", () => {
  it("parses sexagesimal, decimal and letter forms with labels and keeps the raw text", () => {
    const text = [
      "# eta Carinae in three spellings",
      "10:45:03.6 -59:41:04 eta",
      "161.265 -59.684",
      "10h45m03.6s -59d41m04s eta car # trailing comment",
      "10 45 03.6 -59 41 04 spaced",
    ].join("\n");
    const { targets, errors } = parseSkyList(text);
    expect(errors).toEqual([]);
    expect(targets.map((t) => t.label)).toEqual(["eta", "", "eta car", "spaced"]);
    expect(targets[0].lon).toBeCloseTo(ETA_RA_DEG, 9);
    expect(targets[0].lat).toBeCloseTo(ETA_DEC_DEG, 9);
    expect(targets[1].lon).toBeCloseTo(161.265, 9);
    expect(targets[1].lat).toBeCloseTo(-59.684, 9);
    expect(targets[2].lon).toBeCloseTo(ETA_RA_DEG, 9);
    expect(targets[3].lat).toBeCloseTo(ETA_DEC_DEG, 9);
    expect(targets[2].raw).toBe("10h45m03.6s -59d41m04s eta car");
  });

  it("accepts commas as separators", () => {
    const { targets, errors } = parseSkyList("10:45:03.6, -59:41:04, eta");
    expect(errors).toEqual([]);
    expect(targets[0].label).toBe("eta");
    expect(targets[0].lat).toBeCloseTo(ETA_DEC_DEG, 9);
  });

  it("reports errors with line numbers and keeps the good lines", () => {
    const { targets, errors } = parseSkyList("10:45:03.6 -59:41:04 ok\n25:00:00 10:00:00 bad\nonly-one\n1e999 5");
    expect(targets).toHaveLength(1);
    expect(errors).toEqual([
      "line 2: RA 25:00:00 exceeds 24 hours",
      "line 3: needs at least RA and Dec",
      "line 4: 1e999 is not a finite number",
    ]);
  });

  it("caps the list at MAX_TARGETS and says how many were ignored", () => {
    const lines = Array.from({ length: MAX_TARGETS + 3 }, (_, i) => `${(i % 360) + 0.5} 1.5 t${i}`);
    const { targets, errors } = parseSkyList(lines.join("\n"));
    expect(targets).toHaveLength(MAX_TARGETS);
    expect(errors).toEqual([`only the first ${MAX_TARGETS} targets are kept, 3 more were ignored`]);
  });
});

describe("parseSkyCsv", () => {
  it("detects RAJ2000 and DEJ2000 headers with an id column", () => {
    const text = "Source,RAJ2000,DEJ2000,Gmag\nA,161.265,-59.684,6.2\nB,10:45:03.6,-59:41:04,7.1\n";
    const { targets, errors } = parseSkyCsv(text);
    expect(errors).toEqual([]);
    expect(targets).toHaveLength(2);
    expect(targets[0]).toMatchObject({ label: "", lon: 161.265, lat: -59.684 });
    expect(targets[1].lon).toBeCloseTo(ETA_RA_DEG, 9);
  });

  it("uses the id column for labels and reports bad cells with line numbers", () => {
    const text = "id;ra;dec\nHD 1;1.5;2.5\nHD 2;x;2.5\n";
    const { targets, errors } = parseSkyCsv(text);
    expect(targets).toEqual([{ label: "HD 1", lon: 1.5, lat: 2.5, raw: "HD 1;1.5;2.5" }]);
    expect(errors).toEqual(["line 3: x is neither decimal degrees nor sexagesimal"]);
  });

  it("falls back to the plain list parser when there is no header row", () => {
    const { targets } = parseSkyCsv("161.265 -59.684 eta\n");
    expect(targets).toHaveLength(1);
    expect(targets[0].label).toBe("eta");
  });

  it("detects a header only when both coordinate columns are present", () => {
    expect(detectSkyCsvHeader("ra,dec,name")).toEqual({ delimiter: ",", lon: 0, lat: 1, label: 2 });
    expect(detectSkyCsvHeader("ra,flux")).toBeNull();
    expect(detectSkyCsvHeader("")).toBeNull();
  });
});
