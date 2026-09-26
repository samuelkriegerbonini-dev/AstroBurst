import { describe, expect, it } from "vitest";
import { ZERO_BASED_PIXEL_TITLE, shapeSummaryTitle, zeroBasedPixelText } from "../regionGeometry";

describe("0-based pixel labels", () => {
  it("marks the hover readout as 0-based", () => {
    expect(zeroBasedPixelText(12, 340)).toBe("px(12,340) 0-based");
  });

  it("explains the offset against DS9 and .reg files", () => {
    expect(ZERO_BASED_PIXEL_TITLE).toBe(
      "Image pixel coordinates are 0-based at the pixel centre; DS9 and the exported .reg are 1-based (add 1).",
    );
  });

  it("puts the note under the region summary in the row tooltip", () => {
    expect(shapeSummaryTitle({ shape: "point", x: 10, y: 20.25 })).toBe(`point (10.0, 20.3) 0-based\n${ZERO_BASED_PIXEL_TITLE}`);
  });
});
