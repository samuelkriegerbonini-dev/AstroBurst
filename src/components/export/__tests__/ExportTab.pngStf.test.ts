import { describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";

const MONO = "C:\\data\\673nmos.fits";
const RGB = "C:\\data\\m16_rgb.fits";
const AUTO = { shadow: 0.0043, midtone: 0.0085, highlight: 1 };
const MANUAL = { shadow: 0.0043, midtone: 0.0121, highlight: 1 };
const ASINH = { stretch: "asinh", limits: "zscale", symmetric: false, colormap: "viridis", invert: true };

interface Processed {
  fitsPath: string | null;
  planes?: number;
}

const screen = vi.hoisted(() => ({
  file: { path: "", isRgb: false },
  processed: null as Processed | null,
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({
    file: { id: "f", path: screen.file.path, name: "f.fits", status: "done", result: { is_rgb: screen.file.isRgb, previewUrl: "asset://localhost/f.png" } },
  }),
  useHistContext: () => ({ stfParams: MANUAL, histData: { auto_stf: AUTO } }),
  useDisplayContext: () => ({ display: ASINH }),
  useRgbContext: () => ({ rgbChannels: null }),
  useCubeContext: () => ({ isCube: false, cubeDims: null }),
  useRenderContext: () => ({ processed: screen.processed }),
  useDisplayedImage: () => ({
    path: screen.processed?.fitsPath ?? screen.file.path,
    dimensions: [1600, 1600],
    isProcessed: screen.processed !== null,
    label: screen.processed ? "Curves" : null,
    previewOnly: screen.processed !== null && screen.processed.fitsPath === null,
    previewUrl: null,
  }),
}));
vi.mock("../../../context/CompositeContext", () => ({
  useCompositePreview: () => ({ compositePreviewUrl: null }),
  useCompositeStf: () => ({ compositeStfR: AUTO, compositeStfG: AUTO, compositeStfB: AUTO, compositeStfLinked: true }),
}));

import { hooks } from "../../../test/hookRuntime";
import ExportTab from "../ExportTab";

type AnyProps = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...elements(node.props.children)];
}

function panelProps(file: { path: string; isRgb: boolean }, processed: Processed | null): AnyProps {
  Object.assign(screen, { file, processed });
  hooks.mount();
  const inner = (ExportTab as unknown as { type: (props: object) => ReactNode }).type;
  const panel = elements(hooks.render(inner, {}) as ReactNode).find((el) => "rgbFilePath" in el.props);
  if (!panel) throw new Error("ExportTab rendered no ExportPanel");
  return panel.props;
}

describe("ExportTab feeds the PNG note (EX-01)", () => {
  it("passes the viewer's display and the histogram's auto STF next to the Image tool's STF", () => {
    const props = panelProps({ path: MONO, isRgb: false }, null);
    expect(props.stfParams).toBe(MANUAL);
    expect(props.autoStf).toBe(AUTO);
    expect(props.display).toBe(ASINH);
  });

  it("says whether Export as PNG reads an RGB image, from the processed result's planes or the file", () => {
    expect(panelProps({ path: MONO, isRgb: false }, null).pngSourceRgb).toBe(false);
    expect(panelProps({ path: RGB, isRgb: true }, null).pngSourceRgb).toBe(true);
    expect(panelProps({ path: RGB, isRgb: true }, { fitsPath: "C:\\history\\n0001.fits", planes: 3 }).pngSourceRgb).toBe(true);
    expect(panelProps({ path: RGB, isRgb: true }, { fitsPath: "C:\\history\\n0002.fits", planes: 1 }).pngSourceRgb).toBe(false);
    expect(panelProps({ path: RGB, isRgb: true }, { fitsPath: null }).pngSourceRgb, "a PNG-only result exports the file behind it").toBe(true);
    expect(panelProps({ path: MONO, isRgb: false }, { fitsPath: "C:\\history\\n0003.fits" }).pngSourceRgb).toBe(false);
  });
});
