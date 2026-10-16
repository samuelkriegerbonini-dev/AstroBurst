import { afterEach, describe, expect, it, vi } from "vitest";
import { buildArchivePaletteActions } from "../state/useArchivePaletteActions";
import type { AroundTarget } from "../logic/aroundQuery";

const AROUND: AroundTarget = { raDeg: 274.71, decDeg: -13.83, radiusArcmin: 4.2, label: "673nmos.fits" };

function controller() {
  return {
    searchAround: vi.fn(async () => {}),
    openRomanAccess: vi.fn(),
    toggleView: vi.fn(),
    clearCache: vi.fn(async () => {}),
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("archive palette actions", () => {
  it("offers browse, token and cache in Workspace, without the view toggle or Around", () => {
    vi.stubGlobal("navigator", { platform: "Win32" });
    const actions = buildArchivePaletteActions({ mode: "workspace", setMode: vi.fn(), around: null, controller: controller() });
    expect(actions.map((a) => a.label)).toEqual(["Browse MAST archive", "MAST token", "Clear MAST cache"]);
    expect(actions[0].hint).toBe("Ctrl+2");
    expect(new Set(actions.map((a) => a.id)).size).toBe(actions.length);
  });

  it("adds the gallery or table toggle in Archive and the Around search when the image has a WCS", () => {
    const actions = buildArchivePaletteActions({ mode: "archive", setMode: vi.fn(), around: AROUND, controller: controller() });
    expect(actions.map((a) => a.label)).toEqual(["Browse MAST archive", "Search MAST around this image", "MAST token", "Toggle gallery or table", "Clear MAST cache"]);
  });

  it("switches to Archive before each archive action runs", () => {
    const setMode = vi.fn();
    const ctl = controller();
    const actions = buildArchivePaletteActions({ mode: "workspace", setMode, around: AROUND, controller: ctl });
    const byLabel = (label: string) => actions.find((a) => a.label === label)!;
    byLabel("Browse MAST archive").run();
    expect(setMode).toHaveBeenLastCalledWith("archive");
    byLabel("Search MAST around this image").run();
    expect(ctl.searchAround).toHaveBeenCalledWith(AROUND);
    byLabel("MAST token").run();
    expect(ctl.openRomanAccess).toHaveBeenCalledTimes(1);
    expect(setMode).toHaveBeenCalledTimes(3);
    byLabel("Clear MAST cache").run();
    expect(ctl.clearCache).toHaveBeenCalledTimes(1);
  });

  it("uses the Command key hint on macOS", () => {
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    const actions = buildArchivePaletteActions({ mode: "workspace", setMode: vi.fn(), around: null, controller: controller() });
    expect(actions[0].hint).toBe("⌘2");
  });

  it("toggles the view only through the Archive action", () => {
    const ctl = controller();
    const actions = buildArchivePaletteActions({ mode: "archive", setMode: vi.fn(), around: null, controller: ctl });
    actions.find((a) => a.label === "Toggle gallery or table")!.run();
    expect(ctl.toggleView).toHaveBeenCalledTimes(1);
  });
});
