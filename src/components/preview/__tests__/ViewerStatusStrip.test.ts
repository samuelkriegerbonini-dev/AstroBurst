import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { hooks } from "../../../test/hookRuntime";
import { ZERO_BASED_PIXEL_TITLE } from "../../../utils/regionGeometry";
import { getWcsInfo, pixelToWorld } from "../../../services/astrometry";
import { probePixel } from "../../../services/display";
import type { PixelProbeResult, WavelengthProbe } from "../../../shared/types/analysis";
import type { WcsInfo } from "../../../shared/types/astrometry";

vi.mock("react", async (importOriginal) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await importOriginal());
});

interface StripView {
  pixel: { x: number; y: number } | null;
  path: string | null;
}

const view: StripView = { pixel: null, path: "/data/m51_cal.fits" };

vi.mock("../../../hooks/useMousePixelStore", () => ({ useMousePixel: () => view.pixel }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({
  useAnalysisTarget: () => ({ path: view.path, composite: false, fileRgbView: false }),
}));
vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: view.path === null ? null : { path: view.path, result: { dimensions: [64, 64] } } }),
  useRenderContext: () => ({ processed: null, chain: { steps: {}, psfKernel: null, base: null } }),
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
