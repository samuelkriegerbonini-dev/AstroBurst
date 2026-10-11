import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createFramePublishGate } from "../../../utils/cubeNavigation";
import type { CubeDims } from "../../../shared/types/cube";
import type { RampInfo } from "../../../shared/types/ramp";

const cubeState = vi.hoisted(() => ({ ramp: null as unknown }));

vi.mock("../../../context/PreviewContext", () => ({
  useCubeContext: () => ({ isCube: true, ramp: cubeState.ramp, rampSource: cubeState.ramp ? "fits" : null }),
  useFileContext: () => ({ file: { path: "C:/data/cube.fits", result: { header: { BUNIT: "DN" } } } }),
  useRenderContext: () => ({ processed: null }),
}));
vi.mock("../../../hooks/useSpectrumStore", () => ({
  useSpectrum: () => ({ region: null, regionSource: null, regionLoading: false, regionError: null }),
  beginRegionSpectrum: () => {},
  clearRegionSpectrum: () => {},
  commitRegionSpectrum: () => {},
  failRegionSpectrum: () => {},
}));
vi.mock("../../../hooks/useRampStore", () => ({ useRampIntegration: () => 0, setIntegration: () => {} }));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => null }));
vi.mock("../../../hooks/useRegionStore", () => ({ useRegionDoc: () => ({ regions: [], selectedId: null, version: 0 }) }));
vi.mock("../SpectralAxisControls", () => ({ default: () => "[axis-controls]" }));
vi.mock("../LineListControls", () => ({ default: () => "[line-list]" }));
vi.mock("../SpectrumComparisonSection", () => ({ default: () => "[compare]" }));
vi.mock("../LineFitSection", () => ({ default: () => "[line-fit]" }));
vi.mock("../LineMeasurementSection", () => ({ default: () => "[line-measure]" }));
vi.mock("../../CubeFrameNav", () => ({ default: () => "[frame-nav]" }));

import SpectroscopyPanel from "../SpectroscopyPanel";

const RAMP: RampInfo = {
  kind: "jwst_groups",
  nints: 1,
  ngroups: 10,
  nframes: 1,
  groupgap: 0,
  tframe_s: 14.58889,
  tgroup_s: 14.589,
  tgroup_source: "tgroup",
  group_times_s: null,
  readpatt: "NRSIRS2RAPID",
  instrument: "NIRSPEC",
  detector: "NRS1",
  exp_type: "NRS_IFU",
  datamodl: "Level1bModel",
  irs2: null,
  frame_width: 2048,
  frame_height: 3200,
};

const NON_SPECTRAL_DIMS: CubeDims = {
  width: 2048,
  height: 3200,
  frames: 10,
  spectral_axis: null,
  spectral_classification: { is_spectral: false, reason: "ramp: 10 groups x 1 integrations (NRSIRS2RAPID)" },
};

function renderPanel(ramp: RampInfo | null, spectrum: number[]): string {
  cubeState.ramp = ramp;
  return renderToStaticMarkup(
    createElement(SpectroscopyPanel, {
      spectrum,
      wavelengths: null,
      pixelCoord: spectrum.length > 0 ? { x: 1023, y: 2446 } : null,
      cubeDims: NON_SPECTRAL_DIMS,
      filePath: "C:/data/cube.fits",
      publishGate: createFramePublishGate(),
    }),
  );
}

function renderClickedPixel(ramp: RampInfo | null): string {
  return renderPanel(
    ramp,
    Array.from({ length: 10 }, (_, i) => 100 + i * 5),
  );
}

const SPECTRAL_ONLY_TEXT = ["Continuum", "continuum A", "SNR", "Moments", "[line-fit]", "[line-measure]", "[compare]"];

describe("SpectroscopyPanel spectral-axis widgets", () => {
  it("hides the spectral-axis controls and the line list for a ramp and keeps the group navigator", () => {
    const html = renderClickedPixel(RAMP);
    expect(html).toContain("[frame-nav]");
    expect(html).not.toContain("[axis-controls]");
    expect(html).not.toContain("[line-list]");
  });

  it("keeps the spectral-axis controls and the line list for a cube that is not a ramp", () => {
    const html = renderClickedPixel(null);
    expect(html).toContain("[frame-nav]");
    expect(html).toContain("[axis-controls]");
    expect(html).toContain("[line-list]");
  });
});

describe("SpectroscopyPanel spectral-only sections", () => {
  it("hides the continuum, Moments, Line fit, line measurement and comparison sections for a ramp", () => {
    const html = renderClickedPixel(RAMP);
    for (const text of SPECTRAL_ONLY_TEXT) expect(html).not.toContain(text);
    expect(html).toContain("Collapse Mean");
    expect(html).toContain("Collapse range");
  });

  it("hides the comparison section for a ramp before any spectrum is extracted", () => {
    expect(renderPanel(RAMP, [])).not.toContain("[compare]");
    expect(renderPanel(null, [])).toContain("[compare]");
  });

  it("keeps every spectral section for a cube that is not a ramp", () => {
    const html = renderClickedPixel(null);
    for (const text of SPECTRAL_ONLY_TEXT) expect(html).toContain(text);
  });
});

describe("SpectroscopyPanel channel and SNR boxes", () => {
  it("renders the range, continuum and SNR boxes as decimal text fields, none invalid before an edit", () => {
    const html = renderClickedPixel(null);
    expect(html).not.toContain('type="number"');
    expect(html.match(/inputMode="decimal"/g)?.length).toBe(7);
    expect(html).not.toContain("aria-invalid");
  });

  it("keeps only the line range boxes on a ramp", () => {
    const html = renderClickedPixel(RAMP);
    expect(html.match(/inputMode="decimal"/g)?.length).toBe(2);
    expect(html).toContain('aria-label="Range first channel z0"');
    expect(html).toContain('aria-label="Range last channel z1"');
  });
});

describe("SpectroscopyPanel grey text", () => {
  it("colours the idle export buttons through the text token instead of a grey literal", () => {
    const html = renderClickedPixel(null);
    expect(html).not.toContain("#71717a");
    expect(html).toContain("color:var(--ab-text-3)");
  });
});
