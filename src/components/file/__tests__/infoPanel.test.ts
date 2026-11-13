import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { HistogramData, ProcessResult } from "../../../shared/types";
import type { ProcessedResult } from "../../../shared/types/preview";
import { SIGMA_MAD_LABEL } from "../../../utils/analysisLabels";
import { CUBE_FRAME_VALUE_PENDING } from "../../../utils/previewShell";

const { current, wcsReadout, pixelReadout } = vi.hoisted(() => ({
  current: {
    file: null as { name: string; path?: string; result: Partial<ProcessResult> | null } | null,
    histData: null as Partial<HistogramData> | null,
    histDataPath: null as string | null,
    processed: null as ProcessedResult | null,
    mousePixel: null as { x: number; y: number } | null,
    target: { path: null as string | null, dimensions: null as [number, number] | null, displayed: { previewOnly: false } },
  },
  wcsReadout: vi.fn<(props: Record<string, unknown>) => null>(() => null),
  pixelReadout: vi.fn<(props: Record<string, unknown>) => null>(() => null),
}));

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: current.file }),
  useHistContext: () => ({ histData: current.histData, histDataPath: current.histDataPath, stfParams: { shadow: 0, midtone: 0.5, highlight: 1 } }),
  useRenderContext: () => ({ processed: current.processed }),
}));
vi.mock("../../../hooks/useMousePixelStore", () => ({ useMousePixel: () => current.mousePixel }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({
  useAnalysisTarget: () => current.target,
  useMeasurementSource: () => null,
}));
vi.mock("../../header/WcsReadout", () => ({ default: wcsReadout }));
vi.mock("../../header/PixelReadout", () => ({ default: pixelReadout }));
vi.mock("../../analysis/MeasurementBadge", () => ({ default: () => null }));

import { InfoPanel } from "../SidebarPanels";

const HOVER_NOTE = "Hover the image for pixel values.";

function render(): string {
  return renderToStaticMarkup(createElement(InfoPanel));
}

function renderWithHeader(header: Record<string, string>): string {
  current.file = { name: "x.fits", result: { header } };
  return render();
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function metric(html: string, label: string): string | null {
  return new RegExp(`<dt(?: [^>]*)?>${escapeRegExp(label)}</dt><dd>([^<]*)</dd>`).exec(html)?.[1] ?? null;
}

beforeEach(() => {
  current.file = null;
  current.histData = null;
  current.histDataPath = null;
  current.processed = null;
  current.mousePixel = null;
  current.target = { path: null, dimensions: null, displayed: { previewOnly: false } };
  vi.clearAllMocks();
});

describe("InfoPanel", () => {
  it("keeps the raw filter and pupil distinct with readable labels", () => {
    const html = renderWithHeader({ TELESCOP: "JWST", INSTRUME: "NIRCAM", FILTER: "F444W", PUPIL: "F470N" });
    expect(html).toMatch(/>Filter<\/dt><dd[^>]*>F444W<\/dd>/);
    expect(html).toMatch(/>Pupil<\/dt><dd[^>]*>F470N<\/dd>/);
    expect(html).toContain("JWST");
    expect(html).toContain("NIRCAM");
  });

  it("keeps an empty pupil slot as recorded in the header", () => {
    const html = renderWithHeader({ INSTRUME: "NIRCAM", FILTER: "F444W", PUPIL: "CLEAR" });
    expect(html).toMatch(/>Pupil<\/dt><dd[^>]*>CLEAR<\/dd>/);
  });

  it("omits missing metadata instead of rendering empty rows", () => {
    const html = renderWithHeader({ INSTRUME: "MIRI", FILTER: "F770W" });
    expect(html).toContain("F770W");
    expect(html).not.toContain(">Pupil</dt>");
    expect(html).not.toContain(">Exposure</dt>");
  });

  it("identifies the file and keeps zero duration, dimensions and data units", () => {
    current.file = { name: "a-long-observation.fits", result: { dimensions: [2048, 1024], elapsed_ms: 0, header: { BITPIX: "-32", BUNIT: "MJy/sr", EXPTIME: "0" } } };
    const html = render();
    expect(html).toContain("File info");
    expect(html).toContain("a-long-observation.fits");
    expect(html).toContain("2048 × 1024");
    expect(html).toContain("MJy/sr");
    expect(html).toContain("0.00 s");
    expect(html).toMatch(/>Exposure<\/dt><dd[^>]*>0 s<\/dd>/);
  });

  it("binds each statistic and display value to its own label, a zero mean included, and keeps the DQ exclusion count", () => {
    current.file = { name: "x.fits", result: {} };
    current.histData = { mean: 0, median: 2.5, sigma: 3.25, masked: true, dq_excluded: 12 };
    const html = render();
    expect(metric(html, "Mean")).toBe("0");
    expect(metric(html, "Median")).toBe("2.5");
    expect(metric(html, SIGMA_MAD_LABEL)).toBe("3.25");
    expect(metric(html, "Shadows")).toBe("0.0000");
    expect(metric(html, "Midtones")).toBe("0.5000");
    expect(metric(html, "Highlights")).toBe("1.0000");
    expect(html).toMatch(/<p class="ab-info-note"[^>]*>DQ-masked · 12 excluded<\/p>/);
  });

  it("shows a dash for a missing or non-finite statistic", () => {
    current.file = { name: "x.fits", result: {} };
    current.histData = { mean: Number.NaN, median: undefined, sigma: Number.POSITIVE_INFINITY };
    const html = render();
    expect(metric(html, "Mean")).toBe("—");
    expect(metric(html, "Median")).toBe("—");
    expect(metric(html, SIGMA_MAD_LABEL)).toBe("—");
  });

  it("keeps four significant digits and the data unit for the small statistics of a rate image", () => {
    const rate = "/data/jw01266005001_02103_00001_nrs1_rate.fits";
    current.file = { name: "jw01266005001_02103_00001_nrs1_rate.fits", path: rate, result: { header: { BUNIT: "DN/s" } } };
    current.histData = { mean: -0.124586, median: -0.003707, sigma: 0.063128 };
    current.histDataPath = rate;
    const html = render();
    expect(metric(html, "Mean")).toBe("-0.1246 DN/s");
    expect(metric(html, "Median")).toBe("-0.003707 DN/s");
    expect(metric(html, SIGMA_MAD_LABEL)).toBe("0.06313 DN/s");
  });

  it("writes statistics below 1e-3 with an exponent and large ones rounded to whole units", () => {
    const coadd = "/data/frame-r-008162-6-0080.fits";
    current.file = { name: "frame-r-008162-6-0080.fits", path: coadd, result: { header: { BUNIT: "'nanomaggy'" } } };
    current.histData = { mean: 0.00042137, median: -0.0000123456, sigma: 12345.678 };
    current.histDataPath = coadd;
    const html = render();
    expect(metric(html, "Mean")).toBe("4.214e-4 nanomaggy");
    expect(metric(html, "Median")).toBe("-1.235e-5 nanomaggy");
    expect(metric(html, SIGMA_MAD_LABEL)).toBe("12346 nanomaggy");
  });

  it("does not give the file's unit to the statistics of a product on screen", () => {
    current.file = { name: "cube_s3d.fits", path: "/data/cube_s3d.fits", result: { header: { BUNIT: "MJy/sr" } } };
    current.histData = { mean: 152.25, median: 3.5, sigma: 0.25 };
    current.histDataPath = "/out/moment1.fits";
    const html = render();
    expect(metric(html, "Mean")).toBe("152.3");
    expect(metric(html, "Median")).toBe("3.5");
    expect(metric(html, SIGMA_MAD_LABEL)).toBe("0.25");
  });

  it("does not probe the cube file while a PNG-only cube frame is shown and says when its value comes", () => {
    const cube = "/data/jw01366_nirspec_s3d.fits";
    current.file = { name: "jw01366_nirspec_s3d.fits", path: cube, result: { dimensions: [64, 64] } };
    current.target = { path: cube, dimensions: [64, 64], displayed: { previewOnly: true } };
    current.processed = { kind: "cube", frameIndex: 37, fitsPath: null, previewUrl: "asset://out/cube_frame_abc_37.png", dimensions: null, label: "ch 37 (0–39)", inputPath: cube };
    current.mousePixel = { x: 3, y: 4 };
    const html = render();
    expect(pixelReadout.mock.calls.map(([props]) => props.filePath)).not.toContain(cube);
    expect(html).toContain(`<p class="ab-info-note">${CUBE_FRAME_VALUE_PENDING}</p>`);
    expect(html).not.toContain("Hover the image for pixel values.");
  });

  it("probes the frame's own FITS once it is published", () => {
    const cube = "/data/jw01366_nirspec_s3d.fits";
    const frame = "/out/cube_frame_abc_37.fits";
    current.file = { name: "jw01366_nirspec_s3d.fits", path: cube, result: { dimensions: [64, 64] } };
    current.target = { path: frame, dimensions: [64, 64], displayed: { previewOnly: false } };
    current.processed = { kind: "cube", frameIndex: 37, fitsPath: frame, previewUrl: "asset://out/cube_frame_abc_37.png", dimensions: [64, 64], label: "ch 37 (0–39)", inputPath: cube };
    current.mousePixel = { x: 3, y: 4 };
    const html = render();
    expect(pixelReadout.mock.calls.map(([props]) => props.filePath)).toEqual([frame]);
    expect(html).not.toContain(CUBE_FRAME_VALUE_PENDING);
  });

  it("passes the analysis path, dimensions and zero-based cursor coordinates to the readouts", () => {
    current.file = { name: "x.fits", result: { dimensions: [400, 300] } };
    current.target = { path: "processed.fits", dimensions: [200, 150], displayed: { previewOnly: false } };
    current.mousePixel = { x: 0, y: 12 };
    render();
    expect(wcsReadout).toHaveBeenCalledTimes(1);
    expect(wcsReadout.mock.calls[0][0]).toEqual({ filePath: "processed.fits", imageWidth: 200, imageHeight: 150, mouseX: 0, mouseY: 12 });
    expect(pixelReadout).toHaveBeenCalledTimes(1);
    expect(pixelReadout.mock.calls[0][0]).toEqual({ filePath: "processed.fits", mouseX: 0, mouseY: 12 });
  });

  it("shows a useful empty state without scientific values or readout calls", () => {
    const html = render();
    expect(html).toContain("No file selected");
    expect(html).toContain("Select a file to inspect its details.");
    expect(wcsReadout).not.toHaveBeenCalled();
    expect(pixelReadout).not.toHaveBeenCalled();
  });

  it("keeps a single unlabelled section and names each group with an h3, File details included", () => {
    current.file = { name: "x.fits", result: { dimensions: [400, 300], header: { OBJECT: "M16" } } };
    current.histData = { mean: 1, median: 1, sigma: 1 };
    current.target = { path: "x.fits", dimensions: [400, 300], displayed: { previewOnly: false } };
    const html = render();
    expect(html.match(/<section\b/g)).toHaveLength(1);
    expect(html).not.toContain("aria-label=");
    expect(html).toMatch(/<h3[^>]*>File details<\/h3>/);
    for (const heading of ["File details", "Observation", "Image statistics", "Display transfer", "Live readout"]) {
      expect(html, heading).toMatch(new RegExp(`<div class="ab-info-section[^"]*"><h3[^>]*>${heading}`));
    }
    current.file = null;
    const empty = render();
    expect(empty.match(/<section\b/g)).toHaveLength(1);
    expect(empty).not.toContain("aria-label=");
  });

  it("shows the hover hint only while the cursor is off the image", () => {
    current.file = { name: "x.fits", result: { dimensions: [400, 300] } };
    current.target = { path: "x.fits", dimensions: [400, 300], displayed: { previewOnly: false } };
    expect(render()).toContain(HOVER_NOTE);
    current.mousePixel = { x: 12, y: 40 };
    const hovering = render();
    expect(hovering).toContain("Live readout");
    expect(hovering).not.toContain(HOVER_NOTE);
  });

  it("notes that details are pending while the result is missing, with no empty groups", () => {
    current.file = { name: "x.fits", result: null };
    const html = render();
    expect(html).toContain("No image details available yet.");
    expect(html).not.toContain("File details");
    expect(html).not.toContain("ab-info-section");
  });

  it("omits the live readout without a probe path and calls neither readout", () => {
    current.file = { name: "x.fits", result: { dimensions: [400, 300] } };
    const html = render();
    expect(html).toContain("File details");
    expect(html).not.toContain("Live readout");
    expect(html).not.toContain(HOVER_NOTE);
    expect(wcsReadout).not.toHaveBeenCalled();
    expect(pixelReadout).not.toHaveBeenCalled();
  });
});

describe("InfoPanel sky readout follows the analysis target", () => {
  const CUBE_PATH = "/data/cube_s3d.fits";
  const CUBE_GRID: [number, number] = [53, 55];

  beforeEach(() => {
    current.file = { name: "cube_s3d.fits", result: { dimensions: CUBE_GRID, header: {}, elapsed_ms: 10 } };
    current.mousePixel = { x: 20, y: 30 };
  });

  function wcsProps(): Record<string, unknown>[] {
    render();
    return wcsReadout.mock.calls.map(([props]) => props);
  }

  it("probes the PV diagram on screen with its own grid, not the cube it was cut from", () => {
    current.target = { path: "/out/pv.fits", dimensions: [40, 3814], displayed: { previewOnly: false } };
    expect(wcsProps()).toEqual([{ filePath: "/out/pv.fits", imageWidth: 40, imageHeight: 3814, mouseX: 20, mouseY: 30 }]);
  });

  it("probes the loaded cube with the cube grid", () => {
    current.target = { path: CUBE_PATH, dimensions: CUBE_GRID, displayed: { previewOnly: false } };
    expect(wcsProps()).toEqual([{ filePath: CUBE_PATH, imageWidth: 53, imageHeight: 55, mouseX: 20, mouseY: 30 }]);
  });

  it("hides the readout for a PNG-only result on another grid", () => {
    current.target = { path: CUBE_PATH, dimensions: [800, 600], displayed: { previewOnly: true } };
    expect(wcsProps()).toEqual([]);
  });

  it("hides the readout without a measured path", () => {
    current.target = { path: null, dimensions: null, displayed: { previewOnly: false } };
    expect(wcsProps()).toEqual([]);
  });
});
