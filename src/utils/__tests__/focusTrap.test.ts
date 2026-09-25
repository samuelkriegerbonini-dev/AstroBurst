import { describe, expect, it } from "vitest";
import { focusTrapTarget } from "../focusTrap";

describe("focusTrapTarget", () => {
  it("wraps Tab from the last element to the first", () => {
    expect(focusTrapTarget(3, 2, false)).toBe(0);
  });

  it("wraps Shift+Tab from the first element to the last", () => {
    expect(focusTrapTarget(3, 0, true)).toBe(2);
  });

  it("leaves Tab between inner elements to the browser", () => {
    expect(focusTrapTarget(3, 0, false)).toBeNull();
    expect(focusTrapTarget(3, 1, false)).toBeNull();
    expect(focusTrapTarget(3, 1, true)).toBeNull();
    expect(focusTrapTarget(3, 2, true)).toBeNull();
  });

  it("pulls focus from outside the trapped elements to the first on Tab and the last on Shift+Tab", () => {
    expect(focusTrapTarget(3, -1, false)).toBe(0);
    expect(focusTrapTarget(3, -1, true)).toBe(2);
  });

  it("keeps a single element focused in both directions", () => {
    expect(focusTrapTarget(1, 0, false)).toBe(0);
    expect(focusTrapTarget(1, 0, true)).toBe(0);
    expect(focusTrapTarget(1, -1, false)).toBe(0);
  });

  it("has no target when nothing is focusable", () => {
    expect(focusTrapTarget(0, -1, false)).toBeNull();
    expect(focusTrapTarget(0, -1, true)).toBeNull();
  });
});
