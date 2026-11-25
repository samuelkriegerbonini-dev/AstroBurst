import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Region } from "../../../shared/types/regions";

const files = vi.hoisted(() => ({ current: [] as { id: string; path: string; reloads?: number }[] }));
const cube = vi.hoisted(() => ({ region: vi.fn(), pixel: vi.fn() }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../regions/ProfilePlot", () => ({ default: () => null }));
vi.mock("../../../hooks/useFileStore", () => ({
  fileStore: { subscribeToStats: () => () => {}, getDoneFiles: () => files.current },
}));
vi.mock("../../../services/cube", () => ({ getCubeSpectrumRegion: cube.region, getCubeSpectrum: cube.pixel }));

import { hooks } from "../../../test/hookRuntime";
import { textOf } from "../../../test/elementTree";
import SpectrumComparisonSection from "../SpectrumComparisonSection";
import { spectrumComparisonStore } from "../../../hooks/useSpectrumComparisonStore";
import { scientificInputRevisionOf } from "../../../utils/pixelSource";
import { COMPARE_DEBOUNCE_MS } from "../../../utils/spectrumCompare";

const CUBE_PATH = "C:/data/cube_s3d.fits";
const REGION: Region = {
  id: "r1",
  shape: { shape: "circle", x: 3, y: 3, r: 2 },
  props: { color: null, width: null, text: null, dash: null, include: true },
  backgroundId: null,
};
const REGION_VERSION = 4;

type Section = (props: Record<string, unknown>) => unknown;
const inner = (SpectrumComparisonSection as unknown as { type: Section }).type;

function show(): string {
  return textOf(
    hooks.render(inner, {
      filePath: CUBE_PATH,
      regionDoc: { regions: [REGION], selectedId: null, version: REGION_VERSION },
      axis: null,
      mode: "channel",
      restUm: null,
      convention: "optical",
      correction: "none",
      correctionResult: null,
      pixelCoord: null,
      view: "sum",
      bunit: "MJy/sr",
      channelCount: 5,
      windows: null,
    }),
  );
}

function commitAt(revision: string): void {
  spectrumComparisonStore.patch(CUBE_PATH, { enabled: true });
  const seq = spectrumComparisonStore.begin(CUBE_PATH, REGION_VERSION, null, revision);
  spectrumComparisonStore.commit(CUBE_PATH, seq, []);
}

describe("SpectrumComparisonSection after the cube is rewritten", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("window", { setTimeout, clearTimeout });
    hooks.mount();
    spectrumComparisonStore.forget(CUBE_PATH);
    cube.region.mockReset().mockReturnValue(new Promise(() => {}));
    cube.pixel.mockReset();
  });

  afterEach(() => {
    hooks.mount();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("marks the spectra of the earlier revision as refreshing and fetches them again", () => {
    const before = { id: "f1", path: CUBE_PATH, reloads: 0 };
    commitAt(scientificInputRevisionOf(before));
    files.current = [before];
    expect(show()).not.toContain("refreshing");
    vi.advanceTimersByTime(COMPARE_DEBOUNCE_MS + 1);
    expect(cube.region).not.toHaveBeenCalled();

    hooks.mount();
    files.current = [{ ...before, reloads: 1 }];
    expect(show()).toContain("refreshing");
    vi.advanceTimersByTime(COMPARE_DEBOUNCE_MS + 1);
    expect(cube.region).toHaveBeenCalledTimes(1);
    expect(cube.region.mock.calls[0][0]).toBe(CUBE_PATH);
  });
});
