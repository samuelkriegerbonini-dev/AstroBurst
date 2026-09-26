import { describe, expect, it } from "vitest";
import { SYNTH_PIXEL_BUDGET, formatByteSize, synthStackEstimate } from "../synthBudget";

describe("synth stack pixel budget", () => {
  it("mirrors the backend budget of 2^30 pixels", () => {
    expect(SYNTH_PIXEL_BUDGET).toBe(1_073_741_824);
  });

  it("accepts a stack exactly at the budget and refuses one frame more with the largest frame count that fits", () => {
    const atLimit = synthStackEstimate(8192, 8192, 16);
    expect(atLimit.pixels).toBe(SYNTH_PIXEL_BUDGET);
    expect(atLimit.overBudgetReason).toBeNull();

    const over = synthStackEstimate(8192, 8192, 17);
    expect(over.overBudgetReason).toContain("17 frames of 8192x8192");
    expect(over.overBudgetReason).toContain("1,073,741,824");
    expect(over.overBudgetReason).toContain("at most 16 frames at this size");
  });

  it("estimates the written size as 32-bit float pixels", () => {
    expect(synthStackEstimate(2048, 2048, 8).bytes).toBe(2048 * 2048 * 8 * 4);
    expect(synthStackEstimate(8192, 8192, 32).bytes).toBe(8_589_934_592);
  });

  it("keeps the default settings well inside the budget", () => {
    expect(synthStackEstimate(2048, 2048, 8).overBudgetReason).toBeNull();
  });
});

describe("formatByteSize", () => {
  it("uses decimal units with one decimal", () => {
    expect(formatByteSize(512)).toBe("512 B");
    expect(formatByteSize(134_217_728)).toBe("134.2 MB");
    expect(formatByteSize(8_589_934_592)).toBe("8.6 GB");
    expect(formatByteSize(4_500)).toBe("4.5 kB");
  });
});
