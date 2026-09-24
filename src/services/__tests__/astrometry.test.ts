import { describe, it, expectTypeOf } from "vitest";
import type { PlateSolveResult } from "../astrometry";

describe("PlateSolveResult", () => {
  it("declares only the SolveResult fields plate_solve_cmd still serializes", () => {
    expectTypeOf<keyof PlateSolveResult>().toEqualTypeOf<
      | "center_ra"
      | "center_dec"
      | "orientation"
      | "pixel_scale_arcsec"
      | "field_of_view_w_arcmin"
      | "field_of_view_h_arcmin"
    >();
  });
});
