import { afterEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/masks", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/masks")>()),
  createMask: vi.fn(),
  previewMask: vi.fn(),
  deleteMask: vi.fn(),
  exportMask: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri")>()),
  getOutputDir: async () => "C:/out",
}));

import { open } from "@tauri-apps/plugin-dialog";
import { hooks } from "../../../test/hookRuntime";
import { MasksTool, type MasksToolProps } from "../MasksPanel";
import { RunButton, Slider, Toggle, ErrorAlert } from "../../ui";
import { COMPOSITE_MASK_TARGET, createMask, deleteMask, exportMask, previewMask } from "../../../services/masks";
import { MASK_MARK_DEFAULT, MASK_OPACITY_DEFAULT, MASK_TINT_DEFAULT, maskStore } from "../../../utils/maskStore";
import { regionStore } from "../../../utils/regionStore";
import { overlayStore } from "../../../utils/overlayStore";
import { outputKeepList } from "../../../utils/outputKeep";
import { COMPOSITE_RUN_KEY } from "../../../utils/compositeChain";
import { EMPTY_HISTORY_SNAPSHOT, historyStore, type HistorySnapshot } from "../../../utils/historyStore";
import type { HistoryTarget } from "../../../hooks/useHistory";
import type { HistoryDoc, HistoryNode, HistoryState, OpSpec } from "../../../shared/types/ops";
import type { MaskCreateOptions, MaskCreateResult, MaskSpec } from "../../../shared/types/masks";
import type { Region, RegionShape } from "../../../shared/types/regions";

type AnyProps = Record<string, unknown> & { children?: ReactNode };

const HISTORY_DIR = "C:/app/history/5e1f0c2a9b7d3e61";
const ARCSINH = { op: "arcsinh", factor: 50 } as unknown as OpSpec;
const CURVES = { op: "tone" } as unknown as OpSpec;
const CIRCLE: RegionShape = { shape: "circle", x: 20, y: 18, r: 6 };
const BOX: RegionShape = { shape: "box", x: 22, y: 18, width: 4, height: 4, angle: 0 };
const POINT: RegionShape = { shape: "point", x: 40, y: 30 };
const STAR_DEFAULTS = {
  growth_factor: 2.5,
  softness: 4,
  min_fwhm: 1.5,
  max_fwhm: 30,
  max_eccentricity: 0.85,
  luminance_protect: false,
  luminance_ceiling: 0.85,
};
const COVERAGE: Record<string, number> = { luminance: 1, range: 0.125, stars: 0.031, regions: 0.0625, file: 0.25 };

const RANGE_SPEC: MaskSpec = { kind: "range", lo: 0.2, hi: 0.8, fuzz: 0.05, feather_px: 3, invert: true };
const LUMA_SPEC: MaskSpec = { kind: "luminance", auto_stf: true, feather_px: 0, invert: false };
const REGIONS_SPEC: MaskSpec = { kind: "regions", shapes: [CIRCLE], exclude_shapes: [BOX], feather_px: 0, invert: false };

function region(id: string, shape: RegionShape, include: boolean): Region {
  return { id, shape, props: { color: null, width: null, text: null, dash: null, include }, backgroundId: null };
}

function node(id: string, op: OpSpec, mask: string | null = null): HistoryNode {
  return {
    id,
    parent: null,
    op,
    mask,
    output: { path: `${id}.fits`, png: `${id}.png`, extras: [], dims: [64, 48], planes: 1, domain: "linear" },
    at: "2026-10-31T10:00:00Z",
    elapsed_ms: 10,
    app: "0.8.0",
    threads: 8,
    present: true,
    bytes: 12288,
    checkpoint: false,
  } as unknown as HistoryNode;
}

function historyState(root: string, head: string | null, nodes: HistoryNode[], masks: Record<string, unknown> = {}): HistoryState {
  return {
    image_id: "5e1f0c2a9b7d3e61",
    history_dir: HISTORY_DIR,
    root_path: root,
    doc: { ...({} as HistoryDoc), head, nodes, masks },
    read_only: null,
    fingerprint_matches: true,
    busy: false,
  };
}

function fileTarget(path: string): HistoryTarget {
  return { fileKey: `7|${path}`, scope: { imageKey: path, basePath: null }, base: null, rootPreviewUrl: null };
}

function snapshotFor(target: HistoryTarget, state: HistoryState | null, over: Partial<HistorySnapshot> = {}): HistorySnapshot {
  return { fileKey: target.fileKey, scope: target.scope, state, error: null, pending: false, ...over };
}

function created(maskId: string, recipe: MaskSpec, coverage: number, stars: number | null = null): MaskCreateResult {
  return { mask_id: maskId, kind: recipe.kind, coverage, dims: [64, 48], stars, recipe };
}

function backendCreates(): void {
  let issued = 0;
  vi.mocked(createMask).mockImplementation(async (_target, recipe, options: MaskCreateOptions = {}) => {
    issued += 1;
    const echo = { ...recipe, feather_px: options.mods?.feather_px ?? 0, invert: options.mods?.invert ?? false } as MaskSpec;
    return created(`m0${issued}`, echo, COVERAGE[recipe.kind], recipe.kind === "stars" ? 812 : null);
  });
}

function elements(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...elements(node.props.children)];
}

function find(tree: ReactNode, match: (el: ReactElement<AnyProps>) => boolean, what: string): ReactElement<AnyProps> {
  const found = elements(tree).find(match);
  if (!found) throw new Error(`no ${what} in the Masks tool`);
  return found;
}

const labelled = (label: string) => (el: ReactElement<AnyProps>) => el.props["aria-label"] === label;
const kindButton = (kind: string) => (el: ReactElement<AnyProps>) => el.props["data-mask-kind"] === kind;
const control = (type: unknown, label: string) => (el: ReactElement<AnyProps>) => el.type === type && el.props.label === label;

interface Harness {
  props: MasksToolProps;
  view: () => ReactNode;
  html: () => string;
  click: (match: (el: ReactElement<AnyProps>) => boolean, what: string) => void;
  type: (label: string, text: string) => void;
  slide: (label: string, value: number) => void;
  toggle: (label: string, value: boolean) => void;
  create: () => Promise<void>;
}

function harness(over: Partial<MasksToolProps>): Harness {
  const props: MasksToolProps = {
    fileName: "m16.fits",
    compositeMode: false,
    target: null,
    snapshot: EMPTY_HISTORY_SNAPSHOT,
    imageDims: [64, 48],
    compositeRevision: "",
    runLocked: false,
    onHistoryChanged: vi.fn(async () => {}),
    ...over,
  };
  hooks.mount();
  const view = () => hooks.render(MasksTool, props) as ReactNode;
  const h: Harness = {
    props,
    view,
    html: () => renderToStaticMarkup(view() as ReactElement),
    click: (match, what) => (find(view(), match, what).props.onClick as () => void)(),
    type: (label, text) => (find(view(), labelled(label), `${label} field`).props.onChange as (t: string) => void)(text),
    slide: (label, value) => (find(view(), control(Slider, label), `${label} slider`).props.onChange as (v: number) => void)(value),
    toggle: (label, value) => (find(view(), control(Toggle, label), `${label} toggle`).props.onChange as (v: boolean) => void)(value),
    create: async () => {
      const before = vi.mocked(createMask).mock.results.length;
      const button = find(view(), (el) => el.type === RunButton && el.props.label === "Create mask", "Create mask button");
      expect(button.props.disabled, "Create mask is disabled").toBe(false);
      (button.props.onClick as () => void)();
      await vi.waitFor(() => expect(vi.mocked(createMask).mock.results.length).toBe(before + 1));
      await settle();
    },
  };
  return h;
}

function settle(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function rowIds(tree: ReactNode): string[] {
  return elements(tree)
    .map((el) => el.props["data-mask-row"])
    .filter((id): id is string => typeof id === "string");
}

function row(tree: ReactNode, id: string): ReactElement<AnyProps> {
  return find(tree, (el) => el.props["data-mask-row"] === id, `row ${id}`);
}

function textOf(node: ReactNode): string {
  return renderToStaticMarkup(node as ReactElement).replace(/<[^>]+>/g, "").replace(/&#x27;/g, "'");
}

afterEach(() => {
  vi.mocked(createMask).mockReset();
  vi.mocked(previewMask).mockReset();
  vi.mocked(deleteMask).mockReset();
  vi.mocked(exportMask).mockReset();
  vi.mocked(open).mockReset();
  vi.unstubAllGlobals();
  maskStore.setStyle({ opacity: MASK_OPACITY_DEFAULT, tint: MASK_TINT_DEFAULT, displayInvert: false });
  maskStore.setMark(MASK_MARK_DEFAULT);
  historyStore.set(EMPTY_HISTORY_SNAPSHOT);
  hooks.mount();
});

describe("Masks tool", () => {
  it("masks_panel_creates_each_kind_and_lists_coverage", async () => {
    const KEY = "C:/data/m16_kinds.fits";
    const target = fileTarget(KEY);
    regionStore.add(KEY, region("r1", CIRCLE, true));
    regionStore.add(KEY, region("r2", BOX, false));
    regionStore.add(KEY, region("r3", POINT, true));
    backendCreates();
    vi.mocked(previewMask).mockResolvedValue({ width: 4, height: 3, cells: new Uint8Array(12) });
    vi.mocked(open).mockResolvedValue("D:/masks/cloud_mask.fits");
    const h = harness({ target, snapshot: snapshotFor(target, historyState(KEY, "n0002", [node("n0001", ARCSINH), node("n0002", CURVES)])) });
    const shown = { imageKey: KEY, basePath: null, inputPath: `${HISTORY_DIR}/n0002.fits` };

    h.click(kindButton("luminance"), "Luminance kind");
    h.toggle("Auto STF", true);
    await h.create();

    h.click(kindButton("range"), "Range kind");
    h.type("Low", "0.2");
    h.type("High", "0.8");
    h.type("Fuzz", "0.05");
    h.slide("Feather", 3);
    h.toggle("Invert", true);
    await h.create();

    h.click(kindButton("stars"), "Stars kind");
    h.slide("Feather", 0);
    h.toggle("Invert", false);
    await h.create();

    h.click(kindButton("regions"), "Regions kind");
    expect(textOf(h.view())).toContain(
      "1 included region and 1 excluded region from Image › Regions. Excluded regions cut holes in the mask. 1 line or point region has no area and is left out.",
    );
    await h.create();

    h.click(kindButton("file"), "File kind");
    h.click((el) => el.props["data-mask-pick-file"] !== undefined, "Pick FITS button");
    await vi.waitFor(() => expect(textOf(h.view())).toContain("cloud_mask.fits"));
    (find(h.view(), labelled("Mark mode"), "Mark mode select").props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "range" } });
    h.type("Mark Low", "1");
    h.type("Mark High", "5");
    await h.create();

    const plain = { feather_px: 0, invert: false };
    expect(vi.mocked(createMask).mock.calls).toEqual([
      [shown, { kind: "luminance", auto_stf: true }, { mods: plain }],
      [shown, { kind: "range", lo: 0.2, hi: 0.8, fuzz: 0.05 }, { mods: { feather_px: 3, invert: true } }],
      [shown, { kind: "stars", sigma: 5, config: STAR_DEFAULTS }, { mods: plain }],
      [shown, { kind: "regions", shapes: [CIRCLE], exclude_shapes: [BOX] }, { shapesSystem: "image", mods: plain }],
      [shown, { kind: "file", path: "D:/masks/cloud_mask.fits", mark: "range", lo: 1, hi: 5 }, { mods: plain }],
    ]);
    expect(h.props.onHistoryChanged).toHaveBeenCalledTimes(5);

    const tree = h.view();
    expect(rowIds(tree)).toEqual(["m01", "m02", "m03", "m04", "m05"]);
    expect(textOf(row(tree, "m01"))).toContain("coverage 100.00%");
    expect(textOf(row(tree, "m02"))).toContain("coverage 12.50%");
    expect(textOf(row(tree, "m03"))).toContain("812 stars");
    expect(row(tree, "m03").props["data-mask-stars"]).toBe(812);
    expect(textOf(row(tree, "m04"))).toContain("coverage 6.25%");
    expect(row(tree, "m05").props["data-mask-coverage"]).toBe(0.25);
    expect(textOf(find(tree, (el) => el.props["data-mask-result"] !== undefined, "result line"))).toBe("m05 · File · coverage 25.00% · 64×48");
    expect(maskStore.activeOf(KEY)).toBe("m05");
    expect(rowIds(tree).filter((id) => row(tree, id).props["data-mask-active"] === true)).toEqual(["m05"]);
    expect(h.html()).toContain("Range 0.2 to 0.8, fuzz 0.05, feather 3 px, inverted");
  });

  it("masks_panel_says_a_deleted_mask_stays_for_replay_and_restores_an_unused_one", async () => {
    const KEY = "C:/data/m16_delete.fits";
    const target = fileTarget(KEY);
    maskStore.recordCreated(KEY, created("m01", RANGE_SPEC, 0.125));
    maskStore.recordCreated(KEY, created("m02", LUMA_SPEC, 1));
    vi.mocked(deleteMask).mockImplementation(async (_scope, id) =>
      id === "m01"
        ? { mask_id: "m01", recipe: RANGE_SPEC, used_by: ["n0003"], kept_for_replay: true }
        : { mask_id: "m02", recipe: LUMA_SPEC, used_by: [], kept_for_replay: false },
    );
    vi.mocked(createMask).mockResolvedValue(created("m03", LUMA_SPEC, 1));
    const state = historyState(KEY, "n0003", [node("n0003", ARCSINH, "m01")]);
    const h = harness({ target, snapshot: snapshotFor(target, state) });

    h.click(labelled("Delete m01"), "Delete m01");
    expect(deleteMask).not.toHaveBeenCalled();
    h.click(labelled("Delete m01"), "Delete m01");
    await vi.waitFor(() => expect(rowIds(h.view())).toEqual(["m02"]));
    expect(deleteMask).toHaveBeenCalledWith(expect.objectContaining({ imageKey: KEY, basePath: null }), "m01");
    const kept = textOf(find(h.view(), (el) => el.props["data-mask-notice"] !== undefined, "notice"));
    expect(kept).toBe("m01 is no longer listed. n0003 (Arcsinh Stretch) still uses it, so its recipe stays in the history for replay.");
    expect(elements(h.view()).some(labelled("Restore m01"))).toBe(false);

    h.click(labelled("Delete m02"), "Delete m02");
    h.click(labelled("Delete m02"), "Delete m02");
    await vi.waitFor(() => expect(rowIds(h.view())).toEqual([]));
    expect(textOf(find(h.view(), (el) => el.props["data-mask-notice"] !== undefined, "notice"))).toContain("Deleted m02. No History step used it.");

    h.click(labelled("Restore m02"), "Restore m02");
    await vi.waitFor(() => expect(rowIds(h.view())).toEqual(["m03"]));
    expect(createMask).toHaveBeenCalledWith(expect.objectContaining({ imageKey: KEY, inputPath: `${HISTORY_DIR}/n0003.fits` }), LUMA_SPEC);
    expect(textOf(find(h.view(), (el) => el.props["data-mask-notice"] !== undefined, "notice"))).toBe("Restored m02 as m03.");
    expect(maskStore.activeOf(KEY)).toBe("m03");
    expect(h.props.onHistoryChanged).toHaveBeenCalledTimes(3);
  });

  it("masks_panel_measures_a_mask_listed_from_the_history", async () => {
    const KEY = "C:/data/m16_measure.fits";
    const target = fileTarget(KEY);
    maskStore.syncDoc(KEY, { masks: { m01: RANGE_SPEC } });
    vi.mocked(createMask).mockResolvedValue(created("m01", RANGE_SPEC, 0.125));
    const h = harness({ target, snapshot: snapshotFor(target, historyState(KEY, null, [], { m01: RANGE_SPEC })) });

    expect(textOf(row(h.view(), "m01"))).toContain("coverage not measured yet");
    h.click(labelled("Measure the coverage of m01"), "Measure m01");
    await vi.waitFor(() => expect(textOf(row(h.view(), "m01"))).toContain("coverage 12.50%"));
    expect(createMask).toHaveBeenCalledWith({ imageKey: KEY, basePath: null, inputPath: null }, RANGE_SPEC);
    expect(elements(h.view()).some(labelled("Measure the coverage of m01"))).toBe(false);
  });

  it("masks_panel_exports_float_and_label_fits_into_the_output_folder_and_keeps_them", async () => {
    const KEY = "C:/data/m16_export.fits";
    const target = fileTarget(KEY);
    maskStore.recordCreated(KEY, created("m01", REGIONS_SPEC, 0.0625));
    maskStore.recordCreated(KEY, created("m02", LUMA_SPEC, 1));
    vi.mocked(exportMask).mockImplementation(async (_target, id, path, kind) => ({
      mask_id: id,
      kind,
      fits_path: `${path}/m16_export_${id}_${kind === "label" ? "labels" : "mask"}.fits`,
      bitpix: kind === "label" ? 32 : -32,
      dims: [64, 48],
    }));
    const h = harness({ target, snapshot: snapshotFor(target, historyState(KEY, null, [])) });

    expect(elements(row(h.view(), "m02")).some(labelled("Export m02 as a label FITS"))).toBe(false);
    h.click(labelled("Export m01 as a label FITS"), "label export");
    await vi.waitFor(() => expect(exportMask).toHaveBeenCalledTimes(1));
    await settle();
    expect(exportMask).toHaveBeenCalledWith({ imageKey: KEY, basePath: null, inputPath: null }, "m01", "C:/out", "label");
    expect(textOf(find(h.view(), (el) => el.props["data-mask-notice"] !== undefined, "notice"))).toBe(
      "Wrote m16_export_m01_labels.fits (labels, BITPIX 32, 64×48) to the output folder.",
    );

    h.click(labelled("Export m02 as a float FITS"), "float export");
    await vi.waitFor(() => expect(exportMask).toHaveBeenCalledTimes(2));
    await settle();
    expect(exportMask).toHaveBeenLastCalledWith({ imageKey: KEY, basePath: null, inputPath: null }, "m02", "C:/out", "float");
    expect(maskStore.exportedPaths()).toEqual(expect.arrayContaining(["C:/out/m16_export_m01_labels.fits", "C:/out/m16_export_m02_mask.fits"]));
    expect(outputKeepList({ files: [], records: [], maskExports: maskStore.exportedPaths() })).toEqual(
      expect.arrayContaining(["C:/out/m16_export_m01_labels.fits", "C:/out/m16_export_m02_mask.fits"]),
    );
  });

  it("masks_panel_on_the_composite_targets_the_composite_and_says_masks_skip_compose", async () => {
    const KEY = "C:/data/m16_composite.fits";
    const target = fileTarget(KEY);
    backendCreates();
    const h = harness({ compositeMode: true, target, snapshot: snapshotFor(target, historyState(KEY, null, [])) });

    const note = find(h.view(), (el) => el.props["data-mask-composite-note"] !== undefined, "composite note");
    expect(textOf(note)).toBe("Masks apply to the Processing steps. Compose › Stretch and Compose › Adjust run without a mask in this version.");
    const regions = find(h.view(), kindButton("regions"), "Regions kind");
    expect(regions.props.disabled).toBe(true);
    expect(regions.props.title).toBe("Regions belong to a file's pixel grid. Show the file to make a regions mask.");

    await h.create();
    expect(vi.mocked(createMask).mock.calls[0][0]).toBe(COMPOSITE_MASK_TARGET);
    expect(h.props.onHistoryChanged).not.toHaveBeenCalled();
    expect(maskStore.masksOf(COMPOSITE_RUN_KEY).map((m) => m.id)).toEqual(["m01"]);
    expect(maskStore.activeOf(COMPOSITE_RUN_KEY)).toBe("m01");
    expect(maskStore.masksOf(KEY)).toEqual([]);
  });

  it("masks_panel_shows_backend_refusals_in_the_backends_words", async () => {
    const KEY = "C:/data/m16_refusal.fits";
    const target = fileTarget(KEY);
    const refusal = "mask file D:/masks/flipped.fits was not rotated or flipped like m16_refusal.fits (CDELT1 sign differs); redraw or re-export the mask";
    vi.mocked(createMask).mockRejectedValue(new Error(refusal));
    vi.mocked(open).mockResolvedValue("D:/masks/flipped.fits");
    const h = harness({ target, snapshot: snapshotFor(target, historyState(KEY, null, [])) });

    h.click(kindButton("file"), "File kind");
    h.click((el) => el.props["data-mask-pick-file"] !== undefined, "Pick FITS button");
    await vi.waitFor(() => expect(textOf(h.view())).toContain("flipped.fits"));
    await h.create();
    expect(find(h.view(), (el) => el.type === ErrorAlert, "error alert").props.message).toBe(refusal);
    expect(rowIds(h.view())).toEqual([]);
    expect(h.props.onHistoryChanged).not.toHaveBeenCalled();
  });

  it("masks_panel_waits_for_the_history_and_a_running_step_and_says_why_create_is_off", () => {
    const KEY = "C:/data/m16_wait.fits";
    const target = fileTarget(KEY);
    const ready = snapshotFor(target, historyState(KEY, null, []));
    const createButton = (tree: ReactNode) => find(tree, (el) => el.type === RunButton && el.props.label === "Create mask", "Create mask button");
    const why = (tree: ReactNode) => textOf(find(tree, (el) => el.props["data-mask-why"] !== undefined, "reason"));

    const loading = harness({ target, snapshot: { ...ready, pending: true } });
    expect(createButton(loading.view()).props.disabled).toBe(true);
    expect(why(loading.view())).toBe("Loading this image's history…");

    const running = harness({ target, snapshot: ready, runLocked: true });
    expect(createButton(running.view()).props.disabled).toBe(true);
    expect(why(running.view())).toBe("A step is still running on this image");

    const readOnly = harness({ target, snapshot: snapshotFor(target, { ...historyState(KEY, null, []), read_only: "this history was written by a newer AstroBurst; it is read-only" }) });
    expect(why(readOnly.view())).toBe("this history was written by a newer AstroBurst; it is read-only");

    const form = harness({ target, snapshot: ready });
    form.click(kindButton("range"), "Range kind");
    form.type("Low", "0.9");
    form.type("High", "0.1");
    expect(createButton(form.view()).props.disabled).toBe(true);
    expect(why(form.view())).toBe("Low must be at most High.");
    form.type("High", "");
    expect(why(form.view())).toBe("High: Enter a number");
    form.click(kindButton("regions"), "Regions kind");
    expect(why(form.view())).toBe("Draw at least one included region on the image first (Image › Regions).");
    form.click(kindButton("file"), "File kind");
    expect(why(form.view())).toBe("Pick a FITS file to read the mask from.");
  });

  it("masks_panel_draws_the_active_mask_with_the_store_style_and_keeps_it_after_closing", async () => {
    const KEY = "C:/data/m16_overlay.fits";
    const target = fileTarget(KEY);
    vi.stubGlobal("ImageData", class {
      constructor(
        public data: Uint8ClampedArray,
        public width: number,
        public height: number,
      ) {}
    });
    vi.stubGlobal("document", { createElement: () => ({ width: 0, height: 0, getContext: () => ({ putImageData: () => {} }) }) });
    maskStore.recordCreated(KEY, created("m01", RANGE_SPEC, 0.125));
    vi.mocked(previewMask).mockResolvedValue({ width: 4, height: 1, cells: new Uint8Array([0, 0, 200, 255]) });
    const state = historyState(KEY, "n0001", [node("n0001", ARCSINH)], { m01: RANGE_SPEC });
    const h = harness({ target, snapshot: snapshotFor(target, state) });
    const maskOf = () => overlayStore.getDoc(KEY).layers.find((l) => l.id === "mask") as (ReturnType<typeof overlayStore.getDoc>["layers"][number] & { maskId?: string; markedCells?: number }) | undefined;

    h.click(labelled("Show m01 over the image"), "Show m01");
    expect(maskStore.activeOf(KEY)).toBe("m01");
    h.view();
    await vi.waitFor(() => expect(maskOf()?.maskId).toBe("m01"));
    expect(maskOf()?.markedCells).toBe(2);
    expect(previewMask).toHaveBeenCalledWith({ imageKey: KEY, basePath: null, inputPath: `${HISTORY_DIR}/n0001.fits` }, "m01");

    maskStore.setStyle({ displayInvert: true });
    h.view();
    await vi.waitFor(() => expect(maskOf()?.markedCells).toBe(3));
    expect(previewMask).toHaveBeenCalledTimes(1);

    hooks.mount();
    expect(maskOf()?.maskId).toBe("m01");
    historyStore.set(snapshotFor(target, { ...state, doc: { ...(state.doc as HistoryDoc), masks: { m01: RANGE_SPEC } } }));
    expect(maskOf()?.maskId).toBe("m01");
    historyStore.set(snapshotFor(target, historyState(KEY, "n0002", [node("n0001", ARCSINH), node("n0002", CURVES)], { m01: RANGE_SPEC })));
    expect(maskOf()).toBeUndefined();
    expect(maskStore.activeOf(KEY)).toBe("m01");

    const reopened = harness({ target, snapshot: historyStore.get() });
    reopened.view();
    await vi.waitFor(() => expect(maskOf()?.maskId).toBe("m01"));
    expect(previewMask).toHaveBeenLastCalledWith({ imageKey: KEY, basePath: null, inputPath: `${HISTORY_DIR}/n0002.fits` }, "m01");
    maskStore.setActive(KEY, null);
    expect(maskOf()).toBeUndefined();
  });
});
