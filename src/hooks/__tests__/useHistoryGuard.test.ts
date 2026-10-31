import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HeadView, HistoryDoc, HistoryNode, HistoryState, OpSpec } from "../../shared/types/ops";

const SOURCE = "C:/Users/samue/Documents/m16/673nmos.fits";
const DIR = "C:/Users/samue/AppData/Roaming/com.astroburst.desktop/history/8b40961e4db355f7";
const FILE_PNG = "asset://localhost/673nmos.png";
const COMPOSITE_PNG = "asset://localhost/rgb_composite_blend.png";
const KEY = `f673|${SOURCE}`;
const SCOPE = { imageKey: SOURCE, basePath: null };

interface ScreenFile {
  id: string;
  path: string;
  name: string;
  result: { is_rgb: boolean; previewUrl: string };
}

const screen = vi.hoisted(() => ({ file: null as ScreenFile | null, composite: null as string | null }));

const { ops, announceMock, actions } = vi.hoisted(() => ({
  ops: {
    getHistory: vi.fn(),
    setHead: vi.fn(),
    undo: vi.fn(),
    redo: vi.fn(),
    rehydrate: vi.fn(),
    deleteBranch: vi.fn(),
    getKernel: vi.fn(),
  },
  announceMock: vi.fn(),
  actions: {
    currentFileKey: vi.fn(),
    publishProcessed: vi.fn(),
    setChain: vi.fn(),
    resetProcessed: vi.fn(),
    setHistoryId: vi.fn(),
  },
}));

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../services/ops", () => ops);
vi.mock("../../components/dock/useDockDrag", () => ({ announce: announceMock }));
vi.mock("../../infrastructure/tauri", () => ({ getPreviewUrl: async (path: string) => `asset://${path}` }));
vi.mock("../useDockLayout", () => ({ dockStore: { dispatch: vi.fn() } }));
vi.mock("../../context/PreviewContext", () => ({
  fileKeyOf: (file: ScreenFile | null) => (file ? `${file.id}|${file.path}` : null),
  getRenderRecord: () => null,
  useFileContext: () => ({ file: screen.file }),
  useRenderContext: () => ({ chain: { steps: {}, psfKernel: null, base: null } }),
  useRenderActions: () => actions,
}));
vi.mock("../../context/CompositeContext", () => ({
  liveCompositeVersion: () => 0,
  useCompositePreview: () => ({ compositePreviewUrl: screen.composite }),
}));

import { hooks } from "../../test/hookRuntime";
import { useHistory, type HistoryController } from "../useHistory";
import { dockStore } from "../useDockLayout";
import { historyMoveBlock } from "../../utils/historyKeys";
import { historyRerunStore } from "../../utils/historyRerunStore";
import { EMPTY_HISTORY_SNAPSHOT, historyStore } from "../../utils/historyStore";

const BACKGROUND: OpSpec = {
  op: "background",
  v: 1,
  model: "polynomial",
  config: { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" },
};

const NODE: HistoryNode = {
  id: "n0001",
  parent: null,
  op: BACKGROUND,
  mask: null,
  output: { path: "n0001.fits", png: "n0001.png", extras: [], dims: [1600, 1600], planes: 1, domain: "linear" },
  at: "2026-10-25T01:24:00Z",
  elapsed_ms: 900,
  app: "0.7.6",
  threads: 24,
  present: true,
  bytes: 10_240_000,
  checkpoint: false,
};

function doc(head: string | null, nodes: HistoryNode[]): HistoryDoc {
  return {
    format: "astroburst-history",
    version: 1,
    image: { id: "8b40961e4db355f7", ref: SOURCE, size: 1, mtime: null },
    image_key: SOURCE,
    base_path: null,
    head,
    policy: { k: 4 },
    app: "0.7.6",
    threads: 24,
    masks: {},
    nodes,
  };
}

function state(head: string | null, nodes: HistoryNode[] = [NODE]): HistoryState {
  return { image_id: "8b40961e4db355f7", history_dir: DIR, root_path: SOURCE, doc: doc(head, nodes), read_only: null, fingerprint_matches: true, busy: false };
}

function view(head: string | null): HeadView {
  return {
    image_id: "8b40961e4db355f7",
    history_dir: DIR,
    head,
    node: head
      ? { id: NODE.id, parent: null, op: BACKGROUND, fits_path: `${DIR}/n0001.fits`, png_path: `${DIR}/n0001.png`, extras: [], dimensions: [1600, 1600], planes: 1, domain: "linear", present: true }
      : null,
    root_path: SOURCE,
    can_undo: head !== null,
    can_redo: head === null,
    node_count: 1,
  };
}

const MONO_FILE: ScreenFile = { id: "f673", path: SOURCE, name: "673nmos.fits", result: { is_rgb: false, previewUrl: FILE_PNG } };
const RGB_FILE: ScreenFile = { ...MONO_FILE, result: { is_rgb: true, previewUrl: FILE_PNG } };

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const renderHistory = () => hooks.render(useHistory as unknown as (p: object) => unknown, {}) as HistoryController;

async function showFile(file: ScreenFile, composite: string | null): Promise<HistoryController> {
  Object.assign(screen, { file, composite });
  hooks.mount();
  ops.getHistory.mockResolvedValue(state("n0001"));
  const controller = renderHistory();
  await flush();
  for (const fn of [...Object.values(ops), ...Object.values(actions), announceMock]) fn.mockClear();
  return controller;
}

beforeEach(() => {
  for (const fn of [...Object.values(ops), ...Object.values(actions), announceMock]) fn.mockReset();
  vi.mocked(dockStore.dispatch).mockClear();
  actions.currentFileKey.mockReturnValue(KEY);
  historyStore.unbind();
  historyStore.set(EMPTY_HISTORY_SNAPSHOT);
  historyRerunStore.clear();
});

describe("history moves with a colour composite on screen (HI-02)", () => {
  it("the palette's Undo and Redo and the History tool's Undo, Redo and Go to leave the hidden file's head where it is", async () => {
    const controller = await showFile(MONO_FILE, COMPOSITE_PNG);
    const reason = historyMoveBlock({ compositeMode: true, fileName: "673nmos.fits" });

    historyStore.undo();
    historyStore.redo();
    await controller.undo();
    await controller.redo();
    await controller.goTo(null);
    await controller.goTo("n0001");
    await flush();

    expect(reason).not.toBeNull();
    expect(controller.blockedReason).toBe(reason);
    expect(ops.undo).not.toHaveBeenCalled();
    expect(ops.redo).not.toHaveBeenCalled();
    expect(ops.setHead).not.toHaveBeenCalled();
    expect(actions.publishProcessed).not.toHaveBeenCalled();
    expect(actions.resetProcessed).not.toHaveBeenCalled();
    expect(announceMock.mock.calls).toEqual(Array.from({ length: 6 }, () => [reason]));
    expect(historyStore.get().state?.doc?.head).toBe("n0001");
  });

  it("an Undo bound while the file was on screen is refused once the composite is shown", async () => {
    const before = await showFile(MONO_FILE, null);
    screen.composite = COMPOSITE_PNG;
    renderHistory();

    await before.undo();
    historyStore.undo();
    await flush();

    expect(ops.undo).not.toHaveBeenCalled();
    expect(announceMock).toHaveBeenCalledTimes(2);
  });

  it("Re-run with and Re-run downstream are refused like Go to: the head stays, nothing is armed and Processing does not open", async () => {
    const controller = await showFile(MONO_FILE, COMPOSITE_PNG);
    const reason = historyMoveBlock({ compositeMode: true, fileName: "673nmos.fits" });
    ops.setHead.mockResolvedValue(view("n0001"));

    await controller.rerunWith(NODE, false);
    await controller.rerunWith(NODE, true);
    await flush();

    expect(ops.setHead).not.toHaveBeenCalled();
    expect(historyRerunStore.get()).toBeNull();
    expect(historyRerunStore.armed()).toBeNull();
    expect(dockStore.dispatch).not.toHaveBeenCalled();
    expect(announceMock.mock.calls).toEqual([[reason], [reason]]);
  });

  it("with the file on screen, Re-run with still moves the head, asks for the re-run and opens Processing", async () => {
    const controller = await showFile(MONO_FILE, null);
    ops.setHead.mockResolvedValue(view("n0001"));
    ops.getHistory.mockResolvedValue(state("n0001"));

    await controller.rerunWith(NODE, false);
    await flush();

    expect(ops.setHead).toHaveBeenCalledWith(SCOPE, "n0001");
    expect(historyRerunStore.get()).toMatchObject({ section: "background", node: "n0001", downstream: false });
    expect(dockStore.dispatch).toHaveBeenCalledWith({ type: "open", tool: "processing" });
  });

  it("Delete branch stays an explicit action on the named file", async () => {
    const controller = await showFile(MONO_FILE, COMPOSITE_PNG);
    ops.deleteBranch.mockResolvedValue({ ...view(null), removed: ["n0001"] });
    ops.getHistory.mockResolvedValue(state(null, []));

    await controller.deleteBranch("n0001");

    expect(ops.deleteBranch).toHaveBeenCalledWith(SCOPE, "n0001");
  });

  it("with the file on screen, the palette's Undo still moves the head and nothing is refused", async () => {
    const controller = await showFile(MONO_FILE, null);
    ops.undo.mockResolvedValue(view(null));
    ops.getHistory.mockResolvedValue(state(null));

    historyStore.undo();
    await flush();

    expect(controller.blockedReason).toBeNull();
    expect(ops.undo).toHaveBeenCalledWith(SCOPE);
    expect(actions.resetProcessed).toHaveBeenCalledTimes(1);
    expect(announceMock.mock.calls).toEqual([["Undo: Background Extraction"]]);
  });

  it("an RGB file shown through the composite slot keeps its Undo and Go to", async () => {
    const controller = await showFile(RGB_FILE, FILE_PNG);
    ops.setHead.mockResolvedValue(view(null));
    ops.getHistory.mockResolvedValue(state(null));

    await controller.goTo(null);

    expect(controller.blockedReason).toBeNull();
    expect(ops.setHead).toHaveBeenCalledWith(SCOPE, null);
  });
});
