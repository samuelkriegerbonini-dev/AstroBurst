import { describe, expect, it } from "vitest";
import { autoDetectChannel } from "../drizzleChannel";

describe("autoDetectChannel", () => {
  it("reads the channel from the name of a plain FITS frame", () => {
    expect(autoDetectChannel("m42_r.fits")).toBe("r");
    expect(autoDetectChannel("M42-Green.fit")).toBe("g");
    expect(autoDetectChannel("blue_m42.fts")).toBe("b");
    expect(autoDetectChannel("m42_lum.fits")).toBeNull();
  });

  it("gives a gzip-compressed frame the channel of its plain file", () => {
    for (const base of ["m42_r", "M42-Green", "blue_m42", "m42.b", "plain"]) {
      for (const ext of [".fits", ".fit", ".fts"]) {
        expect(autoDetectChannel(`${base}${ext}.gz`), `${base}${ext}.gz`).toBe(autoDetectChannel(`${base}${ext}`));
      }
    }
  });
});
