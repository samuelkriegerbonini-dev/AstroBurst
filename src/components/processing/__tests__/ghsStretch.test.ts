import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

interface SliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  scale?: "linear" | "log";
  format?: (v: number) => string;
}

interface RunButtonProps {
  label: string;
  disabled?: boolean;
  onClick: () => void;
}

const { sliders, runButtons, recordGhsStretchMock, recordArcsinhStretchMock } = vi.hoisted(() => ({
  sliders: [] as SliderProps[],
  runButtons: [] as RunButtonProps[],
  recordGhsStretchMock: vi.fn(),
  recordArcsinhStretchMock: vi.fn(),
}));

vi.mock("../../ui", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../ui")>()),
  Slider: (props: SliderProps) => {
    sliders.push(props);
    return null;
  },
  RunButton: (props: RunButtonProps) => {
    runButtons.push(props);
    return null;
  },
}));

vi.mock("../../../services/processing", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/processing")>()),
  recordGhsStretch: recordGhsStretchMock,
  recordArcsinhStretch: recordArcsinhStretchMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf } from "../../../context/PreviewContext";
import { historyRerunStore, sectionForOp } from "../../../utils/historyRerunStore";
import { GHS_DEFAULTS, ghsOp, ghsOptionsOf, stretchOptionsOf, type GhsSettings } from "../../../services/processing";
import ArcsinhStretchPanel, { GHS_COMPOSITE_NOTICE } from "../ArcsinhStretchPanel";
import { withHighlightProtect, withShadowProtect, withSymmetryPoint } from "../GhsStretchControls";
import type { OpSpec } from "../../../shared/types/ops";
import type { ProcessedFile } from "../../../shared/types";

const PATH = "C:/data/673nmos.fits";
const SCOPE = { imageKey: PATH, basePath: null };
const SETTINGS: GhsSettings = { stretchFactor: 3, localIntensity: 1.5, symmetryPoint: 0.02, shadowProtect: 0.005, highlightProtect: 0.9 };

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "673nmos.fits",
    path: PATH,
    sourcePath: PATH,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/a.png", previewUrl: "asset://a.png", dimensions: [1600, 1600], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

interface RenderOptions {
  rerun?: OpSpec;
  compositeMode?: boolean;
  onProcessingDone?: (result: unknown) => void;
}

function render(id: string, options: RenderOptions = {}): string {
  sliders.length = 0;
  runButtons.length = 0;
  if (options.rerun) historyRerunStore.request({ section: "stretch", op: options.rerun, node: "n0003", downstream: false, scope: SCOPE });
  const file = fileOf(id);
  const props = {
    selectedFile: file,
    outputDir: "C:/out",
    fileKey: fileKeyOf(file),
    compositeMode: options.compositeMode ?? false,
    compositeInput: null,
    onCompositeDone: () => {},
    fileName: file.name,
    onProcessingDone: options.onProcessingDone,
  };
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: createElement(ArcsinhStretchPanel, props) });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function methodButton(html: string, method: "arcsinh" | "ghs"): string {
  const tag = new RegExp(`<button[^>]*data-stretch-method="${method}"[^>]*>`).exec(html)?.[0];
  if (!tag) throw new Error(`no ${method} method button`);
  return tag;
}

function slider(label: string): SliderProps {
  const found = sliders.find((s) => s.label === label);
  if (!found) throw new Error(`no slider ${label}: ${sliders.map((s) => s.label).join(", ")}`);
  return found;
}

function lastRunButton(): RunButtonProps {
  const found = runButtons.at(-1);
  if (!found) throw new Error("no run button rendered");
  return found;
}

beforeEach(() => {
  historyRerunStore.clear();
  recordGhsStretchMock.mockReset();
  recordArcsinhStretchMock.mockReset();
});

describe("GHS steps in the file history", () => {
  it("map to the Stretch section, so Re-run with can open them", () => {
    expect(sectionForOp(ghsOp({ stretchFactor: 3 }))).toBe("stretch");
  });

  it("decode back into the five settings the panel shows", () => {
    expect(ghsOptionsOf(ghsOp(SETTINGS))).toEqual(SETTINGS);
    expect(stretchOptionsOf(ghsOp(SETTINGS))).toEqual({ method: "ghs", ghs: SETTINGS });
    expect(stretchOptionsOf({ op: "arcsinh", v: 1, factor: 120 })).toEqual({ method: "arcsinh", factor: 120 });
    expect(stretchOptionsOf({ op: "tone", v: 1 })).toBeNull();
  });

  it("start from the Compose › Stretch defaults, symmetry point 0.01", () => {
    expect(GHS_DEFAULTS).toEqual({ stretchFactor: 2, localIntensity: 0, symmetryPoint: 0.01, shadowProtect: 0, highlightProtect: 1 });
    expect(ghsOp({ stretchFactor: 3 })).toEqual({ op: "ghs", v: 1, d: 3, b: 0, sp: 0.01, lp: 0, hp: 1 });
  });
});

describe("Stretch panel on a file", () => {
  it("opens on Arcsinh with GHS one click away", () => {
    const html = render("default");
    expect(html).toMatch(/role="group" aria-label="Stretch method"/);
    expect(methodButton(html, "arcsinh")).toContain('aria-pressed="true"');
    expect(methodButton(html, "ghs")).toContain('aria-pressed="false"');
    expect(methodButton(html, "ghs")).not.toContain('disabled=""');
    expect(sliders.map((s) => s.label)).toEqual(["Stretch Factor (S)"]);
    expect(lastRunButton().label).toBe("Apply Stretch (S=50)");
  });

  it("Re-run with on a GHS step opens GHS pre-filled", () => {
    const html = render("rerun", { rerun: ghsOp(SETTINGS) });
    expect(methodButton(html, "ghs")).toContain('aria-pressed="true"');
    expect(methodButton(html, "arcsinh")).toContain('aria-pressed="false"');
    expect(html).toContain("GHS Stretch");
    expect(slider("Stretch (D)").value).toBe(3);
    expect(slider("Local Intensity (b)").value).toBe(1.5);
    expect(slider("Symmetry Point (SP)").value).toBe(0.02);
    expect(slider("Shadow Protect (LP)").value).toBe(0.005);
    expect(slider("Highlight Protect (HP)").value).toBe(0.9);
    expect(lastRunButton().label).toBe("Apply GHS Stretch (D=3.00)");
  });

  it("shows the five GHS sliders at the Compose › Stretch ranges", () => {
    render("ranges", { rerun: ghsOp(GHS_DEFAULTS) });
    expect(sliders.map(({ label, min, max, step, scale }) => ({ label, min, max, step, scale }))).toEqual([
      { label: "Stretch (D)", min: 0, max: 50, step: 0.05, scale: undefined },
      { label: "Local Intensity (b)", min: -5, max: 15, step: 0.1, scale: undefined },
      { label: "Symmetry Point (SP)", min: 0, max: 1, step: 0.001, scale: undefined },
      { label: "Shadow Protect (LP)", min: 0, max: 1, step: 0.001, scale: undefined },
      { label: "Highlight Protect (HP)", min: 0, max: 1, step: 0.001, scale: undefined },
    ]);
    expect(sliders.map((s) => s.format?.(s.value))).toEqual(["2.00", "0.0", "0.010", "0.000", "1.000"]);
  });

  it("Apply records a GHS step on the file with the five settings", async () => {
    const onProcessingDone = vi.fn();
    recordGhsStretchMock.mockResolvedValue({ node: "n0004", fits_path: "C:/app/history/n0004.fits", png_path: "C:/app/history/n0004.png", dimensions: [1600, 1600], elapsed_ms: 40, stretch_factor: 3 });
    render("apply", { rerun: ghsOp(SETTINGS), onProcessingDone });
    expect(lastRunButton().disabled).toBe(false);
    lastRunButton().onClick();
    await vi.waitFor(() => expect(onProcessingDone).toHaveBeenCalledTimes(1));
    expect(recordGhsStretchMock).toHaveBeenCalledWith(SCOPE, PATH, SETTINGS, null);
    expect(recordArcsinhStretchMock).not.toHaveBeenCalled();
    expect(onProcessingDone.mock.calls[0][0]).toMatchObject({ node: "n0004" });
  });

  it("Apply on Arcsinh still records an arcsinh step with the factor", async () => {
    const onProcessingDone = vi.fn();
    recordArcsinhStretchMock.mockResolvedValue({ node: "n0005", fits_path: "C:/app/history/n0005.fits", png_path: "C:/app/history/n0005.png", dimensions: [1600, 1600], elapsed_ms: 30, stretch_factor: 50 });
    render("apply-arcsinh", { onProcessingDone });
    lastRunButton().onClick();
    await vi.waitFor(() => expect(onProcessingDone).toHaveBeenCalledTimes(1));
    expect(recordArcsinhStretchMock).toHaveBeenCalledWith(SCOPE, PATH, 50, null);
    expect(recordGhsStretchMock).not.toHaveBeenCalled();
  });
});

describe("GHS protect points", () => {
  it("stay on either side of the symmetry point", () => {
    const wide: GhsSettings = { ...GHS_DEFAULTS, symmetryPoint: 0.1, shadowProtect: 0.02, highlightProtect: 0.5 };
    expect(withSymmetryPoint(wide, 0.6)).toEqual({ ...wide, symmetryPoint: 0.6, highlightProtect: 0.6 });
    expect(withSymmetryPoint(wide, 0.01)).toEqual({ ...wide, symmetryPoint: 0.01, shadowProtect: 0.01 });
    expect(withShadowProtect(wide, 0.3)).toEqual({ ...wide, shadowProtect: 0.1 });
    expect(withShadowProtect(wide, 0.05)).toEqual({ ...wide, shadowProtect: 0.05 });
    expect(withHighlightProtect(wide, 0.05)).toEqual({ ...wide, highlightProtect: 0.1 });
    expect(withHighlightProtect(wide, 0.8)).toEqual({ ...wide, highlightProtect: 0.8 });
  });
});

describe("Stretch panel in composite mode", () => {
  it("points GHS to Compose › Stretch and keeps Arcsinh for the composite", () => {
    const html = render("composite", { compositeMode: true });
    expect(GHS_COMPOSITE_NOTICE).toBe("GHS on the composite lives in Compose › Stretch.");
    expect(html).toContain(GHS_COMPOSITE_NOTICE);
    expect(methodButton(html, "ghs")).toContain('disabled=""');
    expect(methodButton(html, "arcsinh")).toContain('aria-pressed="true"');
    expect(sliders.map((s) => s.label)).toEqual(["Stretch Factor (S)"]);
  });

  it("a GHS Re-run pending while the composite is shown leaves the composite on Arcsinh", () => {
    const html = render("composite-rerun", { compositeMode: true, rerun: ghsOp(SETTINGS) });
    expect(methodButton(html, "arcsinh")).toContain('aria-pressed="true"');
    expect(sliders.map((s) => s.label)).toEqual(["Stretch Factor (S)"]);
    expect(lastRunButton().label).toBe("Apply Stretch (S=50)");
  });

  it("does not show the composite pointer on a file", () => {
    expect(render("file-pointer")).not.toContain(GHS_COMPOSITE_NOTICE);
  });
});
