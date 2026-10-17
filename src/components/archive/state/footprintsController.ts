import type { MastFootprintInput, MastFootprintRegions, MastFootprintSet, MastRow } from "../../../shared/types/mast";
import type { RegionWire } from "../../../shared/types/regions";
import { mastErrorOf, mastFootprints, mastFootprintsReg } from "../../../services/mast";
import { exportRegions } from "../../../services/regions";
import { overlayStore, type OverlayStoreCore } from "../../../utils/overlayStore";
import { footprintChunks, footprintInput, footprintKey, footprintRegions } from "../../../utils/mastFootprints";
import { FOOTPRINT_LAYER_ID, footprintLayer } from "../../viewer/painters/footprintPainter";
import { imageRegionFileName, skyRegionFileName } from "../logic/footprintView";
import { drawnFootprints, footprintsStore, highlightOf, pixelFootprints, type FootprintsStore, type ShownFootprint, type WcsCheck, type WorkspaceImage } from "./footprintsStore";

export interface FootprintDeps {
  project: (path: string, items: MastFootprintInput[]) => Promise<MastFootprintSet>;
  regText: (items: MastFootprintInput[]) => Promise<MastFootprintRegions>;
  exportImage: (path: string, regions: RegionWire[]) => Promise<string>;
  chooseFile: (defaultName: string) => Promise<string | null>;
  writeText: (path: string, text: string) => Promise<void>;
  overlay: Pick<OverlayStoreCore, "add" | "remove">;
}

type PeekPolygons = [number, number][][] | null;

export interface FootprintsController {
  setWorkspace(file: WorkspaceImage | null, wcs: WcsCheck): Promise<void>;
  toggle(row: MastRow, polygons?: PeekPolygons): Promise<void>;
  clear(): void;
  hover(datasetId: string | null): void;
  focus(datasetId: string | null): void;
  save(row: MastRow | null, polygons?: PeekPolygons): Promise<string | null>;
}

const RESULT_ITEM_SELECTOR = '[data-testid="mast-card"], [data-testid="mast-row"]';
const NOTHING_TO_SAVE = "This dataset has no footprint to save";

export function hoveredDataset(target: EventTarget | null): string | null {
  const element = target as { closest?: (selector: string) => { getAttribute(name: string): string | null } | null } | null;
  if (!element || typeof element.closest !== "function") return null;
  return element.closest(RESULT_ITEM_SELECTOR)?.getAttribute("data-dataset") ?? null;
}

export function createFootprintsController(store: FootprintsStore, deps: FootprintDeps): FootprintsController {
  let seq = 0;
  let drawnOn: string | null = null;
  let painted: { footprints: unknown; highlight: string | null } | null = null;

  const sync = () => {
    const state = store.getState();
    const footprints = drawnFootprints(state);
    const path = state.target !== null && state.items.length > 0 && footprints.length > 0 ? state.target.path : null;
    if (drawnOn !== null && drawnOn !== path) {
      deps.overlay.remove(drawnOn, FOOTPRINT_LAYER_ID);
      painted = null;
    }
    drawnOn = path;
    if (path === null) return;
    const highlight = highlightOf(state);
    if (painted && painted.footprints === footprints && painted.highlight === highlight) return;
    painted = { footprints, highlight };
    deps.overlay.add(path, footprintLayer(footprints, highlight));
  };

  const project = async () => {
    const run = ++seq;
    const { target, items } = store.getState();
    if (target === null || items.length === 0) {
      store.dispatch({ type: "idle" });
      sync();
      return;
    }
    store.dispatch({ type: "projecting" });
    try {
      const sets: MastFootprintSet[] = [];
      for (const chunk of footprintChunks(items)) {
        sets.push(await deps.project(target.projectionPath, chunk));
        if (run !== seq) return;
      }
      const footprints = sets.flatMap((set) => set.footprints);
      const shown = items.slice(0, footprints.length).map((item, i): ShownFootprint => ({ ...footprints[i], key: footprintKey(item) }));
      store.dispatch({ type: "projected", image: target, footprints: shown });
    } catch (err) {
      if (run !== seq) return;
      store.dispatch({ type: "failed", error: mastErrorOf(err) });
    }
    sync();
  };

  const skyText = async (row: MastRow, polygons: PeekPolygons): Promise<string | null> => {
    const sky = await deps.regText([footprintInput(row, polygons)]);
    if (sky.polygons > 0) return sky.regText;
    store.dispatch({ type: "saveFailed", error: sky.notes[0]?.note ?? NOTHING_TO_SAVE });
    return null;
  };

  return {
    setWorkspace(file, wcs) {
      const before = store.getState();
      store.dispatch({ type: "workspace", file, wcs });
      const after = store.getState();
      if (after === before) return Promise.resolve();
      return project();
    },
    toggle(row, polygons = null) {
      store.dispatch({ type: "toggle", input: footprintInput(row, polygons) });
      return project();
    },
    clear() {
      seq++;
      store.dispatch({ type: "clear" });
      sync();
    },
    hover(datasetId) {
      store.dispatch({ type: "hover", datasetId });
      sync();
    },
    focus(datasetId) {
      store.dispatch({ type: "focus", datasetId });
      sync();
    },
    async save(row, polygons = null) {
      store.dispatch({ type: "saveFailed", error: null });
      const state = store.getState();
      const drawn = pixelFootprints(state);
      const image = drawn.length > 0 ? state.target : null;
      try {
        let text: string | null;
        let name: string;
        if (image !== null) {
          text = await deps.exportImage(image.projectionPath, footprintRegions(drawn));
          name = imageRegionFileName(image.name);
        } else if (row !== null) {
          text = await skyText(row, polygons);
          name = skyRegionFileName(row.datasetId);
        } else {
          return null;
        }
        if (text === null) return null;
        const file = await deps.chooseFile(name);
        if (!file) return null;
        await deps.writeText(file, text);
        return file;
      } catch (err) {
        store.dispatch({ type: "saveFailed", error: err instanceof Error ? err.message : mastErrorOf(err).message });
        return null;
      }
    },
  };
}

const LIVE_DEPS: FootprintDeps = {
  project: (path, items) => mastFootprints(path, items),
  regText: (items) => mastFootprintsReg(items),
  exportImage: async (path, regions) => (await exportRegions(path, regions, "image", true)).reg_text,
  chooseFile: async (defaultName) => {
    const { save } = await import("@tauri-apps/plugin-dialog");
    return save({ defaultPath: defaultName, filters: [{ name: "DS9 regions", extensions: ["reg"] }], title: "Save MAST footprints" });
  },
  writeText: async (path, text) => {
    const { writeTextFile } = await import("@tauri-apps/plugin-fs");
    await writeTextFile(path, text);
  },
  overlay: overlayStore,
};

export const footprintsController = createFootprintsController(footprintsStore, LIVE_DEPS);
