import { describe, it, expect } from "vitest";
import { NO_SOLVE_YET, NO_WCS_FILE, SOLVED_WCS_LABEL, solvedWcsSummary, wcsWriteBlocker } from "../solvedWcs";
import type { WriteSolvedWcsResult } from "../../shared/types/astrometry";

function written(overrides: Partial<WriteSolvedWcsResult> = {}): WriteSolvedWcsResult {
  return {
    fits_path: "C:\\Users\\me\\AppData\\Roaming\\astroburst\\output\\wcs_nowcs_673_wcs.fits",
    png_path: "C:\\Users\\me\\AppData\\Roaming\\astroburst\\output\\wcs_nowcs_673_wcs.png",
    dimensions: [1600, 1600],
    center_ra: 83.822,
    center_dec: -5.391,
    pixel_scale_arcsec: 1.99876,
    sip_present: true,
    elapsed_ms: 42,
    ...overrides,
  };
}

describe("wcsWriteBlocker", () => {
  it("asks for a solve when there is no result", () => {
    expect(wcsWriteBlocker(null)).toBe(NO_SOLVE_YET);
    expect(NO_SOLVE_YET).toBe("Plate solve the image first.");
  });

  it("names the missing WCS file when the solve carries no cards", () => {
    expect(wcsWriteBlocker({})).toBe(NO_WCS_FILE);
    expect(wcsWriteBlocker({ wcs_cards: [] })).toBe(NO_WCS_FILE);
    expect(NO_WCS_FILE).toBe("astrometry.net returned no WCS file for this job; solve again to write the WCS.");
  });

  it("lets the write run with at least one card", () => {
    expect(wcsWriteBlocker({ wcs_cards: [["CTYPE1", "RA---TAN-SIP"]] })).toBeNull();
  });
});

describe("solvedWcsSummary", () => {
  it("names the written file, the centre, the scale and SIP for a Windows path", () => {
    expect(solvedWcsSummary(written())).toBe(
      "Wrote wcs_nowcs_673_wcs.fits: centre 83.82200, -5.39100 deg, 1.999\"/px, SIP",
    );
  });

  it("omits SIP when the solution has no distortion terms", () => {
    expect(solvedWcsSummary(written({ fits_path: "/tmp/out/m42_wcs.fits", sip_present: false, pixel_scale_arcsec: 0.4 }))).toBe(
      "Wrote m42_wcs.fits: centre 83.82200, -5.39100 deg, 0.4000\"/px",
    );
  });

  it("labels the published result", () => {
    expect(SOLVED_WCS_LABEL).toBe("Plate-solved WCS");
  });
});
