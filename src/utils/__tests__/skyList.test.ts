import { describe, it, expect } from "vitest";
import { formatLat, formatLon } from "../coordFormat";
import { MAX_TARGETS, detectSkyCsvHeader, parseAngle, parseSkyCsv, parseSkyList, targetProjectionPath } from "../skyList";

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

describe("frame-aware sexagesimal longitude", () => {
  it("reads the degree sign as degrees", () => {
    expect(parseAngle(`10°45'03.6"`, "lon").value).toBeCloseTo(10 + 45 / 60 + 3.6 / 3600, 9);
  });

  it("reads galactic and ecliptic sexagesimal longitude as degrees but keeps explicit hours", () => {
    expect(parseAngle("12:30:00", "lon", false).value).toBeCloseTo(12.5, 9);
    expect(parseAngle("120 15 00", "lon", false).value).toBeCloseTo(120.25, 9);
    expect(parseAngle(`359°54'00.0"`, "lon", false).value).toBeCloseTo(359.9, 9);
    expect(parseAngle("12h30m00s", "lon", false).value).toBeCloseTo(187.5, 9);
    expect(parseAngle("12:30:00", "lon").value).toBeCloseTo(187.5, 9);
  });

  it("reads a decimal value written with a trailing degree sign", () => {
    expect(parseAngle("161.265000°", "lon").value).toBeCloseTo(161.265, 9);
    expect(parseAngle("-59.684420°", "lat").value).toBeCloseTo(-59.68442, 9);
  });

  it("round-trips the WcsReadout text in every frame and format", () => {
    const cases = [
      [false, 12.5, 5],
      [false, 120.25, -1.5],
      [false, 359.9, 0.1],
      [true, 161.265, -59.684],
    ] as const;
    for (const format of ["sexagesimal", "decimal"] as const) {
      for (const [hours, l, b] of cases) {
        const line = `${formatLon(l, { hours, format })} ${formatLat(b, { format })}`;
        const { targets, errors } = parseSkyCsv(line, hours);
        expect(errors).toEqual([]);
        expect(targets[0].lon).toBeCloseTo(l, 4);
        expect(targets[0].lat).toBeCloseTo(b, 4);
      }
    }
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

  it("detects VizieR ICRS, computed J2000 and RAdeg headers and never pairs RA with a galactic or magnitude column", () => {
    const gaia = "RA_ICRS,DE_ICRS,Source,Plx,pmRA,pmDE,Gmag\n161.26502,-59.68442,5350395880829341312,0.43,-6.1,2.2,12.3\n";
    const { targets, errors } = parseSkyCsv(gaia);
    expect(errors).toEqual([]);
    expect(targets).toHaveLength(1);
    expect(targets[0].lon).toBeCloseTo(161.26502, 9);
    expect(targets[0].lat).toBeCloseTo(-59.68442, 9);
    expect(detectSkyCsvHeader("RA_ICRS,DE_ICRS,Source,GLON,GLAT")).toEqual({ delimiter: ",", lon: 0, lat: 1, label: -1 });
    expect(detectSkyCsvHeader("RA_ICRS,DE_ICRS,V,B")).toEqual({ delimiter: ",", lon: 0, lat: 1, label: -1 });
    expect(detectSkyCsvHeader("_RAJ2000,_DEJ2000,GLON,GLAT")).toEqual({ delimiter: ",", lon: 0, lat: 1, label: -1 });
    expect(detectSkyCsvHeader("Name,RAdeg,DEdeg")).toEqual({ delimiter: ",", lon: 1, lat: 2, label: 0 });
  });

  it("reads space separated sexagesimal rows under a whitespace header like the plain list", () => {
    const { targets, errors } = parseSkyCsv("RA Dec name\n10 45 03.6 -59 41 04 eta\n161.265 -59.684 eta car");
    expect(errors).toEqual([]);
    expect(targets.map((t) => t.label)).toEqual(["eta", "eta car"]);
    expect(targets[0].lon).toBeCloseTo(ETA_RA_DEG, 9);
    expect(targets[0].lat).toBeCloseTo(ETA_DEC_DEG, 9);
    expect(targets[1].lon).toBeCloseTo(161.265, 9);
  });

  it("refuses extra space separated fields when the coordinates are not the first two columns", () => {
    const { targets, errors } = parseSkyCsv("name RA Dec\neta 10 45 03.6 -59 41 04\nHD1 161.265 -59.684");
    expect(targets).toEqual([{ label: "HD1", lon: 161.265, lat: -59.684, raw: "HD1 161.265 -59.684" }]);
    expect(errors).toEqual([
      "line 2: has 7 space-separated fields but the header has 3; separate the columns with commas or tabs, or write sexagesimal with colons",
    ]);
  });

  it("reads a nameless spaced sexagesimal row under a whitespace header with a name column", () => {
    const { targets, errors } = parseSkyCsv("RA Dec name\n10 45 03.6 -59 41 04");
    expect(errors).toEqual([]);
    expect(targets[0].lon).toBeCloseTo(ETA_RA_DEG, 9);
    expect(targets[0].lat).toBeCloseTo(ETA_DEC_DEG, 9);
    expect(targets[0].label).toBe("");
  });

  it("reads decimal columns of a whitespace table whose last column is a spaced label", () => {
    const astropy = parseSkyCsv(
      'ra dec pmra pmdec parallax phot_g_mean_mag designation\n161.26502 -59.68442 -6.1 2.2 0.43 12.3 "Gaia DR3 5350395880829341312"',
    );
    expect(astropy.errors).toEqual([]);
    expect(astropy.targets).toHaveLength(1);
    expect(astropy.targets[0].label).toBe("Gaia DR3 5350395880829341312");
    expect(astropy.targets[0].lon).toBeCloseTo(161.26502, 9);
    expect(astropy.targets[0].lat).toBeCloseTo(-59.68442, 9);
    const unquoted = parseSkyCsv("RA Dec pmRA pmDE Plx Gmag name\n161.26502 -59.68442 -6.1 2.2 0.43 12.3 eta car");
    expect(unquoted.errors).toEqual([]);
    expect(unquoted.targets[0].label).toBe("eta car");
    expect(unquoted.targets[0].lon).toBeCloseTo(161.26502, 9);
    expect(unquoted.targets[0].lat).toBeCloseTo(-59.68442, 9);
    const integers = parseSkyCsv("ra dec a b c d name\n10 20 3 4 5 6 HD 1");
    expect(integers.errors).toEqual([]);
    expect(integers.targets[0]).toMatchObject({ label: "HD 1", lon: 10, lat: 20 });
    const idFirst = parseSkyCsv("id RA Dec name\n1 161.265 -59.684 eta car");
    expect(idFirst.errors).toEqual([]);
    expect(idFirst.targets[0]).toMatchObject({ label: "1", lon: 161.265, lat: -59.684 });
  });

  it("keeps a quoted spaced label in one cell of a whitespace table", () => {
    const { targets, errors } = parseSkyCsv('designation ra dec\n"Gaia DR3 5350395880829341312" 161.26502 -59.68442');
    expect(errors).toEqual([]);
    expect(targets[0].label).toBe("Gaia DR3 5350395880829341312");
    expect(targets[0].lon).toBeCloseTo(161.26502, 9);
    expect(targets[0].lat).toBeCloseTo(-59.68442, 9);
  });

  it("reads galactic sexagesimal longitude under a header as degrees", () => {
    const { targets, errors } = parseSkyCsv("glon,glat\n12:30:00,+05:00:00\n", false);
    expect(errors).toEqual([]);
    expect(targets[0].lon).toBeCloseTo(12.5, 9);
    expect(targets[0].lat).toBeCloseTo(5, 9);
  });
});

describe("targetProjectionPath", () => {
  it("projects through the displayed result when a stack or drizzle is shown on the loaded file", () => {
    expect(targetProjectionPath("/data/frame_07.fits", "/out/stack_10f.fits")).toBe("/out/stack_10f.fits");
  });

  it("falls back to the loaded file when nothing processed is displayed", () => {
    expect(targetProjectionPath("/data/frame_07.fits", null)).toBe("/data/frame_07.fits");
  });

  it("has no projection path without a loaded file", () => {
    expect(targetProjectionPath(null, "/out/stack_10f.fits")).toBeNull();
  });
});
