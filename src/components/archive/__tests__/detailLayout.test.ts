import { describe, expect, it } from "vitest";
import { DETAIL_WIDTH_DEFAULT, DETAIL_WIDTH_MAX, DETAIL_WIDTH_MIN, NARROW_BELOW_PX, clampDetailWidth, isNarrow, keyResize, resizeWidth } from "../logic/detailLayout";

describe("detail pane size", () => {
  it("defaults to 420 px within 360 to 640 px", () => {
    expect([DETAIL_WIDTH_DEFAULT, DETAIL_WIDTH_MIN, DETAIL_WIDTH_MAX]).toEqual([420, 360, 640]);
    expect(clampDetailWidth(300)).toBe(360);
    expect(clampDetailWidth(700)).toBe(640);
    expect(clampDetailWidth(512.6)).toBe(513);
    expect(clampDetailWidth(Number.NaN)).toBe(420);
  });

  it("grows when the handle on its left edge is dragged to the left", () => {
    expect(resizeWidth(420, 800, 760)).toBe(460);
    expect(resizeWidth(420, 800, 900)).toBe(360);
    expect(resizeWidth(600, 800, 500)).toBe(640);
  });

  it("moves 16 px per arrow key and resets with Home", () => {
    expect(keyResize(420, "ArrowLeft")).toBe(436);
    expect(keyResize(420, "ArrowRight")).toBe(404);
    expect(keyResize(632, "ArrowLeft")).toBe(640);
    expect(keyResize(500, "Home")).toBe(420);
    expect(keyResize(500, "Enter")).toBeNull();
  });
});

describe("narrow window", () => {
  it("turns the pane into a drawer below 1100 px", () => {
    expect(NARROW_BELOW_PX).toBe(1100);
    expect(isNarrow(1099)).toBe(true);
    expect(isNarrow(900)).toBe(true);
    expect(isNarrow(1100)).toBe(false);
    expect(isNarrow(0)).toBe(false);
  });
});
