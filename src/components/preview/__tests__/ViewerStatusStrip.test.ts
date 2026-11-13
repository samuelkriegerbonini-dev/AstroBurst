import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { hooks } from "../../../test/hookRuntime";
import { ZERO_BASED_PIXEL_TITLE } from "../../../utils/regionGeometry";
import { CUBE_FRAME_VALUE_PENDING } from "../../../utils/previewShell";
import { getWcsInfo, pixelToWorld } from "../../../services/astrometry";
import { probePixel } from "../../../services/display";
import type { PixelProbeResult, WavelengthProbe } from "../../../shared/types/analysis";
import type { WcsInfo } from "../../../shared/types/astrometry";
import type { ProcessedResult } from "../../../shared/types/preview";

vi.mock("react", async (importOriginal) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await importOriginal());
});

interface StripView {
  pixel: { x: number; y: number } | null;
  path: string | null;
  filePath: string | null;
  processed: ProcessedResult | null;
}

const view: StripView = { pixel: null, path: "/data/m51_cal.fits", filePath: null, processed: null };

vi.mock("../../../hooks/useMousePixelStore", () => ({ useMousePixel: () => view.pixel }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({
  useAnalysisTarget: () => ({ path: view.path, composite: false, fileRgbView: false }),
}));
vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => {
    const path = view.filePath ?? view.path;
    return { file: path === null ? null : { path, result: { dimensions: [64, 64] } } };
  },
  useRenderContext: () => ({ processed: view.processed, chain: { steps: {}, psfKernel: null, base: null } }),
}));
vi.mock("../../../services/display", () => ({ probePixel: vi.fn() }));
vi.mock("../../../services/astrometry", () => ({ getWcsInfo: vi.fn(), pixelToWorld: vi.fn() }));

import ViewerStatusStrip from "../ViewerStatusStrip";

const MICRONS: WavelengthProbe = { value: 1.5, unit: "um" };

function answerProbes({ wcs, wavelength }: { wcs: boolean; wavelength: WavelengthProbe | null }): void {
  vi.mocked(getWcsInfo).mockImplementation(() => (wcs ? Promise.resolve({} as WcsInfo) : Promise.reject(new Error("no celestial WCS"))));
  vi.mocked(probePixel).mockImplementation((_path, x, y) => Promise.resolve({ x, y, value: 1234, unit: "MJy/sr", wavelength } as PixelProbeResult));
  vi.mocked(pixelToWorld).mockImplementation(() => Promise.resolve({ points: [[202.4696, 47.1952]], frame: "icrs" }));
}

const Strip = (ViewerStatusStrip as unknown as { type: () => ReactElement }).type;

function draw(): string {
  return renderToStaticMarkup(hooks.render(Strip, undefined) as ReactElement);
}

async function settle(): Promise<string> {
  draw();
  await vi.advanceTimersByTimeAsync(0);
  draw();
  await vi.advanceTimersByTimeAsync(100);
  return draw();
}

function rootTag(markup: string): string {
  return markup.slice(0, markup.indexOf(">") + 1);
}

beforeEach(() => {
  vi.useFakeTimers();
  hooks.mount();
  view.pixel = null;
  view.path = "/data/m51_cal.fits";
  view.filePath = null;
  view.processed = null;
  answerProbes({ wcs: true, wavelength: MICRONS });
});

afterEach(() => {
  hooks.mount();
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe("ViewerStatusStrip root", () => {
  it("names the root statically, without a title, and keeps its tag identical across pixels and when idle", async () => {
    view.pixel = { x: 3, y: 4 };
    const first = await settle();
    view.pixel = { x: 5, y: 6 };
    const second = await settle();
    expect(second).toContain("x 5  y 6");
    view.pixel = null;
    const idle = await settle();
    expect(rootTag(first)).toBe(rootTag(second));
    expect(rootTag(idle)).toBe(rootTag(first));
    expect(rootTag(first)).toContain('role="group"');
    expect(rootTag(first)).toContain('aria-label="Pointer readout"');
    expect(rootTag(first)).not.toContain("title=");
  });
});

describe("ViewerStatusStrip parts", () => {
  it("renders position, value, wavelength and sky in order, with no null or undefined", async () => {
    view.pixel = { x: 3, y: 4 };
    const html = await settle();
    const order = ["x 3  y 4", "1234 MJy/sr", "λ 1.5000 um", "RA ", " ICRS"].map((part) => html.indexOf(part));
    expect(order.every((at, i) => at > -1 && (i === 0 || at > order[i - 1]))).toBe(true);
    expect(html).not.toMatch(/null|undefined/);
  });

  it("gives each part its static title and none to the sky part", async () => {
    view.pixel = { x: 3, y: 4 };
    const html = await settle();
    expect(html).toContain(`title="${ZERO_BASED_PIXEL_TITLE}">x 3  y 4</span>`);
    expect(html).toContain('title="Pixel value of the image being measured">1234 MJy/sr</span>');
    expect(html).toContain('title="Wavelength at this pixel from the WAVELENGTH extension">λ 1.5000 um</span>');
    const skyAt = html.indexOf("RA ");
    expect(html.slice(html.lastIndexOf("<span", skyAt), skyAt)).not.toContain("title=");
  });

  it("leaves out the wavelength and sky parts the file cannot give", async () => {
    answerProbes({ wcs: false, wavelength: null });
    view.pixel = { x: 3, y: 4 };
    const html = await settle();
    expect(html).toContain("1234 MJy/sr");
    expect(html).not.toContain("λ");
    expect(html).not.toContain("RA ");
    expect(html).not.toMatch(/null|undefined/);
  });
});

describe("ViewerStatusStrip idle text", () => {
  it("offers RA/Dec only for a file with a celestial WCS", async () => {
    expect(await settle()).toContain(">Hover the image for x, y, value and RA/Dec</span>");
    hooks.mount();
    answerProbes({ wcs: false, wavelength: null });
    expect(await settle()).toContain(">Hover the image for x, y and value</span>");
  });

  it("offers the wavelength once the file has shown one", async () => {
    view.pixel = { x: 3, y: 4 };
    await settle();
    view.pixel = null;
    expect(await settle()).toContain(">Hover the image for x, y, value, wavelength and RA/Dec</span>");
  });

  it("does not carry the wavelength over to another file", async () => {
    view.pixel = { x: 3, y: 4 };
    await settle();
    view.pixel = null;
    view.path = "/data/m51_i2d.fits";
    answerProbes({ wcs: false, wavelength: null });
    expect(await settle()).toContain(">Hover the image for x, y and value</span>");
  });
});

describe("ViewerStatusStrip over a cube frame shown before its FITS", () => {
  const CUBE = "/data/jw01366_nirspec_s3d.fits";
  const FRAME_FITS = "/out/cube_frame_abc_37.fits";
  const PLANE_0_VALUE = 1234;
  const FRAME_37_VALUE = 5678;

  function frame37(fitsPath: string | null): ProcessedResult {
    return {
      kind: "cube",
      frameIndex: 37,
      fitsPath,
      previewUrl: "asset://out/cube_frame_abc_37.png",
      dimensions: fitsPath === null ? null : [64, 64],
      label: "ch 37 (0–39)",
      inputPath: CUBE,
    };
  }

  beforeEach(() => {
    vi.mocked(probePixel).mockImplementation((path, x, y) =>
      Promise.resolve({ x, y, value: path === CUBE ? PLANE_0_VALUE : FRAME_37_VALUE, unit: "MJy/sr", wavelength: null } as PixelProbeResult),
    );
    view.filePath = CUBE;
    view.pixel = { x: 3, y: 4 };
  });

  it("does not print another plane's value while a PNG-only cube frame is shown", async () => {
    view.path = CUBE;
    view.processed = frame37(null);
    const html = await settle();
    expect(vi.mocked(probePixel).mock.calls.map((call) => call[0])).not.toContain(CUBE);
    expect(html).not.toContain(`${PLANE_0_VALUE} MJy/sr`);
    expect(html).toContain(`title="Pixel value of the image being measured">${CUBE_FRAME_VALUE_PENDING}</span>`);
    expect(html).toContain("x 3  y 4");
  });

  it("probes the frame's own FITS once it is published and prints its value", async () => {
    view.path = FRAME_FITS;
    view.processed = frame37(FRAME_FITS);
    const html = await settle();
    expect(vi.mocked(probePixel).mock.calls.map((call) => call[0])).toEqual([FRAME_FITS]);
    expect(html).toContain(`${FRAME_37_VALUE} MJy/sr`);
  });

  it("follows playback from a frame's PNG to its FITS and drops its value when the next frame shows without one", async () => {
    view.path = CUBE;
    view.processed = frame37(null);
    expect(await settle()).not.toContain(`${PLANE_0_VALUE} MJy/sr`);
    view.path = FRAME_FITS;
    view.processed = frame37(FRAME_FITS);
    expect(await settle()).toContain(`${FRAME_37_VALUE} MJy/sr`);
    view.path = CUBE;
    view.processed = { ...frame37(null), frameIndex: 38, previewUrl: "asset://out/cube_frame_abc_38.png", label: "ch 38 (0–39)" };
    const html = await settle();
    expect(vi.mocked(probePixel).mock.calls.map((call) => call[0])).toEqual([FRAME_FITS]);
    expect(html).not.toContain(`${FRAME_37_VALUE} MJy/sr`);
    expect(html).not.toContain(`${PLANE_0_VALUE} MJy/sr`);
    expect(html).toContain(`title="Pixel value of the image being measured">${CUBE_FRAME_VALUE_PENDING}</span>`);
  });
});
