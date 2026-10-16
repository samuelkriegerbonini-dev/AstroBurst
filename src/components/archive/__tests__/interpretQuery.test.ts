import { describe, expect, it } from "vitest";
import { interpretQuery } from "../logic/interpretQuery";

describe("interpretQuery: coordinates", () => {
  it("reads decimal degrees", () => {
    const q = interpretQuery("274.688 -13.792");
    expect(q.kind).toBe("coordinates");
    expect(q.error).toBeNull();
    expect(q.raDeg).toBeCloseTo(274.688, 6);
    expect(q.decDeg).toBeCloseTo(-13.792, 6);
  });

  it("accepts a comma between decimal values and a plus sign on Dec", () => {
    const q = interpretQuery(" 10.0, +41.27 ");
    expect(q.kind).toBe("coordinates");
    expect(q.raDeg).toBeCloseTo(10, 6);
    expect(q.decDeg).toBeCloseTo(41.27, 6);
  });

  it("reads colon-separated sexagesimal (RA in hours)", () => {
    const q = interpretQuery("18:18:48 -13:47:31");
    expect(q.kind).toBe("coordinates");
    expect(q.raDeg).toBeCloseTo((18 + 18 / 60 + 48 / 3600) * 15, 6);
    expect(q.decDeg).toBeCloseTo(-(13 + 47 / 60 + 31 / 3600), 6);
  });

  it("reads h/m/s and d/m/s letters", () => {
    const q = interpretQuery("18h18m48s -13d47m31s");
    expect(q.kind).toBe("coordinates");
    expect(q.raDeg).toBeCloseTo(274.7, 6);
    expect(q.decDeg).toBeCloseTo(-13.791944, 5);
  });

  it("keeps the sign of a negative Dec whose degrees are zero", () => {
    const q = interpretQuery("00:42:44 -00:30:00");
    expect(q.decDeg).toBeCloseTo(-0.5, 6);
  });

  it("rejects an RA outside [0, 360)", () => {
    const q = interpretQuery("360 10");
    expect(q.kind).toBe("coordinates");
    expect(q.error).toBe("RA must be at least 0 and below 360 degrees");
  });

  it("rejects a Dec outside [-90, 90]", () => {
    const q = interpretQuery("10 -91");
    expect(q.kind).toBe("coordinates");
    expect(q.error).toBe("Dec must be between -90 and +90 degrees");
  });

  it("rejects sexagesimal hours of 24 or more and minutes of 60 or more", () => {
    expect(interpretQuery("24:00:00 +10:00:00").error).toBe("RA hours must be below 24");
    expect(interpretQuery("10:60:00 +10:00:00").error).toBe("Minutes and seconds must be below 60");
  });
});

describe("interpretQuery: programmes and dataset ids", () => {
  it.each([["2739", 2739], ["15238", 15238]])("reads %s as a programme", (text, program) => {
    const q = interpretQuery(text);
    expect(q.kind).toBe("programme");
    expect(q.program).toBe(program);
    expect(q.error).toBeNull();
  });

  it("reads a JWST fileset as a dataset id", () => {
    const q = interpretQuery("jw02739-o001_t001_nircam");
    expect(q.kind).toBe("dataset");
    expect(q.datasetId).toBe("jw02739-o001_t001_nircam");
  });

  it("reads a JWST exposure id as a dataset id and lowercases it", () => {
    const q = interpretQuery("JW02739001002_02101_00001");
    expect(q.kind).toBe("dataset");
    expect(q.datasetId).toBe("jw02739001002_02101_00001");
  });

  it("reads a 9-character HST rootname as a dataset id in upper case", () => {
    const q = interpretQuery("ick909030");
    expect(q.kind).toBe("dataset");
    expect(q.datasetId).toBe("ICK909030");
  });

  it("does not take a 9-letter word for an HST rootname", () => {
    expect(interpretQuery("andromeda").kind).toBe("name");
    expect(interpretQuery("Messier16").kind).toBe("name");
  });
});

describe("interpretQuery: names and empty input", () => {
  it.each(["M16", "Pillars of Creation", "NGC 6611", "HD 93129A", "123"])("reads %s as a name", (text) => {
    const q = interpretQuery(text);
    expect(q.kind).toBe("name");
    expect(q.name).toBe(text);
    expect(q.error).toBeNull();
  });

  it("trims the name", () => {
    expect(interpretQuery("  M16 ").name).toBe("M16");
  });

  it("reports an empty query", () => {
    expect(interpretQuery("   ").kind).toBe("empty");
    expect(interpretQuery("").error).toBeNull();
  });
});

describe("interpretQuery: manual override of the kind", () => {
  it("treats digits as a name when the user picks name", () => {
    const q = interpretQuery("2739", "name");
    expect(q.kind).toBe("name");
    expect(q.name).toBe("2739");
  });

  it("parses coordinates only when they are valid", () => {
    expect(interpretQuery("M16", "coordinates")).toMatchObject({ kind: "coordinates", error: "Coordinates: type RA and Dec, for example 274.688 -13.792" });
  });

  it("requires digits for a programme", () => {
    expect(interpretQuery("M16", "programme").error).toBe("Programme: type a number, for example 2739");
    expect(interpretQuery("0042", "programme")).toMatchObject({ kind: "programme", program: 42, error: null });
  });

  it("accepts any safe id as a dataset and refuses unsafe characters", () => {
    expect(interpretQuery("N9CS020E0", "dataset")).toMatchObject({ kind: "dataset", datasetId: "N9CS020E0", error: null });
    expect(interpretQuery("../etc", "dataset").error).toBe("Dataset id: use letters, digits, dot, underscore or hyphen");
    expect(interpretQuery(".hidden", "dataset").error).toBe("Dataset id: use letters, digits, dot, underscore or hyphen");
  });

  it("ignores the override on an empty field", () => {
    expect(interpretQuery("", "name").kind).toBe("empty");
  });
});
