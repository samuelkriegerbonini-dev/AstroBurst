import { describe, expect, it } from "vitest";
import { regionAt, sedDomain } from "../saguiPlot";

describe("SAGUI scientific viewer", () => {
  it("maps screen coordinates to row-major labels with astronomical Y upward", () => {
    const result = { width: 2, height: 3, labels: [0, 1, 2, 2, 3, 4] };
    expect(regionAt(result, 0.1, 0.1)).toBe(3);
    expect(regionAt(result, 0.9, 0.9)).toBe(1);
    expect(regionAt(result, 0.1, 0.9)).toBe(0);
    expect(regionAt(result, 1, 0)).toBeNull();
    expect(regionAt(result, -0.1, 0.1)).toBeNull();
  });
  it("samples a downsampled label preview by its own size when the backend sends one (PREPUI-08)", () => {
    const preview = { width: 4, height: 2, labels_width: 2, labels_height: 1, labels: [5, 6] };
    expect(regionAt(preview, 0.1, 0.5)).toBe(5);
    expect(regionAt(preview, 0.9, 0.5)).toBe(6);
    expect(regionAt({ width: 4, height: 2, labels: [1, 2, 3, 4, 5, 6, 7, 8] }, 0.9, 0.9)).toBe(4);
  });

  it("includes negative flux and finite uncertainty bounds without fabricating unknown error bars", () => {
    const bounds = sedDomain([{ filter: "A", pivot_um: 2, flux: -2, error: 3 }, { filter: "B", pivot_um: 4, flux: 8, error: null }]);
    expect(bounds.yMin).toBeLessThanOrEqual(-5);
    expect(bounds.yMax).toBeGreaterThanOrEqual(8);
    expect(bounds.xMin).toBeLessThan(2);
    expect(bounds.xMax).toBeGreaterThan(4);
  });
});
