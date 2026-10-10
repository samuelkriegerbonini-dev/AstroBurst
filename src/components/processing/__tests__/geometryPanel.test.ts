import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { hostProps, withPreviewMock, captureHost } = vi.hoisted(() => {
  const hostProps = new Map<string, Record<string, unknown>>();
  type Factory = (type: unknown, props: Record<string, unknown> | null, ...rest: unknown[]) => unknown;
  const captureHost = (factory: Factory): Factory => (type, props, ...rest) => {
    const id = props?.["data-testid"];
    if (typeof type === "string" && typeof id === "string") hostProps.set(id, props as Record<string, unknown>);
    return factory(type, props, ...rest);
  };
  return { hostProps, withPreviewMock: vi.fn(), captureHost };
});

vi.mock("react/jsx-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-runtime")>();
  return { ...actual, jsx: captureHost(actual.jsx as never), jsxs: captureHost(actual.jsxs as never) };
});

vi.mock("react/jsx-dev-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-dev-runtime")>();
  return { ...actual, jsxDEV: captureHost(actual.jsxDEV as never) };
});

vi.mock("../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri")>()),
  withPreview: withPreviewMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { registerFileIngest } from "../../../hooks/useFileIngest";
import { fileStore } from "../../../hooks/useFileStore";
import { withStep } from "../../../utils/processingChain";
import GeometryPanel from "../GeometryPanel";
import { GEOMETRY_OPS, type GeometryResult } from "../../../services/imageGeometry";
import type { AstroFile, ProcessedFile } from "../../../shared/types";

const RAW = "C:/data/jw02739_f444w_i2d.fits";
const BG = "C:/out/jw02739_f444w_i2d_bg.fits";
const DEBAYER_PNG = "asset://C:/out/jw02739_f444w_i2d_debayer.png";

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "jw02739_f444w_i2d.fits",
    path: RAW,
    sourcePath: RAW,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/f444w.png", previewUrl: "asset://C:/out/f444w.png", dimensions: [7065, 4177], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function geometryResult(fitsPath: string): GeometryResult {
  return {
    png_path: fitsPath.replace(/\.fits$/, ".png"),
    fits_path: fitsPath,
    previewUrl: "asset://C:/out/rot90.png",
    dimensions: [4177, 7065],
    original_dimensions: [7065, 4177],
    op: "rot90",
    is_rgb: false,
    wcs_updated: true,
    elapsed_ms: 40,
  };
}

function showBackgroundOutput(file: ProcessedFile): void {
  const key = fileKeyOf(file) as string;
  function Publisher() {
    const { publishProcessed } = useRenderActions();
    publishProcessed(
      key,
      { fitsPath: BG, previewUrl: "asset://C:/out/bg.png", dimensions: [7065, 4177], label: "Background", kind: "processing", inputPath: RAW },
      (c) => withStep(c, "background", { fitsPath: BG, previewUrl: "asset://C:/out/bg.png", dimensions: [7065, 4177] }),
    );
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function showDebayerPreview(file: ProcessedFile): void {
  const key = fileKeyOf(file) as string;
  function Publisher() {
    const { publishProcessed } = useRenderActions();
    publishProcessed(key, { fitsPath: null, previewUrl: DEBAYER_PNG, dimensions: null, label: "Debayer", kind: "debayer", inputPath: RAW });
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function inputLabel(html: string): string | null {
  return /Input: <span[^>]*>([^<]*)<\/span>/.exec(html)?.[1] ?? null;
}

function beforeSrc(html: string): string | null {
  return /<img src="([^"]*)" alt="Before"/.exec(html)?.[1] ?? null;
}

function render(file: ProcessedFile, compositeMode = false): string {
  const panel = createElement(GeometryPanel, {
    selectedFile: file,
    outputDir: "C:/out",
    fileKey: fileKeyOf(file),
    compositeMode,
    fileName: file.name,
  });
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: panel });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function click(testId: string): void {
  const props = hostProps.get(testId);
  if (!props) throw new Error(`${testId} not rendered`);
  (props.onClick as () => void)();
}

describe("GeometryPanel", () => {
  const ingest = vi.fn((files: AstroFile[]) => fileStore.addFiles(files));
  let unregister: () => void;

  beforeEach(() => {
    fileStore.reset();
    hostProps.clear();
    withPreviewMock.mockReset();
    ingest.mockClear();
    unregister = registerFileIngest(ingest);
  });

  afterEach(() => unregister());

  it("renders one button per operation", () => {
    const html = render(fileOf("geom-buttons"));
    for (const op of GEOMETRY_OPS) expect(html).toContain(`data-testid="geometry-op-${op}"`);
    expect(html).not.toContain('data-testid="geometry-result"');
  });

  it("clicking rot90 transforms the displayed image and the result line names the written file", async () => {
    const file = fileOf("geom-displayed");
    showBackgroundOutput(file);
    withPreviewMock.mockResolvedValue(geometryResult("C:/out/jw02739_f444w_i2d_bg_rot90.fits"));
    render(file);
    click("geometry-op-rot90");
    await vi.waitFor(() => expect(ingest).toHaveBeenCalledTimes(1));
    expect(withPreviewMock).toHaveBeenCalledWith("transform_geometry_cmd", "C:/out", { path: BG, op: "rot90" });
    expect(ingest).toHaveBeenCalledWith([{ name: "jw02739_f444w_i2d_bg_rot90.fits", path: "C:/out/jw02739_f444w_i2d_bg_rot90.fits", size: 0 }], { quiet: true });
    const html = render(file);
    expect(html).toMatch(/data-testid="geometry-result"[^>]*>Written jw02739_f444w_i2d_bg_rot90\.fits \(4177x7065\); select it in Files to continue\.</);
  });

  it("uses the loaded file when nothing processed is displayed", async () => {
    const file = fileOf("geom-original");
    withPreviewMock.mockResolvedValue({ ...geometryResult("C:/out/jw02739_f444w_i2d_flip_h.fits"), op: "flip_h", dimensions: [7065, 4177] });
    render(file);
    click("geometry-op-flip_h");
    await vi.waitFor(() => expect(withPreviewMock).toHaveBeenCalledTimes(1));
    expect(withPreviewMock).toHaveBeenCalledWith("transform_geometry_cmd", "C:/out", { path: RAW, op: "flip_h" });
    await vi.waitFor(() => expect(render(file)).toContain("Written jw02739_f444w_i2d_flip_h.fits (7065x4177); select it in Files to continue."));
  });

  it("after a Debayer preview, names the loaded file as its input and compares with the file preview", async () => {
    const file = fileOf("geom-debayer-preview");
    showDebayerPreview(file);
    withPreviewMock.mockResolvedValue(geometryResult("C:/out/jw02739_f444w_i2d_rot90.fits"));
    expect(inputLabel(render(file))).toBe("jw02739_f444w_i2d.fits");
    click("geometry-op-rot90");
    await vi.waitFor(() => expect(ingest).toHaveBeenCalledTimes(1));
    expect(withPreviewMock).toHaveBeenCalledWith("transform_geometry_cmd", "C:/out", { path: RAW, op: "rot90" });
    await vi.waitFor(() => expect(render(file)).toContain("Written jw02739_f444w_i2d_rot90.fits"));
    const html = render(file);
    expect(beforeSrc(html)).toBe("asset://C:/out/f444w.png");
    expect(html).not.toContain(DEBAYER_PNG);
  });

  it("says that it acts on the file in composite mode", () => {
    expect(render(fileOf("geom-composite"), true)).toContain("the composite on screen does not change");
  });
});
