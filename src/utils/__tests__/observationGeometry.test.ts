import { describe, it, expect } from "vitest";
import {
  NO_SITE_HINT,
  NO_TARGET_HINT,
  NO_TIME_HINT,
  SITE_OVERRIDE_STORAGE_KEY,
  formatHours,
  geometryRows,
  geometryText,
  loadSiteOverride,
  overridesFor,
  parseSiteInputs,
  parseTargetInputs,
  saveSiteOverride,
  type StorageLike,
} from "../observationGeometry";
import type { ObservationGeometryResult } from "../../shared/types/geometry";

function memoryStorage(initial: Record<string, string> = {}): StorageLike & { data: Record<string, string> } {
  const data = { ...initial };
  return {
    data,
    getItem: (k) => (k in data ? data[k] : null),
    setItem: (k, v) => {
      data[k] = v;
    },
    removeItem: (k) => {
      delete data[k];
    },
  };
}

function result(overrides: Partial<ObservationGeometryResult> = {}): ObservationGeometryResult {
  return {
    geometry: {
      jd_utc: 2461120.923611,
      jd_tt: 2461120.924412,
      jd_tdb: 2461120.924412,
      bjd_tdb: 2461120.9251234,
      hjd_utc: 2461120.9243211,
      bjd_source: "computed",
      lst_deg: 197.693195,
      hour_angle_deg: 15,
      altitude_deg: 70.1812345,
      azimuth_deg: 180.00005,
      airmass_computed: 1.0626,
      airmass_formula: "Kasten & Young 1989",
      parallactic_angle_deg: -12.3456789,
      sun_altitude_deg: -40.5,
      moon_altitude_deg: 12.25,
      moon_illumination: 0.6786,
      moon_separation_deg: 91.12345,
      time_scale_notes: ["time: DATE-OBS + EXPTIME/2 (600 s)", "TIMESYS absent: UTC assumed"],
    },
    target: { ra_deg: 150, dec_deg: 2, source: "WCS at image centre (49.5, 49.5)" },
    site: { lon_deg: -155.47, lat_deg: 19.82, height_m: 0, source: "SITELAT/SITELONG" },
    time_source: "DATE-OBS + EXPTIME/2 (600 s)",
    airmass_header: 1.07,
    notes: ["site from SITELAT/SITELONG: lon -155.47000 lat 19.82000 height 0 m (east-positive longitude)"],
    elapsed_ms: 2,
    ...overrides,
  };
}

describe("site override storage", () => {
  it("loadSiteOverride returns the stored site and defaults a missing height to 0", () => {
    const storage = memoryStorage({ [SITE_OVERRIDE_STORAGE_KEY]: JSON.stringify({ lat: 19.82, lon: -155.47 }) });
    expect(loadSiteOverride(storage)).toEqual({ lat: 19.82, lon: -155.47, height: 0 });
    const full = memoryStorage({ [SITE_OVERRIDE_STORAGE_KEY]: JSON.stringify({ lat: -30.2, lon: 289.3, height: 2400 }) });
    expect(loadSiteOverride(full)).toEqual({ lat: -30.2, lon: 289.3, height: 2400 });
  });

  it("loadSiteOverride returns null for malformed JSON, a latitude of 95 and a null storage", () => {
    expect(loadSiteOverride(memoryStorage({ [SITE_OVERRIDE_STORAGE_KEY]: "{not json" }))).toBeNull();
    expect(loadSiteOverride(memoryStorage({ [SITE_OVERRIDE_STORAGE_KEY]: JSON.stringify({ lat: 95, lon: 0 }) }))).toBeNull();
    expect(loadSiteOverride(memoryStorage({ [SITE_OVERRIDE_STORAGE_KEY]: JSON.stringify({ lat: 10, lon: 400 }) }))).toBeNull();
    expect(loadSiteOverride(memoryStorage({ [SITE_OVERRIDE_STORAGE_KEY]: JSON.stringify({ lat: "10", lon: 5 }) }))).toBeNull();
    expect(loadSiteOverride(memoryStorage())).toBeNull();
    expect(loadSiteOverride(null)).toBeNull();
    const throwing: StorageLike = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {},
      removeItem: () => {},
    };
    expect(loadSiteOverride(throwing)).toBeNull();
  });

  it("saveSiteOverride writes JSON and removes the key for null", () => {
    const storage = memoryStorage();
    saveSiteOverride(storage, { lat: 19.82, lon: -155.47, height: 4200 });
    expect(JSON.parse(storage.data[SITE_OVERRIDE_STORAGE_KEY])).toEqual({ lat: 19.82, lon: -155.47, height: 4200 });
    saveSiteOverride(storage, null);
    expect(SITE_OVERRIDE_STORAGE_KEY in storage.data).toBe(false);
    expect(() => saveSiteOverride(null, { lat: 1, lon: 2, height: 3 })).not.toThrow();
  });
});

describe("parseSiteInputs", () => {
  it("parseSiteInputs returns null for blank inputs and errors for a lone latitude, a lone height or a longitude of 400", () => {
    expect(parseSiteInputs("", "", "")).toEqual({ site: null, error: null });
    expect(parseSiteInputs("  ", "", " ")).toEqual({ site: null, error: null });
    expect(parseSiteInputs("19.82", "", "")).toEqual({ site: null, error: "site override needs both latitude and longitude" });
    expect(parseSiteInputs("", "", "4200")).toEqual({ site: null, error: "site override needs both latitude and longitude" });
    const lon = parseSiteInputs("19.82", "400", "");
    expect(lon.site).toBeNull();
    expect(lon.error).toContain("longitude");
    const lat = parseSiteInputs("-91", "10", "");
    expect(lat.site).toBeNull();
    expect(lat.error).toContain("latitude");
    const height = parseSiteInputs("10", "10", "20000");
    expect(height.site).toBeNull();
    expect(height.error).toContain("height");
    const text = parseSiteInputs("abc", "10", "");
    expect(text.site).toBeNull();
    expect(text.error).toContain("latitude");
  });

  it("parseSiteInputs defaults a blank height to 0", () => {
    expect(parseSiteInputs("19.82", "-155.47", "")).toEqual({ site: { lat: 19.82, lon: -155.47, height: 0 }, error: null });
    expect(parseSiteInputs("19.82", "-155.47", "4200")).toEqual({ site: { lat: 19.82, lon: -155.47, height: 4200 }, error: null });
  });
});

describe("parseTargetInputs", () => {
  it("parseTargetInputs rejects RA 360 and Dec -91 and accepts decimal degrees", () => {
    expect(parseTargetInputs("", "")).toEqual({ target: null, error: null });
    expect(parseTargetInputs("150.123456", "-2.5")).toEqual({ target: { ra: 150.123456, dec: -2.5 }, error: null });
    expect(parseTargetInputs("0", "90")).toEqual({ target: { ra: 0, dec: 90 }, error: null });
    const ra = parseTargetInputs("360", "0");
    expect(ra.target).toBeNull();
    expect(ra.error).toContain("RA");
    const dec = parseTargetInputs("10", "-91");
    expect(dec.target).toBeNull();
    expect(dec.error).toContain("Dec");
    const half = parseTargetInputs("10", "");
    expect(half.target).toBeNull();
    expect(half.error).toBe("target override needs both RA and Dec");
    const sexagesimal = parseTargetInputs("10:00:00", "5");
    expect(sexagesimal.target).toBeNull();
    expect(sexagesimal.error).toContain("RA");
  });
});

describe("geometryRows", () => {
  it("geometryRows keeps the documented order, digits and -- for nulls with the bjd source as hint", () => {
    const rows = geometryRows(result());
    expect(rows.map((r) => r.key)).toEqual([
      "jd_utc",
      "jd_tt",
      "jd_tdb",
      "bjd_tdb",
      "hjd_utc",
      "lst",
      "hour_angle",
      "altitude",
      "azimuth",
      "airmass_computed",
      "airmass_header",
      "parallactic_angle",
      "sun_altitude",
      "moon_altitude",
      "moon_illumination",
      "moon_separation",
    ]);
    const byKey = Object.fromEntries(rows.map((r) => [r.key, r]));
    expect(byKey.jd_utc.value).toBe("2461120.923611");
    expect(byKey.bjd_tdb.value).toBe("2461120.925123");
    expect(byKey.bjd_tdb.hint).toBe("computed");
    expect(byKey.hjd_utc.value).toBe("2461120.924321");
    expect(byKey.lst.value).toBe("197.6932 (13:10:46.4)");
    expect(byKey.lst.unit).toBe("deg");
    expect(byKey.hour_angle.value).toBe("1.0000");
    expect(byKey.hour_angle.unit).toBe("h");
    expect(byKey.altitude.value).toBe("70.1812");
    expect(byKey.altitude.unit).toBe("deg");
    expect(byKey.airmass_computed.value).toBe("1.0626");
    expect(byKey.airmass_computed.hint).toBe("Kasten & Young 1989");
    expect(byKey.airmass_header.value).toBe("1.0700");
    expect(byKey.parallactic_angle.value).toBe("-12.3457");
    expect(byKey.moon_illumination.value).toBe("67.9");
    expect(byKey.moon_illumination.unit).toBe("%");
    expect(byKey.moon_separation.value).toBe("91.1235");
    expect(rows.every((r) => r.label.length > 0)).toBe(true);
    const empty = geometryRows(
      result({
        geometry: { ...result().geometry, bjd_tdb: null, hjd_utc: null, bjd_source: null, lst_deg: null, hour_angle_deg: null, airmass_computed: null, moon_illumination: null },
        airmass_header: null,
      }),
    );
    const emptyByKey = Object.fromEntries(empty.map((r) => [r.key, r]));
    expect(emptyByKey.bjd_tdb.value).toBe("--");
    expect(emptyByKey.bjd_tdb.hint).toBeUndefined();
    expect(emptyByKey.hjd_utc.value).toBe("--");
    expect(emptyByKey.lst.value).toBe("--");
    expect(emptyByKey.hour_angle.value).toBe("--");
    expect(emptyByKey.airmass_computed.value).toBe("--");
    expect(emptyByKey.airmass_header.value).toBe("--");
    expect(emptyByKey.moon_illumination.value).toBe("--");
  });

  it("formatHours writes 197.693195 degrees as 13:10:46.4", () => {
    expect(formatHours(197.693195)).toBe("13:10:46.4");
    expect(formatHours(0)).toBe("00:00:00.0");
    expect(formatHours(359.99999)).toBe("00:00:00.0");
    expect(formatHours(14.999)).toBe("00:59:59.8");
    expect(formatHours(-15)).toBe("23:00:00.0");
  });

  it("geometryText contains every label and every note", () => {
    const res = result();
    const text = geometryText(res);
    for (const row of geometryRows(res)) {
      expect(text).toContain(`${row.label}: ${row.value}${row.unit ? ` ${row.unit}` : ""}`);
    }
    for (const note of [...res.notes, ...res.geometry.time_scale_notes]) {
      expect(text).toContain(note);
    }
    expect(text).toContain("BJD_TDB: 2461120.925123 d");
    expect(text).toContain("Moon illumination: 67.9 %");
  });
});

describe("hints and overrides", () => {
  it("overridesFor maps the target and site into the camelCase override object", () => {
    expect(overridesFor(null, null)).toEqual({ targetRa: null, targetDec: null, siteLat: null, siteLon: null, siteHeight: null });
    expect(overridesFor({ ra: 150.5, dec: -2 }, { lat: 19.82, lon: -155.47, height: 4200 })).toEqual({
      targetRa: 150.5,
      targetDec: -2,
      siteLat: 19.82,
      siteLon: -155.47,
      siteHeight: 4200,
    });
    expect(overridesFor(null, { lat: 1, lon: 2, height: 0 })).toEqual({ targetRa: null, targetDec: null, siteLat: 1, siteLon: 2, siteHeight: 0 });
    expect(NO_TIME_HINT).toContain("BJDREF");
    expect(NO_SITE_HINT).toContain("altitude");
    expect(NO_TARGET_HINT).toContain("BJD_TDB");
  });
});
