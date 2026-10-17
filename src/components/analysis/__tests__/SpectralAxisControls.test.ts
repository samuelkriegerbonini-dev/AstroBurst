import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { SpectralAxisInfo } from "../../../shared/types/spectral";

const effects = vi.hoisted(() => ({ pending: [] as (() => unknown)[] }));
const correctionMock = vi.hoisted(() => vi.fn<(path: string) => Promise<unknown>>());

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  return {
    ...actual,
    useEffect: (effect: () => unknown) => {
      effects.pending.push(effect);
    },
  };
});
vi.mock("../../../services/spectral", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/spectral")>()),
  getRadialVelocityCorrection: correctionMock,
}));

import SpectralAxisControls, { type SpectralAxisControlsProps } from "../SpectralAxisControls";

const AXIS: SpectralAxisInfo = {
  kind: "wave",
  ctype: "WAVE",
  unit: "um",
  header_unit: "um",
  header_scale: 1,
  values: [0.6555, 0.6556, 0.6557],
  crval: 0.6555,
  cdelt: 1e-4,
  crpix: 1,
  rest_wavelength_um: null,
  rest_frequency_hz: null,
  specsys: null,
  velosys: null,
  notes: [],
};

const NOTE = "The correction is not computed for a spectrum file.";

function render(extra: Partial<SpectralAxisControlsProps>): string {
  effects.pending = [];
  const props: SpectralAxisControlsProps = {
    filePath: null,
    axis: AXIS,
    mode: "wavelength_vac",
    onModeChange: () => {},
    restValue: null,
    onRestChange: () => {},
    convention: "optical",
    onConventionChange: () => {},
    correction: "none",
    onCorrectionChange: () => {},
    ...extra,
  };
  const html = renderToStaticMarkup(createElement(SpectralAxisControls, props));
  effects.pending.forEach((effect) => effect());
  return html;
}

beforeEach(() => {
  correctionMock.mockReset();
  correctionMock.mockResolvedValue({ error: "no target coordinates" });
});

describe("SpectralAxisControls correction request", () => {
  it("asks for the correction of a cube or image path", () => {
    render({ filePath: "C:/data/cube.fits" });
    expect(correctionMock).toHaveBeenCalledTimes(1);
    expect(correctionMock).toHaveBeenCalledWith("C:/data/cube.fits");
  });

  it("sends no request and states why when the panel says the correction is unavailable", () => {
    const loaded = vi.fn();
    const html = render({ filePath: "C:/data/ha_bess.fits", correctionUnavailable: NOTE, onCorrectionLoaded: loaded });
    expect(correctionMock).not.toHaveBeenCalled();
    expect(html).toContain(NOTE);
    expect(html).not.toContain("Load a spectrum or cube");
    expect(loaded).toHaveBeenCalledWith(null);
  });

  it("keeps the load hint when no file is selected", () => {
    const html = render({ filePath: null });
    expect(correctionMock).not.toHaveBeenCalled();
    expect(html).toContain("Load a spectrum or cube to compute the correction.");
    expect(html).not.toContain(NOTE);
  });
});
