import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Region, RegionStatsResult } from "../../shared/types/regions";

const backend = vi.hoisted(() => ({ stats: vi.fn() }));

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../services/regions", () => ({ regionStats: backend.stats }));

import { hooks } from "../../test/hookRuntime";
import { STATS_DEBOUNCE_MS, useRegionStats } from "../useRegionStats";

const PATH = "C:/data/a.fits";
const REGIONS: Region[] = [
  { id: "r1", shape: { shape: "circle", x: 5, y: 5, r: 2 }, props: { color: null, width: null, text: null, dash: null, include: true }, backgroundId: null },
];
const RESULT = {
  regions: [{ id: "r1", count: 12, mean: 3 }],
  masked: false,
  dq_excluded: null,
  region_excluded: 0,
  elapsed_ms: 1,
} as unknown as RegionStatsResult;

describe("useRegionStats after the file is rewritten", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    hooks.mount();
    backend.stats.mockReset();
  });

  afterEach(() => {
    hooks.mount();
    vi.useRealTimers();
  });

  it("clears the statistics of the earlier revision before the new measurement lands, and keeps them cleared when it fails", async () => {
    backend.stats.mockResolvedValue(RESULT);
    const render = (key: string) =>
      hooks.render(() => useRegionStats(PATH, REGIONS, false, undefined, key), undefined) as ReturnType<typeof useRegionStats>;
    render(PATH);
    await vi.advanceTimersByTimeAsync(STATS_DEBOUNCE_MS + 1);
    expect(render(PATH).stats.size).toBe(1);

    backend.stats.mockRejectedValue(new Error("the file changed on disk"));
    render(`${PATH}#1`);
    const cleared = render(`${PATH}#1`);
    expect(cleared.stats.size).toBe(0);
    expect(cleared.measured).toBeNull();
    await vi.advanceTimersByTimeAsync(STATS_DEBOUNCE_MS + 1);
    const failed = render(`${PATH}#1`);
    expect(failed.error).toBe("the file changed on disk");
    expect(failed.stats.size).toBe(0);
  });
});
