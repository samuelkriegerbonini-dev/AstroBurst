import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import type { MastFootprint, MastFootprintInput, MastFootprintSet } from "../../../shared/types/mast";
import type { WcsInfo } from "../../../shared/types/astrometry";
import type { RegionWire } from "../../../shared/types/regions";
import { OverlayStoreCore } from "../../../utils/overlayStore";
import { FOOTPRINT_LAYER_ID, isFootprintLayer } from "../../viewer/painters/footprintPainter";
import { MOVING_TEXT, NO_FOOTPRINT_TEXT, footprintInput, footprintKey } from "../../../utils/mastFootprints";
import {
  createFootprintsState,
  createFootprintsStore,
  footprintNoteFor,
  footprintsReducer,
  highlightOf,
  isShown,
  pixelFootprints,
  shownCount,
  type FootprintsState,
  type ShownFootprint,
} from "../state/footprintsStore";
import { createFootprintsController, hoveredDataset, type FootprintDeps } from "../state/footprintsController";
import { displayedImageRef, footprintImage } from "../state/displayedImage";
import { aroundTargetFor, wcsInfoFor } from "../state/useAroundTarget";
import { FootprintToggleView } from "../FootprintToggle";
import { NO_WORKSPACE_TEXT, sRegionNote, toggleDisabledReason } from "../logic/footprintView";
import { row } from "./fixtures";

const FIELD_PATH = "C:\\data\\jw02739001001_02105_00001_nrca1_cal.fits";
const FIELD = { path: FIELD_PATH, name: "jw02739001001_02105_00001_nrca1_cal.fits", projectionPath: FIELD_PATH };
const OTHER = { path: "C:\\data\\other_i2d.fits", name: "other_i2d.fits", projectionPath: "C:\\data\\other_i2d.fits" };
const SOLVED_PATH = "C:\\out\\jw02739001001_02105_00001_nrca1_cal_wcs.fits";
const SOLVED = { ...FIELD, projectionPath: SOLVED_PATH };

function projected(path: string, items: readonly MastFootprintInput[]): MastFootprintSet {
  return {
    path,
    width: 2048,
    height: 2048,
    footprints: items.map(
      (item, i): MastFootprint => ({
        datasetId: item.datasetId,
        polygons:
          item.targtype === "MOVING"
            ? []
            : [
                [
                  [i, 0],
                  [i + 10, 0],
                  [i + 10, 10],
                ],
              ],
        onImage: true,
        vertices: 3,
        cone: { ra: 274.7, dec: -13.8, radiusDeg: 0.01 },
        note: item.targtype === "MOVING" ? "moving target: the archive footprint is not sidereal" : null,
      }),
    ),
  };
}

function keyed(path: string, items: readonly MastFootprintInput[]): ShownFootprint[] {
  return projected(path, items).footprints.map((f, i) => ({ ...f, key: footprintKey(items[i]) }));
}

function rig(over: Partial<FootprintDeps> = {}) {
  const overlay = new OverlayStoreCore();
  const store = createFootprintsStore();
  const project = vi.fn(async (path: string, items: MastFootprintInput[]) => projected(path, items));
  const deps: FootprintDeps = {
    project,
    regText: vi.fn(async () => ({ regText: "# Region file format: DS9 version 4.1\nicrs\npolygon(1,2,3,4,5,6)\n", polygons: 1, notes: [] })),
    exportImage: vi.fn(async (_path: string, regions: RegionWire[]) => `image\n${regions.length}\n`),
    chooseFile: vi.fn(async (name: string) => `C:\\out\\${name}`),
    writeText: vi.fn(async () => {}),
    overlay,
    ...over,
  };
  const ctl = createFootprintsController(store, deps);
  return { overlay, store, deps, project, ctl };
}

function layerOn(overlay: OverlayStoreCore, path: string) {
  const layer = overlay.getDoc(path).layers.find((l) => l.id === FOOTPRINT_LAYER_ID);
  return layer && isFootprintLayer(layer) ? layer : null;
}

const A = row("jw02739-o001_t001_nircam");
const B = row("jw02739-o002_t001_miri", { sRegion: "POLYGON ICRS 274.72 -13.88 274.75 -13.88 274.75 -13.86" });
const C = row("jw02739-o003_t001_nircam");
const D = row("jw02739-o004_t001_nircam");
const COMET = row("jw01234-o001_t001_nircam", { values: { targtype: "MOVING" } });
const LWP_LARGE = row("LWP15521", { sRegion: "POLYGON ICRS 274.68 -13.80 274.69 -13.80 274.69 -13.79", values: { iue_aper: "LARGE" } });
const LWP_SMALL = row("LWP15521", { sRegion: "CIRCLE ICRS 274.685 -13.795 0.0008", values: { iue_aper: "SMALL" } });

describe("footprints reducer", () => {
  it("toggles datasets in pick order, keeps the workspace and clears projections that belong to another file", () => {
    let s = createFootprintsState();
    s = footprintsReducer(s, { type: "workspace", file: FIELD, wcs: "present" });
    expect(s.target).toEqual(FIELD);
    s = footprintsReducer(s, { type: "toggle", input: footprintInput(A) });
    s = footprintsReducer(s, { type: "toggle", input: footprintInput(B) });
    expect(s.items.map((i) => i.datasetId)).toEqual([A.datasetId, B.datasetId]);
    expect(isShown(s, footprintInput(A))).toBe(true);
    s = footprintsReducer(s, { type: "projected", image: FIELD, footprints: keyed(FIELD.path, s.items) });
    expect(shownCount(s)).toBe(2);
    s = footprintsReducer(s, { type: "toggle", input: footprintInput(A) });
    expect(s.items.map((i) => i.datasetId)).toEqual([B.datasetId]);
    expect(s.footprints.map((f) => f.datasetId)).toEqual([B.datasetId]);
    expect(footprintsReducer(s, { type: "workspace", file: FIELD, wcs: "present" })).toBe(s);
    const moved = footprintsReducer(s, { type: "workspace", file: OTHER, wcs: "absent" });
    expect(moved.target).toBeNull();
    expect(moved.workspace).toEqual(OTHER);
    expect(moved.footprints).toEqual([]);
    expect(moved.items).toEqual(s.items);
    expect(footprintsReducer(s, { type: "clear" }).items).toEqual([]);
  });

  it("keeps no target while the WCS of the workspace image is being read", () => {
    let s = createFootprintsState();
    s = footprintsReducer(s, { type: "workspace", file: FIELD, wcs: "checking" });
    expect(s.workspace).toEqual(FIELD);
    expect(s.target).toBeNull();
    expect(s.wcs).toBe("checking");
    s = footprintsReducer(s, { type: "workspace", file: FIELD, wcs: "present" });
    expect(s.target).toEqual(FIELD);
    expect(s.wcs).toBe("present");
  });

  it("drops the projection when the image on screen changes under the same file", () => {
    let s = footprintsReducer(createFootprintsState(), { type: "workspace", file: FIELD, wcs: "present" });
    s = footprintsReducer(s, { type: "toggle", input: footprintInput(A) });
    s = footprintsReducer(s, { type: "projected", image: FIELD, footprints: keyed(FIELD.path, s.items) });
    expect(shownCount(s)).toBe(1);
    s = footprintsReducer(s, { type: "workspace", file: SOLVED, wcs: "present" });
    expect(s.target).toEqual(SOLVED);
    expect(shownCount(s)).toBe(0);
  });

  it("tells apart rows that share a dataset id but not a footprint", () => {
    expect(footprintKey(footprintInput(LWP_LARGE))).not.toBe(footprintKey(footprintInput(LWP_SMALL)));
    expect(footprintKey(footprintInput(A))).toBe(footprintKey(footprintInput({ ...A, values: { other: 1 } })));
    let s = footprintsReducer(createFootprintsState(), { type: "workspace", file: FIELD, wcs: "present" });
    s = footprintsReducer(s, { type: "toggle", input: footprintInput(LWP_LARGE) });
    expect(isShown(s, footprintInput(LWP_LARGE))).toBe(true);
    expect(isShown(s, footprintInput(LWP_SMALL))).toBe(false);
    s = footprintsReducer(s, { type: "toggle", input: footprintInput(LWP_SMALL) });
    expect(s.items.map((i) => i.sRegion)).toEqual([LWP_LARGE.sRegion, LWP_SMALL.sRegion]);
    s = footprintsReducer(s, { type: "projected", image: FIELD, footprints: keyed(FIELD.path, s.items) });
    expect(shownCount(s)).toBe(2);
    s = footprintsReducer(s, { type: "toggle", input: footprintInput(LWP_SMALL) });
    expect(s.items.map((i) => i.sRegion)).toEqual([LWP_LARGE.sRegion]);
    expect(s.footprints.map((f) => f.key)).toEqual([footprintKey(footprintInput(LWP_LARGE))]);
  });

  it("highlights the hovered row over the selected one when the hovered row is drawn", () => {
    let s: FootprintsState = footprintsReducer(createFootprintsState(), { type: "workspace", file: FIELD, wcs: "present" });
    for (const r of [A, B]) s = footprintsReducer(s, { type: "toggle", input: footprintInput(r) });
    s = footprintsReducer(s, { type: "projected", image: FIELD, footprints: keyed(FIELD.path, s.items) });
    s = footprintsReducer(s, { type: "focus", datasetId: A.datasetId });
    expect(highlightOf(s)).toBe(A.datasetId);
    s = footprintsReducer(s, { type: "hover", datasetId: B.datasetId });
    expect(highlightOf(s)).toBe(B.datasetId);
    expect(footprintsReducer(s, { type: "hover", datasetId: B.datasetId })).toBe(s);
    s = footprintsReducer(s, { type: "hover", datasetId: C.datasetId });
    expect(highlightOf(s)).toBe(A.datasetId);
    s = footprintsReducer(s, { type: "hover", datasetId: null });
    expect(highlightOf(s)).toBe(A.datasetId);
  });
});

describe("footprints on the image on screen", () => {
  it("project through the displayed record of the selected file and fall back to the file", () => {
    const file = { path: FIELD.path, name: FIELD.name };
    expect(footprintImage(file, { filePath: FIELD.path, projectionPath: SOLVED_PATH })).toEqual(SOLVED);
    expect(footprintImage(file, { filePath: OTHER.path, projectionPath: OTHER.path })).toEqual(FIELD);
    expect(footprintImage(file, null)).toEqual(FIELD);
    expect(footprintImage(null, { filePath: FIELD.path, projectionPath: SOLVED_PATH })).toBeNull();
    expect(displayedImageRef(FIELD.path, SOLVED_PATH)).toEqual({ filePath: FIELD.path, projectionPath: SOLVED_PATH });
    expect(displayedImageRef(FIELD.path, null)).toEqual({ filePath: FIELD.path, projectionPath: FIELD.path });
    expect(displayedImageRef(null, SOLVED_PATH)).toBeNull();
  });

  it("count a readable WCS as present even when it gives no search cone, and retry a failed read", async () => {
    const info = { center_ra: 274.7, center_dec: -13.8, fov_arcmin: [Number.NaN, Number.NaN], field_of_view_w_arcmin: Number.NaN, field_of_view_h_arcmin: Number.NaN } as unknown as WcsInfo;
    const read = vi.fn(async () => info);
    expect(await wcsInfoFor("C:/scratch/nan_fov.fits", read)).toBe(info);
    expect(await aroundTargetFor("C:/scratch/nan_fov.fits", "nan_fov.fits", read)).toBeNull();
    expect(read).toHaveBeenCalledTimes(1);
    const failing = vi.fn(async (): Promise<WcsInfo> => {
      throw new Error("no celestial WCS");
    });
    expect(await wcsInfoFor("C:/scratch/plain.fits", failing)).toBeNull();
    await Promise.resolve();
    expect(await wcsInfoFor("C:/scratch/plain.fits", failing)).toBeNull();
    expect(failing).toHaveBeenCalledTimes(2);
  });
});

describe("footprints controller", () => {
  it("projects the shown datasets onto the workspace image and draws one overlay layer there", async () => {
    const { overlay, store, ctl, project } = rig();
    ctl.setWorkspace(FIELD, "present");
    await ctl.toggle(A);
    await ctl.toggle(B);
    await ctl.toggle(C);
    expect(project).toHaveBeenLastCalledWith(FIELD.path, [footprintInput(A), footprintInput(B), footprintInput(C)]);
    const layer = layerOn(overlay, FIELD.path);
    expect(layer?.footprints).toBe(3);
    expect(layer?.highlight).toBeNull();
    expect(store.getState().pending).toBe(false);

    ctl.focus(B.datasetId);
    expect(layerOn(overlay, FIELD.path)?.highlight).toBe(B.datasetId);
    ctl.hover(C.datasetId);
    expect(layerOn(overlay, FIELD.path)?.highlight).toBe(C.datasetId);
    ctl.hover(null);
    expect(layerOn(overlay, FIELD.path)?.highlight).toBe(B.datasetId);

    await ctl.toggle(B);
    expect(layerOn(overlay, FIELD.path)?.footprints).toBe(2);
    expect(layerOn(overlay, FIELD.path)?.highlight).toBeNull();
    ctl.clear();
    expect(layerOn(overlay, FIELD.path)).toBeNull();
    expect(store.getState().items).toEqual([]);
  });

  it("keeps the selected row highlighted while the pointer crosses a row that is not drawn", async () => {
    const { overlay, ctl } = rig();
    ctl.setWorkspace(FIELD, "present");
    await ctl.toggle(A);
    await ctl.toggle(B);
    await ctl.toggle(C);
    ctl.focus(B.datasetId);
    ctl.hover(D.datasetId);
    expect(layerOn(overlay, FIELD.path)?.highlight).toBe(B.datasetId);
    ctl.hover(C.datasetId);
    expect(layerOn(overlay, FIELD.path)?.highlight).toBe(C.datasetId);
  });

  it("projects through the image on screen, keeps the layer on the selected file and saves in its pixels", async () => {
    const { overlay, ctl, project, deps } = rig();
    ctl.setWorkspace(SOLVED, "present");
    await ctl.toggle(A);
    expect(project).toHaveBeenLastCalledWith(SOLVED_PATH, [footprintInput(A)]);
    expect(layerOn(overlay, FIELD.path)?.footprints).toBe(1);
    expect(layerOn(overlay, SOLVED_PATH)).toBeNull();
    await ctl.save(A);
    expect(deps.exportImage).toHaveBeenCalledWith(SOLVED_PATH, expect.any(Array));
    expect(deps.writeText).toHaveBeenLastCalledWith("C:\\out\\jw02739001001_02105_00001_nrca1_cal_mast_footprints.reg", "image\n1\n");

    await ctl.setWorkspace(FIELD, "present");
    expect(project).toHaveBeenLastCalledWith(FIELD.path, [footprintInput(A)]);
    expect(layerOn(overlay, FIELD.path)?.footprints).toBe(1);
    await ctl.setWorkspace({ ...FIELD, projectionPath: "C:\\out\\pv.fits" }, "absent");
    expect(layerOn(overlay, FIELD.path)).toBeNull();
  });

  it("draws both footprints of a dataset listed once per aperture", async () => {
    const { overlay, ctl, store } = rig();
    ctl.setWorkspace(FIELD, "present");
    await ctl.toggle(LWP_LARGE);
    await ctl.toggle(LWP_SMALL);
    expect(layerOn(overlay, FIELD.path)?.footprints).toBe(2);
    expect(isShown(store.getState(), footprintInput(LWP_SMALL))).toBe(true);
    await ctl.toggle(LWP_SMALL);
    expect(layerOn(overlay, FIELD.path)?.footprints).toBe(1);
    expect(isShown(store.getState(), footprintInput(LWP_LARGE))).toBe(true);
  });

  it("moves the layer when the workspace image changes and removes it when the image has no WCS", async () => {
    const { overlay, ctl, project } = rig();
    ctl.setWorkspace(FIELD, "present");
    await ctl.toggle(A);
    expect(layerOn(overlay, FIELD.path)?.footprints).toBe(1);
    await ctl.setWorkspace(OTHER, "present");
    expect(project).toHaveBeenLastCalledWith(OTHER.path, [footprintInput(A)]);
    expect(layerOn(overlay, FIELD.path)).toBeNull();
    expect(layerOn(overlay, OTHER.path)?.footprints).toBe(1);
    await ctl.setWorkspace(OTHER, "absent");
    expect(layerOn(overlay, OTHER.path)).toBeNull();
    await ctl.setWorkspace(null, "absent");
    expect(project).toHaveBeenCalledTimes(2);
  });

  it("sends long lists in chunks under the vertex cap and keeps the notes of moving targets", async () => {
    const { overlay, ctl, project, store } = rig();
    ctl.setWorkspace(FIELD, "present");
    const circles = Array.from({ length: 400 }, (_, i) => row(`c${i}`, { sRegion: "CIRCLE ICRS 274.7 -13.8 0.001" }));
    for (const r of circles) store.dispatch({ type: "toggle", input: footprintInput(r) });
    await ctl.toggle(COMET);
    expect(project.mock.calls.length).toBeGreaterThan(1);
    const sent = project.mock.calls.flatMap((c) => c[1].map((i) => i.datasetId));
    expect(sent).toEqual([...circles.map((r) => r.datasetId), COMET.datasetId]);
    expect(layerOn(overlay, FIELD.path)?.footprints).toBe(400);
    expect(footprintNoteFor(store.getState(), footprintInput(COMET))).toBe("moving target: the archive footprint is not sidereal");
  });

  it("keeps the layer off and names the error when the projection fails", async () => {
    const { overlay, ctl, store } = rig({
      project: vi.fn(async () => {
        throw { code: "mast_bad_input", message: "path: the image has no celestial WCS: no CTYPE", context: {} };
      }),
    });
    ctl.setWorkspace(FIELD, "present");
    await ctl.toggle(A);
    expect(layerOn(overlay, FIELD.path)).toBeNull();
    expect(store.getState().error?.message).toBe("path: the image has no celestial WCS: no CTYPE");
    expect(store.getState().pending).toBe(false);
  });

  it("drops a projection that a newer change overtook", async () => {
    let release: (() => void) | null = null;
    const slow = vi.fn(
      (path: string, items: MastFootprintInput[]) =>
        new Promise<MastFootprintSet>((resolve) => {
          release = () => resolve(projected(path, items));
        }),
    );
    const { overlay, ctl } = rig({ project: slow });
    ctl.setWorkspace(FIELD, "present");
    const first = ctl.toggle(A);
    const stale = release!;
    const second = ctl.toggle(B);
    const fresh = release!;
    fresh();
    await second;
    stale();
    await first;
    expect(layerOn(overlay, FIELD.path)?.footprints).toBe(2);
  });

  it("stops showing progress when the last dataset is removed while a projection runs", async () => {
    let release: (() => void) | null = null;
    const slow = vi.fn(
      (path: string, items: MastFootprintInput[]) =>
        new Promise<MastFootprintSet>((resolve) => {
          release = () => resolve(projected(path, items));
        }),
    );
    const { overlay, ctl, store } = rig({ project: slow });
    ctl.setWorkspace(FIELD, "present");
    const first = ctl.toggle(A);
    expect(store.getState().pending).toBe(true);
    await ctl.toggle(A);
    release!();
    await first;
    expect(store.getState().pending).toBe(false);
    expect(layerOn(overlay, FIELD.path)).toBeNull();

    const again = ctl.toggle(B);
    await ctl.setWorkspace(null, "absent");
    release!();
    await again;
    expect(store.getState().pending).toBe(false);
    expect(layerOn(overlay, FIELD.path)).toBeNull();
  });

  it("saves the projected footprints in image pixels, or the selected one in ICRS without an image", async () => {
    const { ctl, deps } = rig();
    ctl.setWorkspace(FIELD, "present");
    await ctl.toggle(A);
    await ctl.toggle(B);
    expect(await ctl.save(A)).toBe("C:\\out\\jw02739001001_02105_00001_nrca1_cal_mast_footprints.reg");
    expect(deps.exportImage).toHaveBeenCalledWith(FIELD.path, expect.any(Array));
    expect(vi.mocked(deps.exportImage).mock.calls[0][1]).toHaveLength(2);
    expect(deps.writeText).toHaveBeenLastCalledWith("C:\\out\\jw02739001001_02105_00001_nrca1_cal_mast_footprints.reg", "image\n2\n");

    const bare = rig();
    expect(await bare.ctl.save(B)).toBe("C:\\out\\jw02739-o002_t001_miri_footprint.reg");
    expect(bare.deps.regText).toHaveBeenCalledWith([footprintInput(B)]);
    expect(bare.deps.exportImage).not.toHaveBeenCalled();
    expect(vi.mocked(bare.deps.writeText).mock.calls[0][1]).toContain("polygon(1,2,3,4,5,6)");

    const cancelled = rig({ chooseFile: vi.fn(async () => null) });
    expect(await cancelled.ctl.save(B)).toBeNull();
    expect(cancelled.deps.writeText).not.toHaveBeenCalled();

    const empty = rig({ regText: vi.fn(async () => ({ regText: "icrs\n", polygons: 0, notes: [{ datasetId: COMET.datasetId, note: "moving" }] })) });
    expect(await empty.ctl.save(COMET)).toBeNull();
    expect(empty.deps.writeText).not.toHaveBeenCalled();
    expect(empty.store.getState().saveError).toBe("moving");
  });

  it("leaves footprints that miss the image out of the image-pixel .reg, and saves the selected one in ICRS when none overlaps", async () => {
    const far: [number, number][] = [
      [-6380996515240199, -5368276097759968],
      [1e20, 3],
      [4, -1e20],
    ];
    const offFar = (path: string, items: MastFootprintInput[]): MastFootprintSet => {
      const set = projected(path, items);
      return { ...set, footprints: set.footprints.map((f) => (f.datasetId === C.datasetId ? { ...f, polygons: [far], onImage: false } : f)) };
    };
    const mixed = rig({ project: vi.fn(async (path: string, items: MastFootprintInput[]) => offFar(path, items)) });
    mixed.ctl.setWorkspace(FIELD, "present");
    await mixed.ctl.toggle(A);
    await mixed.ctl.toggle(C);
    expect(shownCount(mixed.store.getState())).toBe(2);
    expect(await mixed.ctl.save(A)).toBe("C:\\out\\jw02739001001_02105_00001_nrca1_cal_mast_footprints.reg");
    const regions = vi.mocked(mixed.deps.exportImage).mock.calls[0][1];
    expect(regions.map((r) => r.props.text)).toEqual([A.datasetId]);

    const allOff = rig({ project: vi.fn(async (path: string, items: MastFootprintInput[]) => offFar(path, items)) });
    allOff.ctl.setWorkspace(FIELD, "present");
    await allOff.ctl.toggle(C);
    expect(await allOff.ctl.save(B)).toBe("C:\\out\\jw02739-o002_t001_miri_footprint.reg");
    expect(allOff.deps.exportImage).not.toHaveBeenCalled();
    expect(allOff.deps.regText).toHaveBeenCalledWith([footprintInput(B)]);
    expect(pixelFootprints(allOff.store.getState())).toEqual([]);
  });

  it("forgets a save error when another row is selected", async () => {
    const { ctl, store } = rig({
      regText: vi.fn(async () => {
        throw { code: "mast_bad_input", message: "items: s_region: GALACTIC frames are not supported", context: {} };
      }),
    });
    ctl.focus(B.datasetId);
    expect(await ctl.save(B)).toBeNull();
    expect(store.getState().saveError).toBe("items: s_region: GALACTIC frames are not supported");
    ctl.focus(B.datasetId);
    expect(store.getState().saveError).toBe("items: s_region: GALACTIC frames are not supported");
    ctl.focus(A.datasetId);
    expect(store.getState().saveError).toBeNull();
  });

  it("reads the dataset under the pointer from result cards and table rows only", () => {
    const at = (selector: string | null, id: string | null) => ({
      closest: (s: string) => (selector && s.includes(selector) ? { getAttribute: (name: string) => (name === "data-dataset" ? id : null) } : null),
    });
    expect(hoveredDataset(at('[data-testid="mast-card"]', "a") as unknown as EventTarget)).toBe("a");
    expect(hoveredDataset(at('[data-testid="mast-row"]', "b") as unknown as EventTarget)).toBe("b");
    expect(hoveredDataset(at(null, null) as unknown as EventTarget)).toBeNull();
    expect(hoveredDataset(null)).toBeNull();
  });
});

describe("footprint toggle", () => {
  const base = {
    datasetId: A.datasetId,
    blocked: null,
    workspaceName: FIELD.name,
    fileName: FIELD.name,
    checking: false,
    shown: false,
    count: 0,
    pending: false,
    note: null,
    error: null,
    canSave: true,
    savesPixels: false,
    onToggle: vi.fn(),
    onClear: vi.fn(),
    onSave: vi.fn(),
  };
  const ready = { blocked: null, workspaceName: FIELD.name, fileName: FIELD.name, checking: false };

  it("is enabled only for a row with a footprint and a workspace image with a WCS", () => {
    expect(toggleDisabledReason(ready)).toBeNull();
    expect(toggleDisabledReason({ ...ready, blocked: MOVING_TEXT })).toBe(MOVING_TEXT);
    expect(toggleDisabledReason({ blocked: NO_FOOTPRINT_TEXT, workspaceName: null, fileName: null, checking: false })).toBe(NO_FOOTPRINT_TEXT);
    expect(toggleDisabledReason({ blocked: null, workspaceName: null, fileName: null, checking: false })).toBe("Open an image in the workspace to draw footprints on it");
    expect(toggleDisabledReason({ blocked: null, workspaceName: OTHER.name, fileName: null, checking: false })).toBe("other_i2d.fits has no celestial WCS");
  });

  it("says the WCS is being read instead of missing while the check runs", () => {
    expect(toggleDisabledReason({ blocked: null, workspaceName: OTHER.name, fileName: null, checking: true })).toBe("Reading the WCS of other_i2d.fits…");
  });

  it("names the workspace file, shows the pressed state and the count", () => {
    const off = renderToStaticMarkup(createElement(FootprintToggleView, base));
    expect(off).toContain('data-testid="mast-footprint-toggle"');
    expect(off).toContain('aria-pressed="false"');
    expect(off).toContain(`Show on ${FIELD.name}`);
    expect(off).not.toContain("disabled");
    expect(off).not.toContain("mast-footprint-count");
    expect(off).not.toContain("mast-footprint-reason");
    expect(off).not.toContain("aria-describedby");

    const on = renderToStaticMarkup(createElement(FootprintToggleView, { ...base, shown: true, count: 3, savesPixels: true }));
    expect(on).toContain('aria-pressed="true"');
    expect(on).toContain(`3 footprints on ${FIELD.name}`);
    expect(on).toContain("Clear");
    expect(on).toContain(`Save the footprints drawn on ${FIELD.name} as a DS9 region file in image pixels`);

    const offImage = renderToStaticMarkup(createElement(FootprintToggleView, { ...base, shown: true, count: 1, savesPixels: false }));
    expect(offImage).toContain(`1 footprint on ${FIELD.name}`);
    expect(offImage).toContain("Save this footprint as a DS9 region file in ICRS");

    const failed = renderToStaticMarkup(createElement(FootprintToggleView, { ...base, shown: true, error: "path: the image has no celestial WCS", note: "1 of 2 polygons do not project onto this image" }));
    expect(failed).toContain('role="alert"');
    expect(failed).toContain("path: the image has no celestial WCS");
    expect(failed).toContain("1 of 2 polygons do not project onto this image");
  });

  it("shows why it is disabled as text the button points to", () => {
    const blocked = renderToStaticMarkup(createElement(FootprintToggleView, { ...base, blocked: MOVING_TEXT, workspaceName: null, fileName: null, canSave: false }));
    expect(blocked).toContain("Show on image");
    expect(blocked).toMatch(/data-testid="mast-footprint-toggle"[^>]*disabled=""/);
    const describedBy = blocked.match(/data-testid="mast-footprint-toggle"[^>]*aria-describedby="([^"]+)"/)?.[1];
    expect(describedBy).toBeTruthy();
    const id = (describedBy ?? "").replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    expect(blocked).toMatch(new RegExp(`<span[^>]*id="${id}"[^>]*data-testid="mast-footprint-reason"[^>]*>${MOVING_TEXT}</span>`));

    const checking = renderToStaticMarkup(createElement(FootprintToggleView, { ...base, workspaceName: OTHER.name, fileName: null, checking: true }));
    expect(checking).toMatch(/data-testid="mast-footprint-reason"[^>]*>Reading the WCS of other_i2d\.fits…</);
  });
});

describe("s_region note in Metadata", () => {
  it("gives the projected summary, else the reason the footprint cannot be drawn", () => {
    const ready = { blocked: null, workspaceName: FIELD.name, fileName: FIELD.name, checking: false };
    expect(sRegionNote("1 polygon, 3 vertices", { ...ready, blocked: MOVING_TEXT })).toBe("1 polygon, 3 vertices");
    expect(sRegionNote(null, { ...ready, blocked: MOVING_TEXT })).toBe(MOVING_TEXT);
    expect(sRegionNote(null, { blocked: null, workspaceName: null, fileName: null, checking: false })).toBe(NO_WORKSPACE_TEXT);
    expect(sRegionNote(null, ready)).toBeNull();
  });
});
