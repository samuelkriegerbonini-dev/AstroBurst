import { describe, expect, it, vi } from "vitest";
import {
  DOCK_ONBOARDING_KEY,
  DOCK_ONBOARDING_STEPS,
  DOCK_ONBOARDING_TIP,
  DOCK_ONBOARDING_TITLE,
  DOCK_SETTINGS_TIP_KEY,
  DOCK_TIP_IDS,
  DOCK_TIPS,
  DOCK_TOGGLE_TIP_KEY,
  markDockOnboardingDone,
  movedToAnotherAnchor,
  nextOnboardingCard,
  ONBOARDING_INSET_PX,
  ONBOARDING_KEYS,
  ONBOARDING_MIN_HEIGHT_PX,
  ONBOARDING_ORDER,
  onboardingPlacement,
  shouldShowDockOnboarding,
  stripButtonHint,
  tipPlacement,
  viewerToolbarBottom,
  type OnboardingCardId,
  type OnboardingToolbarHost,
} from "../dockOnboarding";
import { ANCHOR_LABELS, DEFAULT_DOCK_LAYOUT, DOCK_TOOL_META, anchorOf, anchorSide, dockReducer } from "../dockLayout";

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
    expect(onboardingPlacement(viewer, VIEWPORT, viewer.top)).toEqual({ left: 824, top: 116, width: 360, maxHeight: 568, arrow: "right" });
  });

  it("keeps the 360 px cap at the 420 px threshold", () => {
    const viewer = { left: 50, top: 40, width: 420, height: 500 };
    expect(onboardingPlacement(viewer, VIEWPORT, viewer.top)).toEqual({ left: 94, top: 56, width: 360, maxHeight: 468, arrow: "right" });
  });

  it("moves to the top-left with the full available width on a narrow viewer", () => {
    const viewer = { left: 200, top: 80, width: 380, height: 500 };
    expect(onboardingPlacement(viewer, VIEWPORT, viewer.top)).toEqual({ left: 216, top: 96, width: 348, maxHeight: 468, arrow: "none" });
    const justBelow = onboardingPlacement({ left: 0, top: 0, width: 419, height: 400 }, VIEWPORT, 0);
    expect(justBelow.left).toBe(16);
    expect(justBelow.width).toBe(360);
    expect(justBelow.arrow).toBe("none");
  });

  it("never returns a negative width or a height below the floor", () => {
    const tiny = onboardingPlacement({ left: 10, top: 10, width: 20, height: 40 }, VIEWPORT, 10);
    expect(tiny.width).toBe(0);
    expect(tiny.left).toBe(26);
    expect(tiny.maxHeight).toBe(120);
  });

  it("stays inside the window when the viewer reaches past it", () => {
    const placement = onboardingPlacement({ left: 1400, top: 700, width: 600, height: 400 }, VIEWPORT, 700);
    expect(placement.left + placement.width).toBeLessThanOrEqual(VIEWPORT.width);
    expect(placement.top + placement.maxHeight).toBeLessThanOrEqual(VIEWPORT.height);
  });
});

describe("onboardingPlacement under the viewer toolbar", () => {
  const viewer = { left: 300, top: 100, width: 900, height: 600 };

  it("starts the card an inset below the bottom of the toolbar rows", () => {
    expect(onboardingPlacement(viewer, VIEWPORT, 190)).toEqual({ left: 824, top: 206, width: 360, maxHeight: 478, arrow: "right" });
  });

  it("keeps the viewer inset when the toolbar ends above the viewer", () => {
    expect(onboardingPlacement(viewer, VIEWPORT, 40)).toEqual(onboardingPlacement(viewer, VIEWPORT, viewer.top));
    expect(onboardingPlacement(viewer, VIEWPORT, 40).top).toBe(116);
  });

  it("keeps the floor height inside the window when the toolbar sits low", () => {
    const placement = onboardingPlacement(viewer, VIEWPORT, 850);
    expect(placement.top + placement.maxHeight).toBeLessThanOrEqual(VIEWPORT.height);
    expect(placement.maxHeight).toBe(ONBOARDING_MIN_HEIGHT_PX);
  });
});

describe("viewerToolbarBottom", () => {
  function box(top: number, bottom: number) {
    return { getBoundingClientRect: () => ({ top, bottom }) };
  }

  function host({ top, viewportTop, toolbars }: { top: number; viewportTop: number | null; toolbars: ReturnType<typeof box>[] }): OnboardingToolbarHost {
    return {
      ...box(top, 900),
      querySelector: (selector) => (selector === "[data-dock-viewport]" && viewportTop !== null ? box(viewportTop, 900) : null),
      querySelectorAll: (selector) => (selector === ".ab-viewer-toolbar" ? toolbars : []),
    };
  }

  it("is the bottom of the lowest viewer toolbar row", () => {
    expect(viewerToolbarBottom(host({ top: 60, viewportTop: 92, toolbars: [box(126, 158)] }))).toBe(158);
    expect(viewerToolbarBottom(host({ top: 60, viewportTop: 92, toolbars: [box(126, 158), box(126, 190)] }))).toBe(190);
  });

  it("is the bottom of the Preview header row, the viewport top, without a viewer toolbar", () => {
    expect(viewerToolbarBottom(host({ top: 60, viewportTop: 92, toolbars: [] }))).toBe(92);
  });

  it("is the viewer top before the preview has rendered, and ignores a hidden toolbar", () => {
    expect(viewerToolbarBottom(host({ top: 60, viewportTop: null, toolbars: [] }))).toBe(60);
    expect(viewerToolbarBottom(host({ top: 60, viewportTop: 92, toolbars: [box(0, 0)] }))).toBe(92);
  });
});

describe("strip button hints", () => {
  it("tells how a closed tool opens and closes", () => {
    expect(stripButtonHint("Compose", { open: false, disabled: false })).toBe("Compose: click to open, click again to close");
    expect(stripButtonHint("Settings", { open: false, disabled: false })).toBe("Settings: click to open, click again to close");
  });

  it("tells that a click closes an open tool", () => {
    expect(stripButtonHint("Compose", { open: true, disabled: false })).toBe("Compose is open: click to close");
  });

  it("asks for a file first on a disabled tool, open or not", () => {
    expect(stripButtonHint("Compose", { open: false, disabled: true })).toBe("Compose: open a file first");
    expect(stripButtonHint("Compose", { open: true, disabled: true })).toBe("Compose: open a file first");
  });

  it("keeps the label it is given, such as the Log entry count", () => {
    expect(stripButtonHint("Log (3 entries)", { open: false, disabled: false })).toBe("Log (3 entries): click to open, click again to close");
  });
});

describe("dock tips content and keys", () => {
  it("has the toggle tip on Compose with the owner's wording", () => {
    expect(DOCK_TIPS.toggle).toEqual({
      id: "toggle",
      tool: "compose",
      title: "Open and close tools",
      body: "Click any tool in this bar to open it, and click it again to close it.",
    });
  });

  it("has the settings tip on Config naming the Settings sections", () => {
    expect(DOCK_TIPS.settings).toEqual({
      id: "settings",
      tool: "config",
      title: "Settings",
      body: "Click Config for the settings: astrometry.net API key, plate solving, catalogs, archive downloads and output storage. Click it again to close them.",
    });
    expect(DOCK_TIPS.settings.body).toContain(`Click ${DOCK_TOOL_META.config.shortLabel} `);
  });

  it("anchors the toggle tip on the left strip and the settings tip on the right strip by default", () => {
    expect(anchorSide(anchorOf(DEFAULT_DOCK_LAYOUT, DOCK_TIPS.toggle.tool))).toBe("left");
    expect(anchorSide(anchorOf(DEFAULT_DOCK_LAYOUT, DOCK_TIPS.settings.tool))).toBe("right");
  });

  it("queues the toggle tip, the settings tip, then the drag tour, each under its own versioned key", () => {
    expect(DOCK_TIP_IDS).toEqual(["toggle", "settings"]);
    expect(ONBOARDING_ORDER).toEqual(["toggle", "settings", "tour"]);
    expect(DOCK_TOGGLE_TIP_KEY).toBe("ab.onboarding.dockToggle.v1");
    expect(DOCK_SETTINGS_TIP_KEY).toBe("ab.onboarding.settings.v1");
    expect(ONBOARDING_KEYS).toEqual({ toggle: DOCK_TOGGLE_TIP_KEY, settings: DOCK_SETTINGS_TIP_KEY, tour: DOCK_ONBOARDING_KEY });
  });

  it("reads and writes the key it is given, the drag tour key by default", () => {
    const storage = memoryStorage({ [DOCK_ONBOARDING_KEY]: "done" });
    expect(shouldShowDockOnboarding(storage, DOCK_TOGGLE_TIP_KEY)).toBe(true);
    expect(storage.getItem).toHaveBeenLastCalledWith(DOCK_TOGGLE_TIP_KEY);
    markDockOnboardingDone(storage, DOCK_SETTINGS_TIP_KEY);
    expect(storage.setItem).toHaveBeenCalledWith(DOCK_SETTINGS_TIP_KEY, "done");
    expect(shouldShowDockOnboarding(storage, DOCK_SETTINGS_TIP_KEY)).toBe(false);
    expect(shouldShowDockOnboarding(storage, DOCK_TOGGLE_TIP_KEY)).toBe(true);
    expect(shouldShowDockOnboarding(storage)).toBe(false);
    const throwing = throwingStorage();
    expect(shouldShowDockOnboarding(throwing, DOCK_TOGGLE_TIP_KEY)).toBe(true);
    expect(() => markDockOnboardingDone(throwing, DOCK_TOGGLE_TIP_KEY)).not.toThrow();
  });
});

describe("nextOnboardingCard", () => {
  const ALL_READY = { toggle: true, settings: true, tour: true };
  const done = (...cards: OnboardingCardId[]) => new Set<OnboardingCardId>(cards);

  it("follows the queue order while every card is ready", () => {
    expect(nextOnboardingCard(done(), ALL_READY)).toBe("toggle");
    expect(nextOnboardingCard(done("toggle"), ALL_READY)).toBe("settings");
    expect(nextOnboardingCard(done("toggle", "settings"), ALL_READY)).toBe("tour");
    expect(nextOnboardingCard(done("toggle", "settings", "tour"), ALL_READY)).toBeNull();
  });

  it("skips a card that is not ready without dropping it", () => {
    expect(nextOnboardingCard(done(), { toggle: false, settings: true, tour: true })).toBe("settings");
    expect(nextOnboardingCard(done(), { toggle: false, settings: false, tour: true })).toBe("tour");
    expect(nextOnboardingCard(done("settings", "tour"), { toggle: false, settings: true, tour: true })).toBeNull();
    expect(nextOnboardingCard(done("settings", "tour"), ALL_READY)).toBe("toggle");
  });

  it("shows nothing when nothing is ready", () => {
    expect(nextOnboardingCard(done(), { toggle: false, settings: false, tour: false })).toBeNull();
  });
});

describe("tipPlacement", () => {
  const LEFT_BUTTON = { left: 3, top: 820, width: 38, height: 36 };
  const RIGHT_BUTTON = { left: 1559, top: 700, width: 38, height: 36 };

  it("sits to the right of a left-strip button, centred on it, with the arrow on its left edge", () => {
    const button = { left: 3, top: 400, width: 38, height: 36 };
    expect(tipPlacement(button, "left", VIEWPORT, 120)).toEqual({ left: 51, top: 358, width: 280, maxHeight: 868, arrow: "left", arrowY: 60 });
  });

  it("sits to the left of a right-strip button, centred on it, with the arrow on its right edge", () => {
    expect(tipPlacement(RIGHT_BUTTON, "right", VIEWPORT, 120)).toEqual({ left: 1269, top: 658, width: 280, maxHeight: 868, arrow: "right", arrowY: 60 });
  });

  it("stays an inset inside the window and moves the arrow to keep pointing at the button", () => {
    const low = tipPlacement(LEFT_BUTTON, "left", VIEWPORT, 120);
    expect(low).toEqual({ left: 51, top: 764, width: 280, maxHeight: 868, arrow: "left", arrowY: 74 });
    expect(low.top + 120).toBe(VIEWPORT.height - ONBOARDING_INSET_PX);
    const high = tipPlacement({ left: 1559, top: 30, width: 38, height: 36 }, "right", VIEWPORT, 120);
    expect(high.top).toBe(ONBOARDING_INSET_PX);
    expect(high.arrowY).toBe(32);
  });

  it("keeps the arrow off the rounded corners when the button sits past the card", () => {
    expect(tipPlacement({ left: 1559, top: 0, width: 38, height: 10 }, "right", VIEWPORT, 120).arrowY).toBe(14);
    expect(tipPlacement({ left: 3, top: 895, width: 38, height: 10 }, "left", VIEWPORT, 120).arrowY).toBe(106);
  });

  it("caps a tall card at the window height less the insets", () => {
    expect(tipPlacement(LEFT_BUTTON, "left", VIEWPORT, 2000)).toEqual({ left: 51, top: 16, width: 280, maxHeight: 868, arrow: "left", arrowY: 822 });
  });

  it("narrows rather than covering its button in a small window", () => {
    const narrow = { width: 300, height: 400 };
    const left = tipPlacement({ left: 3, top: 200, width: 38, height: 36 }, "left", narrow, 120);
    expect(left.left).toBe(51);
    expect(left.width).toBe(233);
    const right = tipPlacement({ left: 259, top: 200, width: 38, height: 36 }, "right", narrow, 120);
    expect(right).toMatchObject({ left: 16, width: 233, arrow: "right" });
    expect(right.left + right.width).toBeLessThanOrEqual(259);
    const tiny = tipPlacement({ left: 3, top: 20, width: 38, height: 36 }, "left", { width: 60, height: 100 }, 120);
    expect(tiny.width).toBe(0);
    expect(tiny.left).toBeGreaterThanOrEqual(41);
  });

  it("never overlaps its anchor button horizontally", () => {
    for (const viewport of [VIEWPORT, { width: 900, height: 500 }, { width: 120, height: 300 }]) {
      const left = { left: 3, top: 100, width: 38, height: 36 };
      const right = { left: viewport.width - 41, top: 100, width: 38, height: 36 };
      expect(tipPlacement(left, "left", viewport, 120).left).toBeGreaterThanOrEqual(left.left + left.width);
      const placed = tipPlacement(right, "right", viewport, 120);
      expect(placed.left + placed.width).toBeLessThanOrEqual(right.left);
    }
  });
});
