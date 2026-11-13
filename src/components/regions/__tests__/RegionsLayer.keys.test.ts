import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode, RefObject } from "react";
import { hooks } from "../../../test/hookRuntime";
import { regionStore } from "../../../utils/regionStore";
import { DEFAULT_REGION_PROPS } from "../../../utils/regionPersistence";
import { appModeStore } from "../../../hooks/useAppMode";
import type { RegionShape } from "../../../shared/types/regions";

vi.mock("react", async (importOriginal) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await importOriginal());
});

vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => "C:/data/field.fits" }));

import RegionsLayer from "../RegionsLayer";

const KEY = "C:/data/field.fits";

class FakeElement {
  readonly isContentEditable = false;
  private readonly children: FakeElement[] = [];

  constructor(
    readonly tagName: string,
    private readonly parent: FakeElement | null = null,
    private readonly modal = false,
  ) {
    parent?.children.push(this);
  }

  contains(other: unknown): boolean {
    return other === this || this.children.some((child) => child.contains(other));
  }

  closest(selector: string): FakeElement | null {
    if (this.modal && selector.includes("aria-modal")) return this;
    return this.parent?.closest(selector) ?? null;
  }
}

const body = new FakeElement("BODY");
const workspace = new FakeElement("DIV", body);
const cubeViewport = new FakeElement("DIV", workspace);
const viewer = new FakeElement("DIV", cubeViewport);
const filesList = new FakeElement("DIV", workspace);
const dockButton = new FakeElement("BUTTON", workspace);
const searchInput = new FakeElement("INPUT", workspace);
const shortcutSheet = new FakeElement("DIV", body, true);
const sheetCloseButton = new FakeElement("BUTTON", shortcutSheet);

type KeyListener = (e: { key: string; target: unknown; preventDefault: () => void }) => void;
const keydown = new Set<KeyListener>();

beforeEach(() => {
  keydown.clear();
  vi.stubGlobal("HTMLElement", FakeElement);
  vi.stubGlobal("window", {
    devicePixelRatio: 1,
    addEventListener: (type: string, listener: KeyListener) => {
      if (type === "keydown") keydown.add(listener);
    },
    removeEventListener: (type: string, listener: KeyListener) => {
      if (type === "keydown") keydown.delete(listener);
    },
  });
  hooks.mount();
  appModeStore.set("workspace");
  regionStore.clear(KEY);
  regionStore.clearDraft();
  regionStore.setTool("none");
  regionStore.flush();
});

afterEach(() => {
  hooks.mount();
  appModeStore.set("workspace");
  regionStore.flush();
  vi.unstubAllGlobals();
});

function renderLayer(): void {
  const containerRef = { current: viewer } as unknown as RefObject<HTMLDivElement | null>;
  hooks.render((RegionsLayer as unknown as { type: (props: unknown) => ReactNode }).type, {
    containerRef,
    transform: { scale: 1, x: 0, y: 0 },
    renderW: 100,
    renderH: 100,
    fitsW: 100,
    fitsH: 100,
    enabled: true,
  });
  expect(keydown.size).toBe(1);
}

function press(key: string, target: FakeElement): boolean {
  let prevented = false;
  for (const listener of [...keydown]) listener({ key, target, preventDefault: () => void (prevented = true) });
  return prevented;
}

function seedSourceWithBackground(): void {
  regionStore.add(KEY, { id: "src", shape: { shape: "circle", x: 50, y: 50, r: 5 }, props: { ...DEFAULT_REGION_PROPS }, backgroundId: null });
  regionStore.add(KEY, { id: "bg", shape: { shape: "annulus", x: 50, y: 50, r_inner: 8, r_outer: 12 }, props: { ...DEFAULT_REGION_PROPS }, backgroundId: null });
  regionStore.setBackground(KEY, "src", "bg");
  regionStore.select(KEY, "bg");
}

function regionIds(): string[] {
  return regionStore.getDoc(KEY).regions.map((r) => r.id);
}

const TRIANGLE: RegionShape = { shape: "polygon", points: [[10, 10], [40, 10], [25, 30]] };

describe("RegionsLayer Delete and Backspace with the focus outside the image", () => {
  it("keeps the selected annulus and the source's background link when Delete is pressed on the Files list", () => {
    seedSourceWithBackground();
    renderLayer();
    expect(press("Delete", filesList)).toBe(false);
    expect(regionIds()).toEqual(["src", "bg"]);
    expect(regionStore.getDoc(KEY).regions.find((r) => r.id === "src")?.backgroundId).toBe("bg");
    expect(regionStore.getDoc(KEY).selectedId).toBe("bg");
  });

  it("keeps the selected annulus when Backspace is pressed on a dock button", () => {
    seedSourceWithBackground();
    renderLayer();
    press("Backspace", dockButton);
    expect(regionIds()).toEqual(["src", "bg"]);
  });

  it("keeps the selected region when Delete is pressed on the shortcut sheet", () => {
    seedSourceWithBackground();
    renderLayer();
    press("Delete", shortcutSheet);
    expect(regionIds()).toEqual(["src", "bg"]);
  });

  it("keeps the selected region of the hidden workspace when Delete is pressed on the page body in the Archive", () => {
    seedSourceWithBackground();
    renderLayer();
    appModeStore.set("archive");
    press("Delete", body);
    expect(regionIds()).toEqual(["src", "bg"]);
  });

  it("ignores Delete typed in an input", () => {
    seedSourceWithBackground();
    renderLayer();
    press("Delete", searchInput);
    expect(regionIds()).toEqual(["src", "bg"]);
  });
});

describe("RegionsLayer Delete and Backspace with the focus on the image", () => {
  it("deletes the selected region after a click on a 2D image, which leaves the focus on the page body", () => {
    seedSourceWithBackground();
    renderLayer();
    expect(press("Delete", body)).toBe(true);
    expect(regionIds()).toEqual(["src"]);
    expect(regionStore.getDoc(KEY).regions[0].backgroundId).toBeNull();
  });

  it("deletes the selected region from the focused cube viewport, which holds the image", () => {
    seedSourceWithBackground();
    renderLayer();
    expect(press("Backspace", cubeViewport)).toBe(true);
    expect(regionIds()).toEqual(["src"]);
  });
});

describe("RegionsLayer Esc and Enter while the shortcut sheet has the focus", () => {
  it("keeps the polygon draft when Esc closes the sheet", () => {
    regionStore.setTool("polygon");
    renderLayer();
    regionStore.setDraft(KEY, TRIANGLE);
    press("Escape", shortcutSheet);
    expect(regionStore.getDraftFor(KEY)).toEqual(TRIANGLE);
    expect(regionStore.getTool()).toBe("polygon");
  });

  it("keeps the region tool when Esc closes the sheet without a draft", () => {
    regionStore.setTool("polygon");
    renderLayer();
    press("Escape", shortcutSheet);
    expect(regionStore.getTool()).toBe("polygon");
  });

  it("keeps the selected region when Esc closes the sheet", () => {
    regionStore.setTool("select");
    seedSourceWithBackground();
    renderLayer();
    press("Escape", sheetCloseButton);
    expect(regionStore.getDoc(KEY).selectedId).toBe("bg");
    expect(regionStore.getTool()).toBe("select");
  });

  it("does not close the polygon when Enter activates a button of the sheet", () => {
    regionStore.setTool("polygon");
    renderLayer();
    regionStore.setDraft(KEY, TRIANGLE);
    expect(press("Enter", sheetCloseButton)).toBe(false);
    expect(regionStore.getDraftFor(KEY)).toEqual(TRIANGLE);
    expect(regionIds()).toEqual([]);
  });
});

describe("RegionsLayer Esc and Enter outside a dialog", () => {
  it("clears the polygon draft on Esc, then the selection, then the tool", () => {
    regionStore.setTool("polygon");
    seedSourceWithBackground();
    renderLayer();
    regionStore.setDraft(KEY, TRIANGLE);
    press("Escape", body);
    expect(regionStore.getDraftFor(KEY)).toBeNull();
    expect(regionStore.getDoc(KEY).selectedId).toBe("bg");
    press("Escape", dockButton);
    expect(regionStore.getDoc(KEY).selectedId).toBeNull();
    press("Escape", body);
    expect(regionStore.getTool()).toBe("none");
  });

  it("closes a polygon of three vertices on Enter", () => {
    regionStore.setTool("polygon");
    renderLayer();
    regionStore.setDraft(KEY, TRIANGLE);
    expect(press("Enter", body)).toBe(true);
    expect(regionStore.getDraftFor(KEY)).toBeNull();
    expect(regionStore.getDoc(KEY).regions.map((r) => r.shape)).toEqual([TRIANGLE]);
  });
});
