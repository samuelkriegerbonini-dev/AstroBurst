import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";

interface CapturedRun {
  label: string;
  onClick: () => void;
  running: boolean;
  describedBy?: string;
  disabled?: boolean;
}

const { hostProps, selects, runButtons, withPreviewMock, typedInvokeMock, captureHost } = vi.hoisted(() => {
  const hostProps = new Map<string, Record<string, unknown>>();
  const selects = new Map<string, Record<string, unknown>>();
  type Factory = (type: unknown, props: Record<string, unknown> | null, ...rest: unknown[]) => unknown;
  const captureHost = (factory: Factory): Factory => (type, props, ...rest) => {
    if (typeof type === "string" && props) {
      const id = props["data-testid"];
      if (typeof id === "string") hostProps.set(id, props);
      const section = props["data-mask-select"];
      if (typeof section === "string") selects.set(section, props);
    }
    return factory(type, props, ...rest);
  };
  return { hostProps, selects, runButtons: new Map<string, CapturedRun>(), withPreviewMock: vi.fn(), typedInvokeMock: vi.fn(), captureHost };
});

vi.mock("react/jsx-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-runtime")>();
  return { ...actual, jsx: captureHost(actual.jsx as never), jsxs: captureHost(actual.jsxs as never) };
});

vi.mock("react/jsx-dev-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-dev-runtime")>();
  return { ...actual, jsxDEV: captureHost(actual.jsxDEV as never) };
});

vi.mock("../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui")>();
  return {
    ...actual,
    RunButton: (props: CapturedRun) => {
      runButtons.set(props.label, props);
      return null;
    },
  };
});

vi.mock("../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri")>()),
  withPreview: withPreviewMock,
  typedInvoke: typedInvokeMock,
  getPreviewUrl: async (path: string) => `asset://${path}`,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf } from "../../../context/PreviewContext";
import { CurvesPanelView, type CurvesPanelProps } from "../../processing/CurvesPanel";
import LocalContrastPanel from "../../processing/LocalContrastPanel";
import HdrPanel from "../../processing/HdrPanel";
import WaveletPanel from "../../processing/WaveletPanel";
import DeconvolutionPanel from "../../processing/DeconvolutionPanel";
import ArcsinhStretchPanel from "../../processing/ArcsinhStretchPanel";
import PixelMathPanel from "../../processing/PixelMathPanel";
import BackgroundPanel from "../../processing/BackgroundPanel";
import MaskedStretchPanel from "../../processing/MaskedStretchPanel";
import type { CompositeInputView } from "../../processing/compositeProps";
import { IDENTITY_LEVELS } from "../../../services/toneFile";
import { DEFAULT_LHE_CONFIG } from "../../../shared/types/localContrast";
import { maskStore } from "../../../utils/maskStore";
import { historyRerunStore, type RerunRequest } from "../../../utils/historyRerunStore";
import { COMPOSITE_RUN_KEY } from "../../../utils/compositeChain";
import type { MaskCreateResult, MaskSpec } from "../../../shared/types/masks";
import type { OpSpec } from "../../../shared/types/ops";
import type { ProcessedFile } from "../../../shared/types";

const STARS: MaskSpec = { kind: "stars", sigma: 5, feather_px: 0, invert: false };
const LUMA: MaskSpec = { kind: "luminance", auto_stf: true, feather_px: 0, invert: false };
const RANGE: MaskSpec = { kind: "range", lo: 0.2, hi: 0.8, fuzz: 0.05, feather_px: 0, invert: false };
const STARS_LABEL = "m01 · Stars at 5σ";
const LUMA_LABEL = "m02 · Luminance, auto STF";
const MOVED_CURVE = [{ x: 0, y: 0 }, { x: 0.5, y: 0.7 }, { x: 1, y: 1 }];
const COMPOSITE_INPUT: CompositeInputView = { input: "background", previewUrl: "asset://out/background.png", label: "Background" };
const LHE_OP: OpSpec = { op: "lhe", v: 1, ...DEFAULT_LHE_CONFIG };
const GHS_OP: OpSpec = { op: "ghs", v: 1, d: 5, b: 2, sp: 0.05, lp: 0, hp: 1 };
const DELETED_REASON = "Mask m02 was deleted. Pick another mask, or make it again in the Masks tool.";

function fileOf(id: string): ProcessedFile {
  const path = `C:/data/${id}.fits`;
  return {
    id,
    name: `${id}.fits`,
    path,
    sourcePath: path,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: `C:/out/${id}.png`, previewUrl: `asset://C:/out/${id}.png`, dimensions: [64, 64], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function created(id: string, recipe: MaskSpec): MaskCreateResult {
  return { mask_id: id, kind: recipe.kind, coverage: 0.25, dims: [64, 64], stars: null, recipe };
}

function withMasks(key: string, ...masks: MaskCreateResult[]): void {
  for (const mask of masks) maskStore.recordCreated(key, mask);
}

function runResponse(node: string): Record<string, unknown> {
  return {
    node,
    parent: null,
    image_id: "3f9c0a1e5b7d2c44",
    history_dir: "C:/app/history/3f9c0a1e5b7d2c44",
    fits_path: `C:/app/history/3f9c0a1e5b7d2c44/${node}.fits`,
    png_path: `C:/app/history/3f9c0a1e5b7d2c44/${node}.png`,
    model_png: null,
    dimensions: [64, 64],
    is_rgb: false,
    domain: "linear",
    elapsed_ms: 3,
    previewUrl: `asset://out/${node}.png`,
    basePreviewUrl: "asset://out/base.png",
    chain_restarted: false,
    displayed: "stretched",
  };
}

interface PanelOptions {
  compositeMode?: boolean;
}

function panelProps(file: ProcessedFile, options: PanelOptions = {}): Record<string, unknown> {
  const compositeMode = options.compositeMode ?? false;
  return {
    selectedFile: file,
    outputDir: "C:/out",
    fileKey: fileKeyOf(file),
    compositeMode,
    compositeInput: compositeMode ? COMPOSITE_INPUT : null,
    onCompositeDone: () => {},
    fileName: file.name,
    disabledReason: null,
    disabledReasonId: "rgb-note",
  };
}

function wrap(file: ProcessedFile, panel: ReturnType<typeof createElement>): string {
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: panel });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function renderPanel(component: ComponentType<never>, file: ProcessedFile, options: PanelOptions = {}): string {
  selects.clear();
  runButtons.clear();
  return wrap(file, createElement(component, panelProps(file, options) as never));
}

function renderCurves(file: ProcessedFile, options: PanelOptions = {}): string {
  selects.clear();
  hostProps.clear();
  return wrap(
    file,
    createElement(CurvesPanelView, {
      ...(panelProps(file, options) as unknown as CurvesPanelProps),
      levels: IDENTITY_LEVELS,
      points: MOVED_CURVE,
      onLevelsChange: () => {},
      onPointsChange: () => {},
    }),
  );
}

function maskSelect(section: string): Record<string, unknown> {
  const props = selects.get(section);
  if (!props) throw new Error(`no Mask select in the ${section} Run row`);
  return props;
}

function choose(section: string, value: string): void {
  (maskSelect(section).onChange as (event: { target: { value: string } }) => void)({ target: { value } });
}

function run(label: string): CapturedRun {
  const button = runButtons.get(label);
  if (!button) throw new Error(`"${label}" not rendered`);
  return button;
}

function curvesRun(): Record<string, unknown> {
  const props = hostProps.get("curves-run");
  if (!props) throw new Error("curves-run not rendered");
  return props;
}

function options(html: string): { value: string; label: string; selected: boolean }[] {
  const select = /<select[^>]*aria-label="Mask"[^>]*>([\s\S]*?)<\/select>/.exec(html);
  if (!select) throw new Error("no Mask select in the markup");
  return [...select[1].matchAll(/<option value="([^"]*)"([^>]*)>([^<]*)<\/option>/g)].map((m) => ({ value: m[1], label: m[3], selected: m[2].includes("selected") }));
}

function selectedLabel(html: string): string | undefined {
  return options(html).find((o) => o.selected)?.label;
}

async function settled(render: () => unknown, running: () => boolean): Promise<void> {
  await vi.waitFor(() => {
    render();
    expect(running()).toBe(false);
  });
}

async function sentArgs(command: string): Promise<Record<string, unknown>> {
  await vi.waitFor(() => expect(withPreviewMock.mock.calls.some((call) => call[0] === command)).toBe(true));
  const call = withPreviewMock.mock.calls.filter((c) => c[0] === command).at(-1);
  return call?.[2] as Record<string, unknown>;
}

function rerunRequest(file: ProcessedFile, request: Omit<RerunRequest, "scope">): void {
  historyRerunStore.request({ ...request, scope: { imageKey: file.path, basePath: null } } as RerunRequest);
}

beforeEach(() => {
  withPreviewMock.mockReset();
  withPreviewMock.mockImplementation(async () => runResponse("n0001"));
  typedInvokeMock.mockReset();
  typedInvokeMock.mockResolvedValue({ node: "n0001", fits_path: "C:/out/n0001.fits" });
  historyRerunStore.clear();
});

describe("MaskSelect in the Run row", () => {
  it("mask_select_offers_no_mask_by_default_and_the_masks_of_the_shown_image", () => {
    const file = fileOf("curves-list");
    withMasks(file.path, created("m01", STARS), created("m02", LUMA));
    withMasks(fileOf("other-file").path, created("m03", RANGE));
    const html = renderCurves(file);
    expect(options(html)).toEqual([
      { value: "", label: "No mask", selected: true },
      { value: "m01", label: STARS_LABEL, selected: false },
      { value: "m02", label: LUMA_LABEL, selected: false },
    ]);
    const runAt = html.lastIndexOf("<button", html.indexOf('data-testid="curves-run"'));
    const between = html.slice(html.indexOf("</select>"), runAt);
    expect(html.indexOf("</select>")).toBeGreaterThan(-1);
    expect(between).not.toMatch(/<(button|input|select|a)\b|tabindex/);
  });

  it("mask_select_passes_the_mask_id_to_runOp", async () => {
    const file = fileOf("curves-mask");
    withMasks(file.path, created("m01", STARS), created("m02", LUMA));
    renderCurves(file);
    (curvesRun().onClick as () => void)();
    const unmasked = await sentArgs("run_op_cmd");
    expect(unmasked).not.toHaveProperty("mask");
    await settled(() => renderCurves(file), () => curvesRun()["data-running"] === true);
    withPreviewMock.mockClear();

    choose("curves", "m02");
    const html = renderCurves(file);
    expect(selectedLabel(html)).toBe(LUMA_LABEL);
    (curvesRun().onClick as () => void)();
    const args = await sentArgs("run_op_cmd");
    expect(args).toMatchObject({ imageKey: file.path, basePath: null, inputPath: file.path, mask: "m02" });
    expect((args.op as OpSpec).op).toBe("tone");
  });

  it("mask_select_passes_the_mask_to_the_composite_call", async () => {
    const file = fileOf("lhe-composite");
    withMasks(file.path, created("m01", RANGE));
    withMasks(COMPOSITE_RUN_KEY, created("m01", STARS));
    const html = renderPanel(LocalContrastPanel as ComponentType<never>, file, { compositeMode: true });
    expect(options(html).map((o) => o.label)).toEqual(["No mask", STARS_LABEL]);
    choose("local_contrast", "m01");
    renderPanel(LocalContrastPanel as ComponentType<never>, file, { compositeMode: true });
    run("Run Local Contrast").onClick();
    const args = await sentArgs("lhe_composite_cmd");
    expect(args).toMatchObject({ chainInput: "background", mask: "m01" });
    expect(withPreviewMock.mock.calls.some((call) => call[0] === "run_op_cmd")).toBe(false);
  });

  it("mask_select_is_absent_on_curves_in_composite_mode", () => {
    const file = fileOf("curves-composite");
    withMasks(file.path, created("m01", STARS));
    withMasks(COMPOSITE_RUN_KEY, created("m01", STARS));
    expect(renderCurves(file)).toContain('aria-label="Mask"');
    const html = renderCurves(file, { compositeMode: true });
    expect(html).not.toContain('aria-label="Mask"');
    expect(selects.has("curves")).toBe(false);
  });

  it.each([
    { name: "LocalContrastPanel", component: LocalContrastPanel, section: "local_contrast", label: "Run Local Contrast", op: "lhe", composite: "lhe_composite_cmd" },
    { name: "HdrPanel", component: HdrPanel, section: "hdr", label: "Run HDRMT", op: "hdrmt", composite: "hdrmt_composite_cmd" },
    { name: "WaveletPanel", component: WaveletPanel, section: "denoise", label: "Run Noise Reduction", op: "wavelet_denoise", composite: "composite_wavelet_denoise_cmd" },
    { name: "DeconvolutionPanel", component: DeconvolutionPanel, section: "deconvolution", label: "Run Deconvolution", op: "rl_deconv", composite: "composite_deconvolve_rl_cmd" },
    { name: "ArcsinhStretchPanel", component: ArcsinhStretchPanel, section: "stretch", label: "Apply Stretch (S=50)", op: "arcsinh", composite: "arcsinh_stretch_composite_cmd" },
    { name: "PixelMathPanel", component: PixelMathPanel, section: "pixelmath", label: "Run PixelMath", op: "pixelmath", composite: "pixelmath_composite_cmd" },
  ])("$name sends the picked mask on a file and on the composite", async ({ component, section, label, op, composite }) => {
    const file = fileOf(`${section}-mask`);
    withMasks(file.path, created("m01", STARS), created("m02", LUMA));
    withMasks(COMPOSITE_RUN_KEY, created("m01", RANGE));

    renderPanel(component as ComponentType<never>, file);
    expect(maskSelect(section)["aria-label"]).toBe("Mask");
    choose(section, "m02");
    renderPanel(component as ComponentType<never>, file);
    run(label).onClick();
    const fileArgs = await sentArgs("run_op_cmd");
    expect(fileArgs).toMatchObject({ imageKey: file.path, mask: "m02" });
    expect((fileArgs.op as OpSpec).op).toBe(op);
    await settled(() => renderPanel(component as ComponentType<never>, file), () => run(label).running);

    renderPanel(component as ComponentType<never>, file, { compositeMode: true });
    choose(section, "m01");
    renderPanel(component as ComponentType<never>, file, { compositeMode: true });
    run(label).onClick();
    expect(await sentArgs(composite)).toMatchObject({ chainInput: "background", mask: "m01" });
  });

  it("Background and Masked Stretch offer no Mask select", () => {
    const file = fileOf("no-mask-steps");
    withMasks(file.path, created("m01", STARS));
    expect(renderPanel(BackgroundPanel as ComponentType<never>, file)).not.toContain('aria-label="Mask"');
    expect(renderPanel(MaskedStretchPanel as ComponentType<never>, file)).not.toContain('aria-label="Mask"');
    expect(selects.size).toBe(0);
  });

  it("a picked id that now names another recipe is not used silently", () => {
    const file = fileOf("reused-id");
    withMasks(file.path, created("m01", STARS));
    renderPanel(LocalContrastPanel as ComponentType<never>, file);
    choose("local_contrast", "m01");
    maskStore.recordDeleted(file.path, { mask_id: "m01", recipe: STARS, used_by: [], kept_for_replay: false });
    withMasks(file.path, created("m01", LUMA));
    const html = renderPanel(LocalContrastPanel as ComponentType<never>, file);
    expect(selectedLabel(html)).toBe(`${STARS_LABEL} (not listed)`);
    expect(run("Run Local Contrast").disabled).toBe(true);
    expect(html).toContain("Mask m01 is no longer listed for this image. Pick a mask again.");
  });
});

describe("Re-run with… and the mask", () => {
  it("Re-run with restores the mask the step used and runs with it", async () => {
    const file = fileOf("rerun-mask");
    withMasks(file.path, created("m01", STARS), created("m02", LUMA));
    rerunRequest(file, { section: "local_contrast", op: LHE_OP, node: "n0003", downstream: false, mask: { id: "m01", recipe: STARS, deleted: false } } as never);
    const html = renderPanel(LocalContrastPanel as ComponentType<never>, file);
    expect(selectedLabel(html)).toBe(STARS_LABEL);
    expect(run("Run Local Contrast").disabled).toBe(false);
    run("Run Local Contrast").onClick();
    expect(await sentArgs("run_op_cmd")).toMatchObject({ mask: "m01" });
  });

  it("Re-run with names a deleted mask instead of dropping it and keeps Run off until another is picked", async () => {
    const file = fileOf("rerun-deleted");
    withMasks(file.path, created("m01", STARS));
    rerunRequest(file, { section: "local_contrast", op: LHE_OP, node: "n0003", downstream: false, mask: { id: "m02", recipe: RANGE, deleted: true } } as never);
    const html = renderPanel(LocalContrastPanel as ComponentType<never>, file);
    expect(selectedLabel(html)).toBe("m02 · Range 0.2 to 0.8, fuzz 0.05 (deleted)");
    expect(run("Run Local Contrast").disabled).toBe(true);
    const note = /<div id="([^"]+)" role="note" data-mask-blocked=""[^>]*>([^<]*)<\/div>/.exec(html);
    expect(note?.[2]).toBe(DELETED_REASON);
    expect(run("Run Local Contrast").describedBy?.split(" ")).toContain(note?.[1]);
    expect(html).toContain(`title="${DELETED_REASON}"`);

    choose("local_contrast", "");
    const unblocked = renderPanel(LocalContrastPanel as ComponentType<never>, file);
    expect(selectedLabel(unblocked)).toBe("No mask");
    expect(run("Run Local Contrast").disabled).toBe(false);
    run("Run Local Contrast").onClick();
    expect(await sentArgs("run_op_cmd")).not.toHaveProperty("mask");
  });

  it("a GHS re-run restores its mask in the Stretch row", async () => {
    const file = fileOf("rerun-ghs");
    withMasks(file.path, created("m01", STARS));
    rerunRequest(file, { section: "stretch", op: GHS_OP, node: "n0004", downstream: false, mask: { id: "m01", recipe: STARS, deleted: false } } as never);
    const html = renderPanel(ArcsinhStretchPanel as ComponentType<never>, file);
    expect(selectedLabel(html)).toBe(STARS_LABEL);
    run("Apply GHS Stretch (D=5.00)").onClick();
    const args = await sentArgs("run_op_cmd");
    expect(args).toMatchObject({ mask: "m01" });
    expect((args.op as OpSpec).op).toBe("ghs");
  });

  it("an inline recorded mask is offered by its recipe and sent as that recipe", async () => {
    const file = fileOf("rerun-inline");
    rerunRequest(file, { section: "local_contrast", op: LHE_OP, node: "n0002", downstream: false, mask: { id: null, recipe: RANGE, deleted: false } } as never);
    const html = renderPanel(LocalContrastPanel as ComponentType<never>, file);
    expect(selectedLabel(html)).toBe("Recorded · Range 0.2 to 0.8, fuzz 0.05");
    run("Run Local Contrast").onClick();
    expect(await sentArgs("run_op_cmd")).toMatchObject({ mask: RANGE });
  });

  it("a downstream re-run shows the step's mask and locks the selector, since the re-run keeps it", () => {
    const file = fileOf("rerun-downstream");
    withMasks(file.path, created("m01", STARS));
    rerunRequest(file, { section: "local_contrast", op: LHE_OP, node: "n0003", downstream: true, mask: { id: "m02", recipe: RANGE, deleted: true } } as never);
    const html = renderPanel(LocalContrastPanel as ComponentType<never>, file);
    expect(selectedLabel(html)).toBe("m02 · Range 0.2 to 0.8, fuzz 0.05 (deleted)");
    expect(maskSelect("local_contrast").disabled).toBe(true);
    expect(maskSelect("local_contrast").title).toBe("Re-running n0003 keeps the mask it used.");
    expect(run("Run Local Contrast").disabled).toBe(false);
    expect(html).not.toContain("data-mask-blocked");
  });
});
