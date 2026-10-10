import { describe, expect, it, vi } from "vitest";
import {
  DOCK_ONBOARDING_KEY,
  DOCK_ONBOARDING_STEPS,
  DOCK_ONBOARDING_TIP,
  DOCK_ONBOARDING_TITLE,
  markDockOnboardingDone,
  movedToAnotherAnchor,
  onboardingPlacement,
  shouldShowDockOnboarding,
} from "../dockOnboarding";
import { ANCHOR_LABELS, DEFAULT_DOCK_LAYOUT, dockReducer } from "../dockLayout";

function memoryStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  return {
    data,
    getItem: vi.fn((key: string) => data.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => {
      data.set(key, value);
    }),
  };
}

function throwingStorage() {
  return {
    getItem: vi.fn((): string | null => {
      throw new Error("SecurityError");
    }),
    setItem: vi.fn((): void => {
      throw new Error("QuotaExceededError");
    }),
  };
}

const VIEWPORT = { width: 1600, height: 900 };

describe("dock onboarding storage", () => {
  it("uses the versioned key", () => {
    expect(DOCK_ONBOARDING_KEY).toBe("ab.onboarding.dock.v1");
  });

  it("shows when the key is absent", () => {
    const storage = memoryStorage();
    expect(shouldShowDockOnboarding(storage)).toBe(true);
    expect(storage.getItem).toHaveBeenCalledWith("ab.onboarding.dock.v1");
  });

  it("stays hidden once the key is present", () => {
    expect(shouldShowDockOnboarding(memoryStorage({ "ab.onboarding.dock.v1": "done" }))).toBe(false);
    expect(shouldShowDockOnboarding(memoryStorage({ "ab.onboarding.dock.v1": "" }))).toBe(false);
  });

  it("shows when storage is missing or throws, without throwing itself", () => {
    expect(shouldShowDockOnboarding(null)).toBe(true);
    const storage = throwingStorage();
    expect(() => shouldShowDockOnboarding(storage)).not.toThrow();
    expect(shouldShowDockOnboarding(storage)).toBe(true);
  });

  it("writes done under the key", () => {
    const storage = memoryStorage();
    markDockOnboardingDone(storage);
    expect(storage.setItem).toHaveBeenCalledWith("ab.onboarding.dock.v1", "done");
    expect(shouldShowDockOnboarding(storage)).toBe(false);
  });

  it("swallows a throwing or missing storage when marking done", () => {
    const storage = throwingStorage();
    expect(() => markDockOnboardingDone(storage)).not.toThrow();
    expect(storage.setItem).toHaveBeenCalledTimes(1);
    expect(() => markDockOnboardingDone(null)).not.toThrow();
  });
});

describe("dock onboarding move detection", () => {
  it("is true when the tool changed anchor", () => {
    const after = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "right-bottom" });
    expect(movedToAnotherAnchor(DEFAULT_DOCK_LAYOUT, after, "image")).toBe(true);
  });

  it("is false for a reorder inside the anchor or an untouched tool", () => {
    const reordered = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "move", tool: "image", anchor: "right-top", index: 0 });
    expect(reordered).not.toBe(DEFAULT_DOCK_LAYOUT);
    expect(movedToAnotherAnchor(DEFAULT_DOCK_LAYOUT, reordered, "image")).toBe(false);
    expect(movedToAnotherAnchor(DEFAULT_DOCK_LAYOUT, DEFAULT_DOCK_LAYOUT, "image")).toBe(false);
  });
});

describe("dock onboarding content", () => {
  it("has the owner's title and tip", () => {
    expect(DOCK_ONBOARDING_TITLE).toBe("Arrange your tool windows");
    expect(DOCK_ONBOARDING_TIP).toBe("Right-click a tool (or Shift+F10) for Move to…, and Reset Layout puts everything back.");
  });

  it("has two steps, vertical then horizontal, with the verbatim texts", () => {
    expect(DOCK_ONBOARDING_STEPS.map((s) => s.demo)).toEqual(["vertical", "horizontal"]);
    expect(DOCK_ONBOARDING_STEPS[0].title).toBe("Drag vertically");
    expect(DOCK_ONBOARDING_STEPS[0].body).toBe(
      "Drag any tool icon from a side strip and drop it on the top half of either strip. The tool opens as a full-height column beside the viewer.",
    );
    expect(DOCK_ONBOARDING_STEPS[1].title).toBe("Drag horizontally");
    expect(DOCK_ONBOARDING_STEPS[1].body).toBe(
      "Drop it on the bottom half of a strip instead to open it under the viewer. Two bottom tools sit side by side.",
    );
  });

  it("labels the demo with the real anchor names", () => {
    expect(DOCK_ONBOARDING_STEPS[0].label).toBe("Move to Right Top");
    expect(DOCK_ONBOARDING_STEPS[1].label).toBe("Move to Bottom Right");
    expect(DOCK_ONBOARDING_STEPS[0].label).toBe(`Move to ${ANCHOR_LABELS[DOCK_ONBOARDING_STEPS[0].anchor]}`);
    expect(DOCK_ONBOARDING_STEPS[1].label).toBe(`Move to ${ANCHOR_LABELS[DOCK_ONBOARDING_STEPS[1].anchor]}`);
  });
});

describe("onboardingPlacement", () => {
  it("sits in the top-right corner of a wide viewer with a 16 px inset and points right", () => {
    const viewer = { left: 300, top: 100, width: 900, height: 600 };
    expect(onboardingPlacement(viewer, VIEWPORT)).toEqual({ left: 824, top: 116, width: 360, maxHeight: 568, arrow: "right" });
  });

  it("keeps the 360 px cap at the 420 px threshold", () => {
    const viewer = { left: 50, top: 40, width: 420, height: 500 };
    expect(onboardingPlacement(viewer, VIEWPORT)).toEqual({ left: 94, top: 56, width: 360, maxHeight: 468, arrow: "right" });
  });

  it("moves to the top-left with the full available width on a narrow viewer", () => {
    const viewer = { left: 200, top: 80, width: 380, height: 500 };
    expect(onboardingPlacement(viewer, VIEWPORT)).toEqual({ left: 216, top: 96, width: 348, maxHeight: 468, arrow: "none" });
    const justBelow = onboardingPlacement({ left: 0, top: 0, width: 419, height: 400 }, VIEWPORT);
    expect(justBelow.left).toBe(16);
    expect(justBelow.width).toBe(360);
    expect(justBelow.arrow).toBe("none");
  });

  it("never returns a negative width or a height below the floor", () => {
    const tiny = onboardingPlacement({ left: 10, top: 10, width: 20, height: 40 }, VIEWPORT);
    expect(tiny.width).toBe(0);
    expect(tiny.left).toBe(26);
    expect(tiny.maxHeight).toBe(120);
  });

  it("stays inside the window when the viewer reaches past it", () => {
    const placement = onboardingPlacement({ left: 1400, top: 700, width: 600, height: 400 }, VIEWPORT);
    expect(placement.left + placement.width).toBeLessThanOrEqual(VIEWPORT.width);
    expect(placement.top + placement.maxHeight).toBeLessThanOrEqual(VIEWPORT.height);
  });
});
