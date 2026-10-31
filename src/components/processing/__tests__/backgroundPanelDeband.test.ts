import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { captured, choices, clicks, extractBackgroundMock, compositeBackgroundMock, chooseDuringRender } = vi.hoisted(() => {
  const choices = new Map<string, string[]>();
  const clicks = new Set<string>();
  type Factory = (type: unknown, props: Record<string, unknown> | null, ...rest: unknown[]) => unknown;
  const chooseDuringRender = (factory: Factory): Factory => (type, props, ...rest) => {
    const id = props?.["data-testid"];
    const queue = typeof type === "string" && typeof id === "string" ? choices.get(id) : undefined;
    const value = queue?.shift();
    if (value !== undefined) (props?.onChange as (e: { target: { value: string } }) => void)({ target: { value } });
    const text = props?.children;
    if (type === "button" && typeof text === "string" && clicks.delete(text)) (props?.onClick as () => void)();
    return factory(type, props, ...rest);
  };
  return {
    captured: { onClick: null as null | (() => void), label: null as string | null, runningLabel: null as string | null },
    choices,
    clicks,
    extractBackgroundMock: vi.fn(),
    compositeBackgroundMock: vi.fn(),
    chooseDuringRender,
  };
});

vi.mock("react/jsx-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-runtime")>();
  return { ...actual, jsx: chooseDuringRender(actual.jsx as never), jsxs: chooseDuringRender(actual.jsxs as never) };
});

vi.mock("react/jsx-dev-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-dev-runtime")>();
  return { ...actual, jsxDEV: chooseDuringRender(actual.jsxDEV as never) };
});

vi.mock("../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui")>();
  return {
    ...actual,
    RunButton: (props: { onClick: () => void; label: string; runningLabel?: string }) => {
      captured.onClick = props.onClick;
      captured.label = props.label;
      captured.runningLabel = props.runningLabel ?? null;
      return null;
    },
  };
});

vi.mock("../../../services/processing", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/processing")>()),
  recordExtractBackground: extractBackgroundMock,
}));

vi.mock("../../../services/compositeChain", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/compositeChain")>()),
  compositeBackground: compositeBackgroundMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { withStep } from "../../../utils/processingChain";
import { historyRerunStore } from "../../../utils/historyRerunStore";
import BackgroundPanel from "../BackgroundPanel";
import type { CompositeInputView } from "../compositeProps";
import type { ProcessedFile } from "../../../shared/types";
import type { BackgroundResult } from "../../../shared/types/processing";

const RAW = "C:/data/banded_2048.fits";
const CORRECTED = "C:/out/banded_2048_bg_corrected.fits";
const COMPOSITE_INPUT: CompositeInputView = { input: "base", previewUrl: "asset://out/blend.png", label: "Composite" };
const SPLINE_LINE = "De-banding runs with the Polynomial model: set Model to Polynomial to de-band";
const COMPOSITE_LINE = "De-banding works on a single file, not on the composite";
const DEBAND_AXES: [string, string, string][] = [
  ["deband_auto", "Auto (detect)", "De-band (auto axis)"],
  ["deband_rows", "Rows", "De-band (rows)"],
  ["deband_cols", "Columns", "De-band (columns)"],
  ["deband_both", "Both", "De-band (both axes)"],
];

interface Radio {
  value: string;
  checked: boolean;
  disabled: boolean;
  label: string;
  name: string;
}

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "banded_2048.fits",
    path: RAW,
    sourcePath: RAW,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/banded.png", previewUrl: "asset://C:/out/banded.png", dimensions: [2048, 2048], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function backgroundResult(axis: BackgroundResult["axis"]): BackgroundResult {
  return {
    corrected_png: "C:/out/banded_2048_bg_corrected.png",
    corrected_fits: CORRECTED,
    cache_key: CORRECTED,
    model_png: "C:/out/banded_2048_bg_model.png",
    previewUrl: "asset://C:/out/banded_2048_bg_corrected.png",
    modelUrl: "asset://C:/out/banded_2048_bg_model.png",
    dimensions: [2048, 2048],
    sample_count: axis ? undefined : 64,
    rms_residual: axis ? undefined : 0.0123,
    elapsed_ms: 1500,
    axis,
  };
}

function choose(testId: string, ...values: string[]): void {
  choices.set(testId, values);
}

function render(file: ProcessedFile, overrides: Record<string, unknown> = {}, onProcessingDone: (res: unknown) => void = () => {}): string {
  const panel = createElement(BackgroundPanel, {
    selectedFile: file,
    outputDir: "C:/out",
    onProcessingDone,
    fileKey: fileKeyOf(file),
    compositeMode: false,
    compositeInput: null,
    onCompositeDone: () => {},
    fileName: file.name,
    disabledReason: null,
    ...overrides,
  });
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: panel });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function publishBackground(file: ProcessedFile): void {
  const key = fileKeyOf(file) as string;
  const step = { fitsPath: CORRECTED, previewUrl: "asset://C:/out/banded_2048_bg_corrected.png", dimensions: [2048, 2048] as [number, number] };
  function Publisher() {
    const { publishProcessed } = useRenderActions();
    publishProcessed(key, { ...step, label: "Background", kind: "processing", inputPath: RAW }, (c) => withStep(c, "background", step));
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function textOf(markup: string): string {
  return markup.replace(/<!-- -->/g, "").replace(/<[^>]*>/g, "").replace(/&#x27;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, "&").trim();
}

function radioGroup(html: string, testId: string): { describedBy: string | null; radios: Radio[] } | null {
  const match = new RegExp(`<div([^>]*role="radiogroup"[^>]*data-testid="${testId}"[^>]*)>([\\s\\S]*?)</div>`).exec(html);
  if (!match) return null;
  const radios = [...match[2].matchAll(/<label[^>]*>(<input[^>]*>)([\s\S]*?)<\/label>/g)].map((m) => ({
    value: /value="([^"]*)"/.exec(m[1])?.[1] ?? "",
    checked: /checked=""/.test(m[1]),
    disabled: /disabled=""/.test(m[1]),
    label: textOf(m[2]),
    name: /name="([^"]*)"/.exec(m[1])?.[1] ?? "",
  }));
  return { describedBy: /aria-describedby="([^"]*)"/.exec(match[1])?.[1] ?? null, radios };
}

function radios(html: string, testId: string): Radio[] {
  const group = radioGroup(html, testId);
  if (!group) throw new Error(`radiogroup ${testId} not rendered`);
  return group.radios;
}

function checkedValue(html: string, testId: string): string | undefined {
  return radios(html, testId).find((r) => r.checked)?.value;
}

function sliderLabels(html: string): string[] {
  return [...html.matchAll(/<label class="ab-slider-label" for="[^"]+">([^<]*)/g)].map((m) => m[1]);
}

function rangeOf(html: string, label: string): { min: string; max: string; value: string } {
  const id = new RegExp(`<label class="ab-slider-label" for="([^"]+)">${label}<`).exec(html)?.[1];
  const tag = id ? new RegExp(`<input[^>]*id="${id}"[^>]*>`).exec(html)?.[0] : undefined;
  if (!tag) throw new Error(`no slider ${label}`);
  const attr = (name: string) => new RegExp(`${name}="([^"]*)"`).exec(tag)?.[1] ?? "";
  return { min: attr("min"), max: attr("max"), value: attr("value") };
}

function noteWith(html: string, text: string): string {
  const note = [...html.matchAll(/<(div|p)([^>]*role="note"[^>]*)>([\s\S]*?)<\/\1>/g)].find((m) => textOf(m[3]) === text);
  const id = note ? /id="([^"]+)"/.exec(note[2])?.[1] : undefined;
  if (!id) throw new Error(`no note "${text}"`);
  return id;
}

function clickRun(): void {
  if (!captured.onClick) throw new Error("RunButton not rendered");
  captured.onClick();
}

beforeEach(() => {
  choices.clear();
  clicks.clear();
  historyRerunStore.clear();
  captured.onClick = null;
  captured.label = null;
  captured.runningLabel = null;
  extractBackgroundMock.mockReset();
  compositeBackgroundMock.mockReset();
});

describe("BackgroundPanel operation and de-band", () => {
  it("offers Operation as Remove gradient or De-band and, for Remove gradient, Apply as with when to use each", () => {
    const html = render(fileOf("deband-options"));
    const operation = radios(html, "background-operation");
    const applyAs = radios(html, "background-apply-as");
    expect(operation.map(({ value, checked, disabled, label }) => ({ value, checked, disabled, label }))).toEqual([
      { value: "gradient", checked: true, disabled: false, label: "Remove gradient" },
      { value: "deband", checked: false, disabled: false, label: "De-band" },
    ]);
    expect(applyAs.map(({ value, checked, disabled, label }) => ({ value, checked, disabled, label }))).toEqual([
      { value: "subtract", checked: true, disabled: false, label: "Subtract (additive: sky glow, light pollution)" },
      { value: "divide", checked: false, disabled: false, label: "Divide (multiplicative: vignetting; needs a positive model)" },
    ]);
    expect(new Set(operation.map((r) => r.name)).size).toBe(1);
    expect(new Set(applyAs.map((r) => r.name)).size).toBe(1);
    expect(operation[0].name).not.toBe(applyAs[0].name);
    expect(html).not.toContain('data-testid="background-mode"');
    expect(radioGroup(html, "background-axis")).toBeNull();
    expect(sliderLabels(html)).toEqual(["Grid Size", "Polynomial Degree", "Sigma Clip", "Iterations"]);
    expect(rangeOf(html, "Iterations")).toMatchObject({ min: "1", max: "10" });
    expect(captured.label).toBe("Extract Background");
  });

  it("De-band shows only the axis, Sigma Clip and Iterations up to 5, and Run names the axis", () => {
    choose("background-operation-deband", "deband");
    const html = render(fileOf("deband-controls"));
    expect(checkedValue(html, "background-operation")).toBe("deband");
    expect(radios(html, "background-axis").map(({ value, checked, label }) => [value, checked, label])).toEqual(
      DEBAND_AXES.map(([value, label], i) => [value, i === 0, label]),
    );
    expect(sliderLabels(html)).toEqual(["Sigma Clip", "Iterations"]);
    expect(html).not.toContain("Grid Size");
    expect(html).not.toContain("Polynomial Degree");
    expect(rangeOf(html, "Iterations")).toMatchObject({ min: "1", max: "5", value: "3" });
    expect(radioGroup(html, "background-apply-as")).toBeNull();
    expect(captured.label).toBe("De-band (auto axis)");
    expect(captured.runningLabel).toBe("De-banding...");
    for (const [axis, , runLabel] of DEBAND_AXES) {
      choose("background-operation-deband", "deband");
      choose(`background-axis-${axis}`, axis);
      render(fileOf(`deband-axis-${axis}`));
      expect(captured.label).toBe(runLabel);
    }
  });

  it("choosing De-band and running records deband_auto with only the de-band settings; the result reads De-banded with its Axis cell", async () => {
    const file = fileOf("deband-run");
    const done = vi.fn();
    extractBackgroundMock.mockResolvedValue(backgroundResult("rows"));
    choose("background-operation-deband", "deband");
    render(file, {}, done);
    clickRun();
    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(extractBackgroundMock).toHaveBeenCalledWith({ imageKey: RAW, basePath: null }, RAW, { sigmaClip: 2.5, iterations: 3, mode: "deband_auto" });
    expect(done.mock.calls[0][0]).toMatchObject({ corrected_fits: CORRECTED, axis: "rows" });
    publishBackground(file);
    const html = render(file);
    expect(html).toMatch(/data-testid="background-deband-axis"[^>]*>rows</);
    expect(html).toContain(">Axis<");
    expect(html).not.toContain(">Samples<");
    expect(html).not.toContain(">RMS<");
    expect(html).toContain('<div class="ab-compare-label left-2">De-banded</div>');
  });

  it("brings iterations set for the gradient down to the 5 that De-band runs", async () => {
    historyRerunStore.request({
      section: "background",
      op: { op: "background", v: 1, model: "polynomial", config: { gridSize: 8, polyDegree: 3, sigmaClip: 2, iterations: 8, mode: "subtract" } },
      node: "n0002",
      downstream: false,
      scope: { imageKey: RAW, basePath: null },
    });
    const file = fileOf("deband-iterations");
    const done = vi.fn();
    extractBackgroundMock.mockResolvedValue(backgroundResult("cols"));
    choose("background-operation-deband", "deband");
    const html = render(file, {}, done);
    expect(rangeOf(html, "Iterations")).toMatchObject({ max: "5", value: "5" });
    clickRun();
    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(extractBackgroundMock.mock.calls[0][2]).toEqual({ sigmaClip: 2, iterations: 5, mode: "deband_auto" });
  });

  it("a subtract result keeps Samples and RMS, has no Axis cell and reads Background Removed", async () => {
    const file = fileOf("deband-subtract");
    const done = vi.fn();
    extractBackgroundMock.mockResolvedValue(backgroundResult(null));
    render(file, {}, done);
    clickRun();
    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(extractBackgroundMock.mock.calls[0][2]).toEqual({ gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" });
    publishBackground(file);
    const html = render(file);
    expect(html).toContain(">Samples<");
    expect(html).toContain(">RMS<");
    expect(html).not.toContain('data-testid="background-deband-axis"');
    expect(html).toContain('<div class="ab-compare-label left-2">Background Removed</div>');
  });

  it("the Model view of a de-band result reads Band Pattern, and a gradient result keeps Background Model", async () => {
    for (const [axis, label] of [["rows", "Band Pattern"], [null, "Background Model"]] as const) {
      const file = fileOf(`deband-model-view-${axis ?? "gradient"}`);
      const done = vi.fn();
      extractBackgroundMock.mockResolvedValue(backgroundResult(axis));
      if (axis) choose("background-operation-deband", "deband");
      render(file, {}, done);
      clickRun();
      await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
      publishBackground(file);
      clicks.add("Model");
      const html = render(file);
      expect(clicks.has("Model")).toBe(false);
      expect(html).toContain(`<div class="ab-compare-label left-2">${label}</div>`);
      expect(html).toContain(`alt="${label}"`);
      expect(html).toContain('src="asset://C:/out/banded_2048_bg_model.png');
    }
  });

  it("under the Spline model, disables De-band with a visible line the group points to, and keeps Apply as", () => {
    choose("background-model", "spline");
    const html = render(fileOf("deband-spline"));
    expect(radios(html, "background-operation").map(({ value, checked, disabled }) => ({ value, checked, disabled }))).toEqual([
      { value: "gradient", checked: true, disabled: false },
      { value: "deband", checked: false, disabled: true },
    ]);
    expect(radioGroup(html, "background-operation")?.describedBy).toBe(noteWith(html, SPLINE_LINE));
    expect(radios(html, "background-apply-as").map((r) => r.disabled)).toEqual([false, false]);
    expect(radioGroup(html, "background-axis")).toBeNull();
    expect(captured.label).toBe("Extract Background (Spline)");
  });

  it("in composite mode, disables De-band with the visible composite line, also under the Spline model", () => {
    for (const model of ["polynomial", "spline"]) {
      choose("background-model", model);
      const html = render(fileOf(`deband-composite-${model}`), { compositeMode: true, compositeInput: COMPOSITE_INPUT });
      expect(radios(html, "background-operation")[1]).toMatchObject({ value: "deband", checked: false, disabled: true });
      expect(radioGroup(html, "background-operation")?.describedBy).toBe(noteWith(html, COMPOSITE_LINE));
      expect(html).not.toContain(SPLINE_LINE);
      expect(radios(html, "background-apply-as").map((r) => r.disabled)).toEqual([false, false]);
    }
  });

  it("a De-band choice held from the file view runs the composite with the last Apply as choice, which the group shows", async () => {
    compositeBackgroundMock.mockResolvedValue({ previewUrl: "asset://out/c.png", sample_count: [1, 1, 1], rms_residual: [0, 0, 0], elapsed_ms: 1 });
    choose("background-apply-as-divide", "divide");
    choose("background-operation-deband", "deband");
    const html = render(fileOf("deband-held"), { compositeMode: true, compositeInput: COMPOSITE_INPUT, onCompositeDone: vi.fn() });
    expect(choices.get("background-apply-as-divide")).toEqual([]);
    expect(choices.get("background-operation-deband")).toEqual([]);
    expect(checkedValue(html, "background-apply-as")).toBe("divide");
    expect(checkedValue(html, "background-operation")).toBe("gradient");
    clickRun();
    await vi.waitFor(() => expect(compositeBackgroundMock).toHaveBeenCalledTimes(1));
    expect(compositeBackgroundMock.mock.calls[0][2]).toMatchObject({ model: "polynomial", mode: "divide" });
  });

  it("records a Spline run on the composite as DBE, and a Polynomial run as Background Removed", async () => {
    for (const [model, label] of [["spline", "DBE"], ["polynomial", "Background Removed"]]) {
      compositeBackgroundMock.mockReset();
      compositeBackgroundMock.mockResolvedValue({ previewUrl: `asset://out/${model}.png`, sample_count: [1, 1, 1], rms_residual: [0, 0, 0], elapsed_ms: 1 });
      const onCompositeDone = vi.fn();
      choose("background-model", model);
      render(fileOf(`composite-label-${model}`), { compositeMode: true, compositeInput: COMPOSITE_INPUT, onCompositeDone });
      clickRun();
      await vi.waitFor(() => expect(onCompositeDone).toHaveBeenCalledTimes(1));
      expect(compositeBackgroundMock.mock.calls[0][2]).toMatchObject({ model });
      expect(onCompositeDone.mock.calls[0].slice(0, 2)).toEqual(["background", label]);
    }
  });
});
