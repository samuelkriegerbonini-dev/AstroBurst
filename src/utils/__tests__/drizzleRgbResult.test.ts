import { describe, expect, it } from "vitest";
import { DRIZZLE_GAIN_FIELD, drizzleResultItems, isCancelledRun, manualGains } from "../drizzleRgbResult";
import type { DrizzleRgbResult } from "../../shared/types/stacking";

const result: DrizzleRgbResult = {
  png_path: "C:/out/drizzle_rgb_1.png",
  fits_path: null,
  dimensions: [2048, 2048],
  output_dims: [2048, 2048],
  input_dims: [1024, 1024],
  frame_count_r: 3,
  frame_count_g: 3,
  frame_count_b: 2,
  rejected_pixels: 12345,
  scale: 2,
  warnings: [],
  elapsed_ms: 10,
};

function item(label: string, kernel: string) {
  return drizzleResultItems(result, kernel).find((i) => i.label === label)?.value;
}

describe("drizzleResultItems", () => {
  it("shows the kernel the run used, not a later selection", () => {
    expect(item("Kernel", "square")).toBe("square");
    expect(item("Kernel", "lanczos3")).toBe("lanczos3");
  });

  it("keeps the output size, scale, frame counts and rejected pixels", () => {
    expect(item("Output", "square")).toBe("2048×2048");
    expect(item("Scale", "square")).toBe("2.0x");
    expect([item("R frames", "square"), item("G frames", "square"), item("B frames", "square")]).toEqual([3, 3, 2]);
    expect(item("Rejected", "square")).toBe("12,345");
  });
});

describe("isCancelledRun", () => {
  it("recognises the backend cancel error and nothing else", () => {
    expect(isCancelledRun("Operation cancelled")).toBe(true);
    expect(isCancelledRun("Task join failed: CANCELLED")).toBe(true);
    expect(isCancelledRun("Channel R has 1 frame(s); drizzle needs at least 2 per channel")).toBe(false);
    expect(isCancelledRun("")).toBe(false);
  });
});

describe("manualGains", () => {
  it("refuses a decimal comma and sends no gains, so 2,5 never becomes 25 or 2", () => {
    expect(manualGains({ r: "2,5", g: "1", b: "1" })).toEqual({
      gains: null,
      error: "R gain: Use a dot (.) as the decimal separator",
    });
  });

  it("sends the dot-decimal gains typed in the three fields", () => {
    expect(manualGains({ r: "2.5", g: " 1 ", b: "0.8" })).toEqual({
      gains: { wbR: 2.5, wbG: 1, wbB: 0.8 },
      error: null,
    });
  });

  it("refuses a blank gain instead of sending 0", () => {
    expect(manualGains({ r: "1", g: "", b: "1" })).toEqual({ gains: null, error: "G gain: Enter a number" });
  });

  it("accepts only the factors the backend applies, 0.01 to 100", () => {
    expect(DRIZZLE_GAIN_FIELD).toMatchObject({ min: 0.01, max: 100 });
    expect(manualGains({ r: "0.01", g: "100", b: "1" }).error).toBeNull();
    expect(manualGains({ r: "1", g: "1", b: "0" })).toEqual({ gains: null, error: "B gain: Use a value from 0.01 to 100" });
    expect(manualGains({ r: "100.5", g: "1", b: "1" }).error).toBe("R gain: Use a value from 0.01 to 100");
  });
});
