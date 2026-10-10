import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { hostProps, applyToneFileMock, captureHost } = vi.hoisted(() => {
  const hostProps = new Map<string, Record<string, unknown>>();
  type Factory = (type: unknown, props: Record<string, unknown> | null, ...rest: unknown[]) => unknown;
  const captureHost = (factory: Factory): Factory => (type, props, ...rest) => {
    const id = props?.["data-testid"];
    if (typeof type === "string" && typeof id === "string") hostProps.set(id, props as Record<string, unknown>);
    return factory(type, props, ...rest);
  };
  return { hostProps, applyToneFileMock: vi.fn(), captureHost };
});

vi.mock("react/jsx-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-runtime")>();
  return { ...actual, jsx: captureHost(actual.jsx as never), jsxs: captureHost(actual.jsxs as never) };
});

vi.mock("react/jsx-dev-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-dev-runtime")>();
  return { ...actual, jsxDEV: captureHost(actual.jsxDEV as never) };
});

vi.mock("../../../services/toneFile", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/toneFile")>()),
  applyToneFile: applyToneFileMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { withStep } from "../../../utils/processingChain";
import CurvesPanel, { CurvesPanelView, type CurvesPanelProps } from "../CurvesPanel";
import { RGB_FITS_NOTICE } from "../rgbFitsNotice";
import { IDENTITY_CURVE, IDENTITY_LEVELS, TONE_IDENTITY_MESSAGE, type ToneFileResult } from "../../../services/toneFile";
import type { CurvePoint, LevelsParams } from "../../../services/tone";
import type { ProcessedFile } from "../../../shared/types";

const PLANE_SENTENCE = "Levels and the curve apply to all three planes.";
const COMPOSITE_NOTICE = "Curves on the composite live in Compose › Adjust.";
const MOVED_CURVE: CurvePoint[] = [{ x: 0, y: 0 }, { x: 0.5, y: 0.7 }, { x: 1, y: 1 }];

function fileOf(id: string, isRgb: boolean): ProcessedFile {
  const path = isRgb ? `C:/data/${id}_rgb.fits` : `C:/data/${id}.fits`;
  return {
    id,
    name: path.split("/").pop() ?? path,
    path,
    sourcePath: path,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/x.png", previewUrl: "asset://C:/out/x.png", dimensions: [64, 64], elapsed_ms: 1, is_rgb: isRgb },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function toneResult(fitsPath: string, isRgb: boolean): ToneFileResult {
  return {
    png_path: fitsPath.replace(/\.fits$/, ".png"),
    fits_path: fitsPath,
    previewUrl: "asset://out/tone.png",
    dimensions: [64, 64],
    is_rgb: isRgb,
    levels_applied: false,
    curves_applied: true,
    input_normalized: null,
    elapsed_ms: 3,
  };
}

function baseProps(file: ProcessedFile, overrides: Partial<CurvesPanelProps> = {}): CurvesPanelProps {
  return {
    selectedFile: file,
    outputDir: "C:/out",
    fileKey: fileKeyOf(file),
    compositeMode: false,
    fileName: file.name,
    disabledReason: null,
    disabledReasonId: "rgb-note",
    ...overrides,
  };
}

function wrap(file: ProcessedFile, panel: ReturnType<typeof createElement>): string {
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: panel });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function renderStateful(file: ProcessedFile, overrides: Partial<CurvesPanelProps> = {}): string {
  return wrap(file, createElement(CurvesPanel, baseProps(file, overrides)));
}

function renderView(file: ProcessedFile, tone: { levels?: LevelsParams; points?: CurvePoint[] }, overrides: Partial<CurvesPanelProps> = {}): string {
  return wrap(
    file,
    createElement(CurvesPanelView, {
      ...baseProps(file, overrides),
      levels: tone.levels ?? IDENTITY_LEVELS,
      points: tone.points ?? IDENTITY_CURVE.map((p) => ({ ...p })),
      onLevelsChange: () => {},
      onPointsChange: () => {},
    }),
  );
}

function showToneOutput(file: ProcessedFile, fitsPath: string): void {
  const key = fileKeyOf(file) as string;
  function Publisher() {
    const { publishProcessed } = useRenderActions();
    publishProcessed(
      key,
      { fitsPath, previewUrl: "asset://out/tone.png", dimensions: [64, 64], label: "Curves", kind: "processing", inputPath: file.path },
      (c) => withStep(c, "tone", { fitsPath, previewUrl: "asset://out/tone.png", dimensions: [64, 64] }),
    );
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function runButton(): Record<string, unknown> {
  const props = hostProps.get("curves-run");
  if (!props) throw new Error("curves-run not rendered");
  return props;
}

beforeEach(() => {
  hostProps.clear();
  applyToneFileMock.mockReset();
});

describe("CurvesPanel run button", () => {
  it("is disabled with the identity title while the levels and the curve are identity", () => {
    const html = renderStateful(fileOf("identity", false));
    expect(runButton().disabled).toBe(true);
    expect(runButton().title).toBe(TONE_IDENTITY_MESSAGE);
    expect(html).toMatch(/<button[^>]*data-testid="curves-run"[^>]*disabled=""/);
    expect(html).toContain(`title="${TONE_IDENTITY_MESSAGE}"`);
    expect(html).not.toContain(PLANE_SENTENCE);
  });

  it("with a curve point moved it sends a three-point curve and hands fits_path to onProcessingDone", async () => {
    const file = fileOf("moved", false);
    const onProcessingDone = vi.fn();
    applyToneFileMock.mockResolvedValue(toneResult("C:/out/moved_tone.fits", false));
    renderView(file, { points: MOVED_CURVE }, { onProcessingDone });
    expect(runButton().disabled).toBe(false);
    expect(runButton().title).toBeUndefined();
    (runButton().onClick as () => void)();
    await vi.waitFor(() => expect(onProcessingDone).toHaveBeenCalledTimes(1));
    expect(applyToneFileMock).toHaveBeenCalledTimes(1);
    const [path, dir, options] = applyToneFileMock.mock.calls[0];
    expect(path).toBe(file.path);
    expect(dir).toBe("C:/out");
    expect(options.curve).toEqual(MOVED_CURVE);
    expect(onProcessingDone.mock.calls[0][0].fits_path).toBe("C:/out/moved_tone.fits");
  });
});

describe("CurvesPanel on an RGB FITS", () => {
  it("ignores the RGB disabled reason, renders the plane sentence and not the RGB notice", () => {
    const html = renderView(fileOf("rgb", true), { levels: { black: 0.02, gamma: 1.5, white: 0.6 } }, { disabledReason: RGB_FITS_NOTICE });
    expect(html).toContain(PLANE_SENTENCE);
    expect(html).not.toContain(RGB_FITS_NOTICE);
    expect(runButton().disabled).toBe(false);
    expect(runButton()["aria-describedby"]).toBeUndefined();
  });

  it("keeps the identity title on an RGB FITS", () => {
    renderStateful(fileOf("rgb-identity", true), { disabledReason: RGB_FITS_NOTICE });
    expect(runButton().disabled).toBe(true);
    expect(runButton().title).toBe(TONE_IDENTITY_MESSAGE);
  });
});

describe("CurvesPanel in composite mode", () => {
  it("points to Compose › Adjust", () => {
    const html = renderStateful(fileOf("composite", false), { compositeMode: true });
    expect(html).toContain(COMPOSITE_NOTICE);
  });

  it("does not run on the file behind the composite", () => {
    renderView(fileOf("composite-run", false), { points: MOVED_CURVE }, { compositeMode: true });
    expect(runButton().disabled).toBe(true);
  });

  it("does not show the file's Curves result under the composite notice", async () => {
    const file = fileOf("composite-result", false);
    const out = "C:/out/composite-result_tone.fits";
    const onProcessingDone = vi.fn();
    applyToneFileMock.mockResolvedValue(toneResult(out, false));
    renderView(file, { points: MOVED_CURVE }, { onProcessingDone });
    (runButton().onClick as () => void)();
    await vi.waitFor(() => expect(onProcessingDone).toHaveBeenCalledTimes(1));
    showToneOutput(file, out);
    await vi.waitFor(() => expect(renderView(file, { points: MOVED_CURVE })).toContain("Curve applied"));
    const html = renderView(file, { points: MOVED_CURVE }, { compositeMode: true });
    expect(html).toContain(COMPOSITE_NOTICE);
    expect(html).not.toContain("Curve applied");
  });
});
