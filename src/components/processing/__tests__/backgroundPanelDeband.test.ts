import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { captured, choices, extractBackgroundMock, compositeBackgroundMock, chooseDuringRender } = vi.hoisted(() => {
  const choices = new Map<string, string[]>();
  type Factory = (type: unknown, props: Record<string, unknown> | null, ...rest: unknown[]) => unknown;
  const chooseDuringRender = (factory: Factory): Factory => (type, props, ...rest) => {
    const id = props?.["data-testid"];
    const queue = typeof type === "string" && typeof id === "string" ? choices.get(id) : undefined;
    const value = queue?.shift();
    if (value !== undefined) (props?.onChange as (e: { target: { value: string } }) => void)({ target: { value } });
    return factory(type, props, ...rest);
  };
  return {
    captured: { onClick: null as null | (() => void) },
    choices,
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
    RunButton: (props: { onClick: () => void }) => {
      captured.onClick = props.onClick;
      return null;
    },
  };
});

vi.mock("../../../services/processing", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/processing")>()),
  extractBackground: extractBackgroundMock,
}));

vi.mock("../../../services/compositeChain", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/compositeChain")>()),
  compositeBackground: compositeBackgroundMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { withStep } from "../../../utils/processingChain";
import BackgroundPanel from "../BackgroundPanel";
import type { CompositeInputView } from "../compositeProps";
import type { ProcessedFile } from "../../../shared/types";
import type { BackgroundResult } from "../../../shared/types/processing";

const RAW = "C:/data/banded_2048.fits";
const CORRECTED = "C:/out/banded_2048_bg_corrected.fits";
const COMPOSITE_INPUT: CompositeInputView = { input: "base", previewUrl: "asset://out/blend.png", label: "Composite" };
const DEBAND_OPTIONS: [string, string][] = [
  ["deband_auto", "De-band auto (detect axis)"],
  ["deband_rows", "De-band rows"],
  ["deband_cols", "De-band columns"],
  ["deband_both", "De-band both axes"],
];

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

function optionTag(html: string, value: string): string {
  const match = new RegExp(`<option value="${value}"[^>]*>[^<]*</option>`).exec(html);
  if (!match) throw new Error(`option ${value} not rendered`);
  return match[0];
}

function clickRun(): void {
  if (!captured.onClick) throw new Error("RunButton not rendered");
  captured.onClick();
}

beforeEach(() => {
  choices.clear();
  captured.onClick = null;
  extractBackgroundMock.mockReset();
  compositeBackgroundMock.mockReset();
});

describe("BackgroundPanel de-band modes", () => {
  it("offers the four de-band modes after Subtract and Divide under the polynomial model", () => {
    const html = render(fileOf("deband-options"));
    expect(optionTag(html, "subtract")).toBe('<option value="subtract" selected="">Subtract</option>');
    expect(optionTag(html, "divide")).toBe('<option value="divide">Divide</option>');
    for (const [value, label] of DEBAND_OPTIONS) expect(optionTag(html, value)).toBe(`<option value="${value}">${label}</option>`);
  });

  it("choosing deband_auto and running calls extractBackground with mode deband_auto, and the Axis cell shows the result's axis", async () => {
    const file = fileOf("deband-run");
    const done = vi.fn();
    extractBackgroundMock.mockResolvedValue(backgroundResult("rows"));
    choose("background-mode", "deband_auto");
    expect(optionTag(render(file, {}, done), "deband_auto")).toContain('selected=""');
    clickRun();
    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(extractBackgroundMock).toHaveBeenCalledWith(RAW, "C:/out", { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "deband_auto" });
    expect(done.mock.calls[0][0]).toMatchObject({ corrected_fits: CORRECTED, axis: "rows" });
    publishBackground(file);
    const html = render(file);
    expect(html).toMatch(/data-testid="background-deband-axis"[^>]*>rows</);
    expect(html).toContain(">Axis<");
    expect(html).not.toContain(">Samples<");
    expect(html).not.toContain(">RMS<");
  });

  it("a subtract result keeps Samples and RMS and has no Axis cell", async () => {
    const file = fileOf("deband-subtract");
    const done = vi.fn();
    extractBackgroundMock.mockResolvedValue(backgroundResult(null));
    render(file, {}, done);
    clickRun();
    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(extractBackgroundMock.mock.calls[0][2]).toMatchObject({ mode: "subtract" });
    publishBackground(file);
    const html = render(file);
    expect(html).toContain(">Samples<");
    expect(html).toContain(">RMS<");
    expect(html).not.toContain('data-testid="background-deband-axis"');
  });

  it("disables the four de-band options under the Spline model and keeps Subtract and Divide", () => {
    choose("background-model", "spline");
    const html = render(fileOf("deband-spline"));
    expect(optionTag(html, "subtract")).not.toContain("disabled");
    expect(optionTag(html, "divide")).not.toContain("disabled");
    for (const [value] of DEBAND_OPTIONS) expect(optionTag(html, value)).toContain('disabled=""');
  });

  it("disables the de-band options in composite mode, where the composite background has no de-band", () => {
    const html = render(fileOf("deband-composite"), { compositeMode: true, compositeInput: COMPOSITE_INPUT });
    for (const [value] of DEBAND_OPTIONS) expect(optionTag(html, value)).toContain('disabled=""');
    expect(optionTag(html, "subtract")).not.toContain("disabled");
  });

  it("a de-band mode held from the file view runs the composite with the last Subtract/Divide choice, which the select shows", async () => {
    compositeBackgroundMock.mockResolvedValue({ previewUrl: "asset://out/c.png", sample_count: [1, 1, 1], rms_residual: [0, 0, 0], elapsed_ms: 1 });
    choose("background-mode", "divide", "deband_rows");
    const html = render(fileOf("deband-held"), { compositeMode: true, compositeInput: COMPOSITE_INPUT, onCompositeDone: vi.fn() });
    expect(choices.get("background-mode")).toEqual([]);
    expect(optionTag(html, "divide")).toContain('selected=""');
    expect(optionTag(html, "deband_rows")).not.toContain("selected");
    clickRun();
    await vi.waitFor(() => expect(compositeBackgroundMock).toHaveBeenCalledTimes(1));
    expect(compositeBackgroundMock.mock.calls[0][2]).toMatchObject({ model: "polynomial", mode: "divide" });
  });
});
