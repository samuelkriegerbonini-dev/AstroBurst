import { describe, expect, it, vi } from "vitest";
import type { ForcedPhotometryResult } from "../../shared/types/forcedPhotometry";
import type { ResolutionCheck, ResolutionRequest, ResolutionVerdict } from "../../shared/types/resolution";
import { EMPTY_FORCED_SIZES, FORCED_SIZE_BATCH, ForcedSizeStore } from "../useForcedSizes";

const req = (n: number) => ({ source_fwhm_arcsec: n }) as ResolutionRequest;
const answer = (verdict: ResolutionVerdict | null) => ({ verdict }) as ResolutionCheck;
const RESULT = { bands: [], sources: [] } as unknown as ForcedPhotometryResult;
const OTHER = { bands: [], sources: [] } as unknown as ForcedPhotometryResult;
const PSF_TEXT = "0.976 arcsec (70 stars of this image)";
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

const VERDICT_OF: Record<number, ResolutionVerdict> = { 1: "unresolved", 2: "marginal", 3: "resolved" };

describe("ForcedSizeStore", () => {
  it("checks the requests in order and keeps a verdict per row", async () => {
    const check = vi.fn(async (request: ResolutionRequest) => answer(VERDICT_OF[request.source_fwhm_arcsec as number]));
    const store = new ForcedSizeStore(check);
    await store.run({ result: RESULT, band: 1, psfText: PSF_TEXT, requests: [req(1), null, req(2), req(3)] });
    expect(store.getSnapshot()).toEqual({
      result: RESULT,
      band: 1,
      psfText: PSF_TEXT,
      verdicts: ["unresolved", null, "marginal", "resolved"],
      checked: 3,
      total: 3,
      running: false,
      error: null,
    });
    expect(store.getSnapshot().result).toBe(RESULT);
    expect(check).toHaveBeenCalledTimes(3);
    expect(check.mock.calls.map(([r]) => r.source_fwhm_arcsec)).toEqual([1, 2, 3]);
  });

  it("a check without a verdict leaves the row empty", async () => {
    const store = new ForcedSizeStore(async (request) => answer(request.source_fwhm_arcsec === 2 ? null : "resolved"));
    await store.run({ result: RESULT, band: 0, psfText: PSF_TEXT, requests: [req(1), req(2), req(3)] });
    const state = store.getSnapshot();
    expect(state.verdicts).toEqual(["resolved", null, "resolved"]);
    expect(state.checked).toBe(3);
    expect(state.total).toBe(3);
  });

  it("sends sixteen requests at a time", async () => {
    expect(FORCED_SIZE_BATCH).toBe(16);
    let inFlight = 0;
    let maxInFlight = 0;
    const store = new ForcedSizeStore(async () => {
      inFlight++;
      maxInFlight = Math.max(maxInFlight, inFlight);
      await tick();
      inFlight--;
      return answer("unresolved");
    });
    const listener = vi.fn();
    store.subscribe(listener);
    const done = store.run({ result: RESULT, band: 0, psfText: PSF_TEXT, requests: Array.from({ length: 40 }, (_, i) => req(i + 1)) });
    const started = store.getSnapshot();
    expect(started.running).toBe(true);
    expect(started.checked).toBe(0);
    expect(started.total).toBe(40);
    expect(started.verdicts).toEqual(Array.from({ length: 40 }, () => null));
    await done;
    expect(maxInFlight).toBe(16);
    expect(store.getSnapshot().checked).toBe(40);
    expect(store.getSnapshot().running).toBe(false);
    expect(listener.mock.calls.length).toBeLessThanOrEqual(5);
  });

  it("a failed check stops the run and keeps what it has", async () => {
    const store = new ForcedSizeStore(async (request) => {
      if ((request.source_fwhm_arcsec as number) > FORCED_SIZE_BATCH) throw new Error("psf_fwhm_arcsec must be a positive finite number");
      return answer("marginal");
    });
    await store.run({ result: RESULT, band: 0, psfText: PSF_TEXT, requests: Array.from({ length: 40 }, (_, i) => req(i + 1)) });
    const state = store.getSnapshot();
    expect(state.running).toBe(false);
    expect(state.error).toBe("psf_fwhm_arcsec must be a positive finite number");
    expect(state.verdicts.slice(0, 16)).toEqual(Array.from({ length: 16 }, () => "marginal"));
    expect(state.verdicts.slice(16)).toEqual(Array.from({ length: 24 }, () => null));
    expect(state.checked).toBe(16);
  });

  it("a newer run drops the answers of an older one, and clear empties the store", async () => {
    const pending: { request: ResolutionRequest; resolve: (check: ResolutionCheck) => void }[] = [];
    const store = new ForcedSizeStore(
      (request) => new Promise<ResolutionCheck>((resolve) => void pending.push({ request, resolve })),
    );
    const runA = store.run({ result: RESULT, band: 0, psfText: "A", requests: [req(1), req(2)] });
    const runB = store.run({ result: OTHER, band: 2, psfText: "B", requests: [req(3)] });
    expect(pending).toHaveLength(3);
    pending[0].resolve(answer("resolved"));
    pending[1].resolve(answer("resolved"));
    await runA;
    expect(store.getSnapshot().result).toBe(OTHER);
    expect(store.getSnapshot().verdicts).toEqual([null]);
    expect(store.getSnapshot().running).toBe(true);
    pending[2].resolve(answer("unresolved"));
    await runB;
    expect(store.getSnapshot()).toEqual({
      result: OTHER,
      band: 2,
      psfText: "B",
      verdicts: ["unresolved"],
      checked: 1,
      total: 1,
      running: false,
      error: null,
    });
    expect(store.getSnapshot().result).toBe(OTHER);

    store.clear();
    expect(store.getSnapshot()).toBe(EMPTY_FORCED_SIZES);
    expect(store.getSnapshot()).toBe(store.getSnapshot());
    expect(EMPTY_FORCED_SIZES).toEqual({
      result: null,
      band: null,
      psfText: null,
      verdicts: [],
      checked: 0,
      total: 0,
      running: false,
      error: null,
    });
    expect(Object.isFrozen(EMPTY_FORCED_SIZES)).toBe(true);
  });
});
