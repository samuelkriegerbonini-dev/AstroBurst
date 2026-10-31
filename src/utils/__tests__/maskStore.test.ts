import { describe, expect, it, vi } from "vitest";
import { MASK_MARK_DEFAULT, MASK_OPACITY_DEFAULT, MASK_TINT_DEFAULT, MaskStoreCore, listedMasks, type MaskHistorySource } from "../maskStore";
import { COMPOSITE_RUN_KEY } from "../compositeChain";
import { EMPTY_HISTORY_SNAPSHOT, type HistorySnapshot } from "../historyStore";
import type { HistoryDoc, HistoryState } from "../../shared/types/ops";
import type { MaskCreateResult, MaskSpec } from "../../shared/types/masks";

const FILE = "C:\\data\\jw02739-o001_t001_nircam_clear-f444w_i2d.fits";
const OTHER = "C:\\data\\673nmos.fits";

const RANGE: MaskSpec = { kind: "range", lo: 112, hi: 124, fuzz: 4, feather_px: 3, invert: false };
const STARS: MaskSpec = { kind: "stars", sigma: 5, config: { growth_factor: 1.5, softness: 2 }, feather_px: 0, invert: true };
const LUMINANCE: MaskSpec = { kind: "luminance", auto_stf: true, feather_px: 0, invert: false };

function doc(masks: Record<string, unknown>, deleted?: string[]): Pick<HistoryDoc, "masks" | "deleted_masks"> {
  return deleted ? { masks, deleted_masks: deleted } : { masks };
}

function created(maskId: string, recipe: MaskSpec, coverage: number, stars: number | null = null): MaskCreateResult {
  return { mask_id: maskId, kind: recipe.kind, coverage, dims: [2048, 2048], stars, recipe };
}

function historyState(masks: Record<string, unknown>, deleted?: string[]): HistoryState {
  return {
    image_id: "3f9c0a1e5b7d2c44",
    history_dir: "C:/app/history/3f9c0a1e5b7d2c44",
    root_path: FILE,
    doc: { ...({} as HistoryDoc), ...doc(masks, deleted) },
    read_only: null,
    fingerprint_matches: true,
    busy: false,
  };
}

function snapshotOf(state: HistoryState | null, imageKey = FILE, error: string | null = null): HistorySnapshot {
  return { ...EMPTY_HISTORY_SNAPSHOT, fileKey: `7|${imageKey}`, scope: { imageKey, basePath: null }, state, error };
}

function fakeHistory(initial: HistorySnapshot): MaskHistorySource & { publish: (next: HistorySnapshot) => void } {
  let snapshot = initial;
  const listeners = new Set<() => void>();
  return {
    get: () => snapshot,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    publish: (next) => {
      snapshot = next;
      listeners.forEach((l) => l());
    },
  };
}

describe("mask store", () => {
  it("mask_store_lists_the_doc_masks_minus_deleted_masks", () => {
    const listed = listedMasks(doc({ m01: RANGE, m02: STARS, m10: LUMINANCE, m03: { kind: "brush" }, m04: "m01" }, ["m02"]));
    expect(listed.map((m) => m.id)).toEqual(["m01", "m10"]);
    expect(listed[0]).toEqual({ id: "m01", kind: "range", recipe: RANGE, coverage: null, stars: null, dims: null });
    expect(listed[1].kind).toBe("luminance");
    expect(listedMasks(doc({ m02: STARS }))).toHaveLength(1);
    expect(listedMasks(null)).toEqual([]);

    const store = new MaskStoreCore();
    store.syncDoc(FILE, doc({ m01: RANGE, m02: STARS }, ["m02"]));
    expect(store.masksOf(FILE).map((m) => m.id)).toEqual(["m01"]);
    expect(store.masksOf(OTHER)).toEqual([]);
    expect(store.masksOf(null)).toBe(store.masksOf(OTHER));
  });

  it("mask_store_keeps_coverage_from_create_until_the_recipe_changes", () => {
    const store = new MaskStoreCore();
    store.recordCreated(FILE, created("m02", STARS, 0.0123, 412));
    store.recordCreated(FILE, created("m01", RANGE, 0.25));
    expect(store.masksOf(FILE).map((m) => [m.id, m.coverage, m.stars])).toEqual([
      ["m01", 0.25, null],
      ["m02", 0.0123, 412],
    ]);
    const listed = store.masksOf(FILE);
    store.syncDoc(FILE, doc({ m01: RANGE, m02: STARS }));
    expect(store.masksOf(FILE)).toBe(listed);

    store.syncDoc(FILE, doc({ m01: LUMINANCE, m02: STARS }));
    expect(store.masksOf(FILE).map((m) => [m.id, m.kind, m.coverage, m.dims])).toEqual([
      ["m01", "luminance", null, null],
      ["m02", "stars", 0.0123, [2048, 2048]],
    ]);
  });

  it("mask_store_clears_the_active_mask_when_it_leaves_the_list", () => {
    const store = new MaskStoreCore();
    store.recordCreated(FILE, created("m01", RANGE, 0.25));
    store.recordCreated(FILE, created("m02", STARS, 0.01, 40));
    store.setActive(FILE, "m09");
    expect(store.activeOf(FILE)).toBeNull();
    store.setActive(FILE, "m02");
    expect(store.activeOf(FILE)).toBe("m02");
    expect(store.activeOf(OTHER)).toBeNull();

    store.recordDeleted(FILE, { mask_id: "m02", recipe: STARS, used_by: ["n0001"], kept_for_replay: true });
    expect(store.masksOf(FILE).map((m) => m.id)).toEqual(["m01"]);
    expect(store.activeOf(FILE)).toBeNull();

    store.setActive(FILE, "m01");
    store.syncDoc(FILE, doc({ m01: LUMINANCE }));
    expect(store.activeOf(FILE)).toBeNull();

    store.setActive(FILE, "m01");
    store.syncDoc(FILE, doc({ m01: LUMINANCE }, ["m01"]));
    expect(store.masksOf(FILE)).toEqual([]);
    expect(store.activeOf(FILE)).toBeNull();
  });

  it("mask_store_defaults_to_red_at_045_without_display_invert", () => {
    const store = new MaskStoreCore();
    expect(MASK_OPACITY_DEFAULT).toBe(0.45);
    expect(MASK_TINT_DEFAULT).toEqual([255, 0, 0]);
    expect(MASK_MARK_DEFAULT).toBe("nonzero");
    expect(store.get().style).toEqual({ opacity: 0.45, tint: [255, 0, 0], displayInvert: false });
    expect(store.get().mark).toBe("nonzero");

    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);
    const before = store.get();
    store.setStyle({ opacity: 0.45 });
    store.setMark("nonzero");
    store.setActive(FILE, null);
    expect(store.get()).toBe(before);
    expect(listener).not.toHaveBeenCalled();

    store.setStyle({ displayInvert: true, opacity: 1.7 });
    expect(store.get().style).toEqual({ opacity: 1, tint: [255, 0, 0], displayInvert: true });
    store.setStyle({ opacity: -2, tint: [0, 255, 255] });
    expect(store.get().style).toEqual({ opacity: 0, tint: [0, 255, 255], displayInvert: true });
    store.setStyle({ opacity: 0.6 });
    store.setStyle({ opacity: Number.NaN });
    expect(store.get().style.opacity).toBe(0.6);
    store.setMark("range");
    expect(store.get().mark).toBe("range");
    expect(listener).toHaveBeenCalledTimes(4);
    unsubscribe();
    store.setMark("nan");
    expect(listener).toHaveBeenCalledTimes(4);
  });

  it("mask_store_follows_the_history_snapshot_of_the_shown_image", () => {
    const first = historyState({ m01: RANGE, m02: STARS }, ["m02"]);
    const history = fakeHistory(snapshotOf(first));
    const store = new MaskStoreCore(history);
    expect(store.masksOf(FILE).map((m) => m.id)).toEqual(["m01"]);

    store.recordCreated(FILE, created("m03", LUMINANCE, 0.5));
    history.publish(snapshotOf(first, FILE, "nothing to undo: the original image is already shown"));
    expect(store.masksOf(FILE).map((m) => m.id)).toEqual(["m01", "m03"]);

    history.publish(snapshotOf(historyState({ m01: RANGE, m03: LUMINANCE })));
    expect(store.masksOf(FILE).map((m) => [m.id, m.coverage])).toEqual([
      ["m01", null],
      ["m03", 0.5],
    ]);

    history.publish(snapshotOf(historyState({ m05: STARS }), OTHER));
    expect(store.masksOf(OTHER).map((m) => m.id)).toEqual(["m05"]);
    expect(store.masksOf(FILE).map((m) => m.id)).toEqual(["m01", "m03"]);

    history.publish(snapshotOf(null, FILE));
    history.publish({ ...EMPTY_HISTORY_SNAPSHOT });
    expect(store.masksOf(FILE).map((m) => m.id)).toEqual(["m01", "m03"]);

    history.publish(snapshotOf({ ...historyState({}), doc: null }));
    expect(store.masksOf(FILE)).toEqual([]);
  });

  it("mask_store_keeps_composite_masks_from_create_and_delete", () => {
    const store = new MaskStoreCore();
    store.recordCreated(COMPOSITE_RUN_KEY, created("m01", STARS, 0.02, 90));
    store.setActive(COMPOSITE_RUN_KEY, "m01");
    expect(store.masksOf(COMPOSITE_RUN_KEY).map((m) => m.id)).toEqual(["m01"]);
    expect(store.activeOf(COMPOSITE_RUN_KEY)).toBe("m01");
    expect(store.masksOf(FILE)).toEqual([]);
    store.recordDeleted(COMPOSITE_RUN_KEY, { mask_id: "m01", recipe: STARS, used_by: [], kept_for_replay: false });
    expect(store.masksOf(COMPOSITE_RUN_KEY)).toEqual([]);
    expect(store.activeOf(COMPOSITE_RUN_KEY)).toBeNull();
  });

  it("mask_store_keeps_each_exported_fits_path_once_without_notifying", () => {
    const store = new MaskStoreCore();
    const listener = vi.fn();
    store.subscribe(listener);
    const before = store.get();
    store.recordExported({ mask_id: "m01", kind: "float", fits_path: "C:/out/673nmos_m01_mask.fits", bitpix: -32, dims: [2048, 2048] });
    store.recordExported({ mask_id: "m02", kind: "label", fits_path: "C:/out/673nmos_m02_labels.fits", bitpix: 32, dims: [2048, 2048] });
    store.recordExported({ mask_id: "m01", kind: "float", fits_path: "C:/out/673nmos_m01_mask.fits", bitpix: -32, dims: [2048, 2048] });
    expect(store.exportedPaths()).toEqual(["C:/out/673nmos_m01_mask.fits", "C:/out/673nmos_m02_labels.fits"]);
    expect(store.get()).toBe(before);
    expect(listener).not.toHaveBeenCalled();
  });
});
