import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { captured, clicks, extractBackgroundMock, clickDuringRender } = vi.hoisted(() => {
  const clicks = new Set<string>();
  type Factory = (type: unknown, props: Record<string, unknown> | null, ...rest: unknown[]) => unknown;
  const clickDuringRender = (factory: Factory): Factory => (type, props, ...rest) => {
    const text = props?.children;
    if (type === "button" && typeof text === "string" && clicks.delete(text)) (props?.onClick as () => void)();
    return factory(type, props, ...rest);
  };
  return {
    captured: { onClick: null as null | (() => void) },
    clicks,
    extractBackgroundMock: vi.fn(),
    clickDuringRender,
  };
});

vi.mock("react/jsx-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-runtime")>();
  return { ...actual, jsx: clickDuringRender(actual.jsx as never), jsxs: clickDuringRender(actual.jsxs as never) };
});

vi.mock("react/jsx-dev-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-dev-runtime")>();
  return { ...actual, jsxDEV: clickDuringRender(actual.jsxDEV as never) };
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
  recordExtractBackground: extractBackgroundMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { withStep } from "../../../utils/processingChain";
import BackgroundPanel from "../BackgroundPanel";
import type { ProcessedFile } from "../../../shared/types";
import type { BackgroundResult } from "../../../shared/types/processing";

const STACK = "C:/out/light_001_stack3_20261010-120000-000.fits";
const STACK_LABEL = "Stack - 3 frames";

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "light_001.fits",
    path: STACK,
    sourcePath: "C:/data/light_001.fits",
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/stack.png", previewUrl: "asset://C:/out/stack.png", dimensions: [66, 50], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function correctedOf(id: string): string {
  return `C:/out/${id}_bg_corrected.fits`;
}

function backgroundResult(id: string): BackgroundResult {
  return {
    corrected_png: `C:/out/${id}_bg_corrected.png`,
    corrected_fits: correctedOf(id),
    cache_key: correctedOf(id),
    model_png: `C:/out/${id}_bg_model.png`,
    previewUrl: `asset://C:/out/${id}_bg_corrected.png`,
    modelUrl: `asset://C:/out/${id}_bg_model.png`,
    dimensions: [66, 50],
    sample_count: 64,
    rms_residual: 0.0123,
    elapsed_ms: 1500,
    axis: null,
  };
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
  const step = { fitsPath: correctedOf(file.id), previewUrl: `asset://C:/out/${file.id}_bg_corrected.png`, dimensions: [66, 50] as [number, number] };
  function Publisher() {
    useRenderActions().publishProcessed(key, { ...step, label: "Background", kind: "processing", inputPath: STACK }, (c) => withStep(c, "background", step));
    return null;
  }
  renderToStaticMarkup(createElement(CompositeProvider, { children: createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) }) }));
}

async function runAndCompare(file: ProcessedFile, overrides: Record<string, unknown>): Promise<string> {
  const done = vi.fn();
  extractBackgroundMock.mockResolvedValue(backgroundResult(file.id));
  render(file, overrides, done);
  if (!captured.onClick) throw new Error("RunButton not rendered");
  captured.onClick();
  await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
  publishBackground(file);
  clicks.add("Compare");
  return render(file, overrides);
}

function compareLabels(html: string): { caption: string; left: string | null } {
  const caption = /<span class="text-xs text-zinc-400">([\s\S]*?)<\/span>/.exec(html)?.[1]?.replace(/<!-- -->/g, "") ?? "";
  const left = /<div class="ab-compare-label left-2">([^<]*)<\/div>/.exec(html)?.[1] ?? null;
  return { caption, left };
}

beforeEach(() => {
  clicks.clear();
  captured.onClick = null;
  extractBackgroundMock.mockReset();
});

describe("BackgroundPanel compare label", () => {
  it("names the compare base after the input it was handed, so a stacked base reads as the stack", async () => {
    const html = await runAndCompare(fileOf("bg-stack-label"), { inputLabel: STACK_LABEL });
    expect(compareLabels(html)).toEqual({ caption: `${STACK_LABEL} / Background Removed`, left: STACK_LABEL });
  });

  it("keeps Original when no input label is given", async () => {
    const html = await runAndCompare(fileOf("bg-original-label"), {});
    expect(compareLabels(html)).toEqual({ caption: "Original / Background Removed", left: "Original" });
  });
});
