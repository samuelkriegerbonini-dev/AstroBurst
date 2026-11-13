import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

type Star = { x: number; y: number; fwhm: number; snr: number; ellipticity: number; peak: number; flux: number };
type Result = {
  kernel: number[][];
  kernel_size: number;
  average_fwhm: number;
  average_ellipticity: number;
  stars_used: Star[];
  stars_rejected: number;
  spread_pixels: number;
};

const live = vi.hoisted(() => ({ result: null as Result | null, scale: null as number | null, pixelScale: vi.fn() }));

vi.mock("../../../hooks/useProcessingRun", () => ({
  useProcessingRun: (kind: string) => ({
    running: false,
    blocked: false,
    busyTitle: undefined,
    result: kind === "psf" ? live.result : null,
    error: null,
    run: vi.fn(),
  }),
}));
vi.mock("../../../hooks/useCompositeChain", () => ({ useCompositeChain: () => ({ psfKernel: null }) }));
vi.mock("../../../hooks/useResolutionSettings", () => ({ useWcsPixelScale: live.pixelScale }));
vi.mock("../../../services/processing", () => ({ buildEmpiricalPsf: vi.fn(), estimatePsf: vi.fn() }));
vi.mock("../../../services/compositeChain", () => ({ compositeChainState: vi.fn(), compositeEstimatePsf: vi.fn() }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));

import PsfPanel from "../PsfPanel";

function star(fwhm: number): Star {
  return { x: 10, y: 20, fwhm, snr: 120, ellipticity: 0.04, peak: 1, flux: 1 };
}

const RESULT: Result = {
  kernel: [[1]],
  kernel_size: 1,
  average_fwhm: 4.2,
  average_ellipticity: 0.05,
  stars_used: [star(4.0), star(4.2), star(4.6)],
  stars_rejected: 5,
  spread_pixels: 6.1,
};

function render(compositeMode = false): string {
  return renderToStaticMarkup(
    createElement(PsfPanel, {
      selectedFile: { path: "a.fits" },
      fileKey: "a.fits",
      compositeMode,
      compositeInput: compositeMode ? { input: "base", previewUrl: "asset://out/blend.png", label: "Composite" } : null,
      fileName: "a.fits",
    }),
  );
}

function scatterRow(html: string): string {
  return html.match(/Star scatter<\/div><div[^>]*>([^<]*)<\/div>/)?.[1] ?? "";
}

beforeEach(() => {
  live.result = RESULT;
  live.scale = 0.262;
  live.pixelScale.mockReset();
  live.pixelScale.mockImplementation(() => live.scale);
});

describe("PSF panel units", () => {
  it("the FWHM is printed in pixels and arcsec", () => {
    expect(render()).toContain("4.20 px (1.100&quot;)");
    expect(live.pixelScale).toHaveBeenCalledWith("a.fits");
  });

  it("the scatter between the stars is printed with its definition", () => {
    const html = render();
    expect(html).toContain("Star scatter");
    expect(html).toContain("± 0.30 px (0.078&quot;)");
    expect(html).toContain('title="1.4826 × MAD (robust)"');
  });

  it("every star row carries the unit", () => {
    const html = render();
    expect(html).toContain("FWHM=4.0 px");
    expect(html).toContain("FWHM=4.2 px");
    expect(html).toContain("FWHM=4.6 px");
  });

  it("without a pixel scale only pixels are printed", () => {
    live.scale = null;
    const html = render();
    expect(html).toContain("4.20 px");
    expect(html).not.toContain("4.20 px (");
    expect(html).toContain("± 0.30 px");
    expect(html).not.toContain("± 0.30 px (");
  });

  it("fewer than three stars give no scatter", () => {
    live.result = { ...RESULT, stars_used: [star(4.0), star(4.6)] };
    const html = render();
    expect(html).toContain("Star scatter");
    expect(scatterRow(html)).toBe("--");
  });

  it("composite mode asks for no pixel scale", () => {
    render(true);
    expect(live.pixelScale).toHaveBeenCalledWith(null);
    expect(live.pixelScale).not.toHaveBeenCalledWith("a.fits");
  });
});
