import { describe, it, expect } from "vitest";
import {
  shouldRetryWithoutFullAnalysis,
  combineAttemptErrors,
  selectCubePlaneHdu,
  isCubePlaneResult,
} from "../fitsErrors";

const CUBE_REJECTION =
  "Failed to load C:/data/jw01234_s3d.fits: HDU 1 (SCI): 3D cube of 1400 planes, not a single 2D image; " +
  "no HDU in this file holds a 2D image. " +
  "HDUs: [0] EXTNAME=(none) XTENSION=PRIMARY NAXIS=0 -- header only, no pixel data; " +
  "[1] EXTNAME=SCI XTENSION=IMAGE NAXIS=3 [50x50x1400] -- 3D cube of 1400 planes, not a single 2D image";

const X1D_REJECTION =
  "Failed to load C:/data/jw01234_x1d.fits: HDU 1 (EXTRACT1D): BINTABLE extension, holds no image pixels; " +
  "no HDU in this file holds a 2D image. " +
  "HDUs: [0] EXTNAME=(none) XTENSION=PRIMARY NAXIS=0 -- header only, no pixel data; " +
  "[1] EXTNAME=EXTRACT1D XTENSION=BINTABLE NAXIS=2 [232x1024] -- BINTABLE extension, holds no image pixels";

function legacyIsRetriable(msg: string): boolean {
  return (
    !msg.includes("Calibration reference file") &&
    !msg.includes("No such file") &&
    !msg.includes("not found") &&
    !msg.includes("Permission denied")
  );
}

describe("shouldRetryWithoutFullAnalysis", () => {
  it("does not retry structurally unsupported products the old prose sniffing retried", () => {
    expect(legacyIsRetriable(CUBE_REJECTION)).toBe(true);
    expect(legacyIsRetriable(X1D_REJECTION)).toBe(true);
    expect(shouldRetryWithoutFullAnalysis(CUBE_REJECTION)).toBe(false);
    expect(shouldRetryWithoutFullAnalysis(X1D_REJECTION)).toBe(false);
  });

  it("does not retry the errors the old allowlist already excluded", () => {
    for (const msg of [
      "Calibration reference file (no image data): jwst_nircam_flat.asdf",
      "Failed to open C:/x.fits: No such file or directory (os error 2)",
      "Failed to open C:/x.fits: Permission denied (os error 5)",
      "HDU 1 cannot be loaded as a 2D image: 3D cube of 5 planes, not a single 2D image",
    ]) {
      expect(shouldRetryWithoutFullAnalysis(msg)).toBe(false);
    }
  });

  it("retries only when the full-analysis task itself panicked", () => {
    expect(shouldRetryWithoutFullAnalysis("Task join failed: task panicked")).toBe(true);
    expect(shouldRetryWithoutFullAnalysis("  Task join failed: cancelled")).toBe(true);
    expect(shouldRetryWithoutFullAnalysis("Reader task join failed: task panicked")).toBe(false);
  });

  it("keeps every new rejection message clear of the old allowlist tokens", () => {
    for (const msg of [CUBE_REJECTION, X1D_REJECTION]) {
      expect(msg).not.toContain("not found");
      expect(msg).not.toContain("No such file");
      expect(msg).not.toContain("Permission denied");
      expect(msg).not.toContain("Calibration reference file");
    }
  });
});

const S3D_PATH = "C:/data/jw01234_s3d.fits";

const S3D_EXTENSIONS = [
  { index: 0, extname: null, naxis: 0, naxis3: 0, has_data: false, ref: `${S3D_PATH}#hdu=0` },
  { index: 1, extname: "SCI", naxis: 3, naxis3: 1400, has_data: true, ref: `${S3D_PATH}#hdu=1` },
  { index: 2, extname: "ERR", naxis: 3, naxis3: 1400, has_data: true, ref: `${S3D_PATH}#hdu=2` },
  { index: 3, extname: "DQ", naxis: 3, naxis3: 1400, has_data: true, ref: `${S3D_PATH}#hdu=3` },
  { index: 4, extname: "HDRTAB", naxis: 2, naxis3: 0, has_data: false, ref: `${S3D_PATH}#hdu=4` },
];

const X1D_PATH = "C:/data/jw01234_x1d.fits";

const X1D_EXTENSIONS = [
  { index: 0, extname: null, naxis: 0, naxis3: 0, has_data: false, ref: `${X1D_PATH}#hdu=0` },
  { index: 1, extname: "EXTRACT1D", naxis: 2, naxis3: 0, has_data: false, ref: `${X1D_PATH}#hdu=1` },
];

const ROMAN_PATH = "C:/data/r0001_wfi01_uncal.asdf";

const ROMAN_RAMP_EXTENSIONS = [
  { index: 0, extname: "roman.data", naxis: 3, naxis3: 6, has_data: true, ref: `${ROMAN_PATH}#array=roman.data` },
  { index: 1, extname: "roman.dq", naxis: 2, naxis3: 0, has_data: true, ref: `${ROMAN_PATH}#array=roman.dq` },
];

describe("selectCubePlaneHdu", () => {
  it("keeps a spectral cube openable by naming its science HDU", () => {
    expect(selectCubePlaneHdu(S3D_EXTENSIONS)).toBe(`${S3D_PATH}#hdu=1`);
  });

  it("falls back to the first cube when no SCI extension exists", () => {
    expect(selectCubePlaneHdu(S3D_EXTENSIONS.filter((e) => e.extname !== "SCI"))).toBe(
      `${S3D_PATH}#hdu=2`,
    );
  });

  it("recovers an ASDF ramp through its array key, which is the only ref the backend accepts", () => {
    expect(selectCubePlaneHdu(ROMAN_RAMP_EXTENSIONS)).toBe(`${ROMAN_PATH}#array=roman.data`);
    expect(selectCubePlaneHdu(ROMAN_RAMP_EXTENSIONS)).not.toContain("#hdu=");
  });

  it("offers no cube fallback for a product that holds no cube", () => {
    expect(selectCubePlaneHdu(X1D_EXTENSIONS)).toBeNull();
    expect(selectCubePlaneHdu([])).toBeNull();
    expect(
      selectCubePlaneHdu([
        { index: 1, extname: "SCI", naxis: 3, naxis3: 1, has_data: true, ref: `${S3D_PATH}#hdu=1` },
      ]),
    ).toBeNull();
  });
});

describe("isCubePlaneResult", () => {
  it("marks a loaded cube plane so it cannot become the auto-resample target", () => {
    expect(isCubePlaneResult({ NAXIS: "3", NAXIS3: "1400" }, undefined)).toBe(true);
    expect(isCubePlaneResult({ NAXIS: "3", NAXIS3: "1400" }, false)).toBe(true);
  });

  it("leaves ordinary 2D images and RGB composites in the grouping", () => {
    expect(isCubePlaneResult({ NAXIS: "2" }, undefined)).toBe(false);
    expect(isCubePlaneResult(null, undefined)).toBe(false);
    expect(isCubePlaneResult({ NAXIS: "3", NAXIS3: "3" }, true)).toBe(false);
    expect(isCubePlaneResult({ NAXIS3: "not-a-number" }, undefined)).toBe(false);
  });
});

describe("combineAttemptErrors", () => {
  it("surfaces a differing retry failure instead of hiding it", () => {
    expect(combineAttemptErrors("first", "second")).toContain("second");
    expect(combineAttemptErrors("first", "second")).toContain("first");
  });

  it("does not duplicate an identical message", () => {
    expect(combineAttemptErrors("same", "same")).toBe("same");
  });
});
