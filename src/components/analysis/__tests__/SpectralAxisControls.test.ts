import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { SpectralAxisInfo } from "../../../shared/types/spectral";

const correctionMock = vi.hoisted(() => vi.fn<(path: string) => Promise<unknown>>());

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/spectral", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/spectral")>()),
  getRadialVelocityCorrection: correctionMock,
}));

import { hooks } from "../../../test/hookRuntime";
import SpectralAxisControls, { type SpectralAxisControlsProps } from "../SpectralAxisControls";
import { documentAxis } from "../../../utils/spectrumDocument";

const controlsFunction = (SpectralAxisControls as unknown as { type: (props: SpectralAxisControlsProps) => ReactElement }).type;

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
  return renderToStaticMarkup(hooks.render(controlsFunction, props) as ReactElement);
}

beforeEach(() => {
  hooks.mount();
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

describe("SpectralAxisControls rest wavelength prefill", () => {
  it("keeps the picked rest wavelength when the document frame relabels the same axis as air", () => {
    const onRestChange = vi.fn();
    render({ axis: AXIS, restValue: null, onRestChange });
    expect(onRestChange).toHaveBeenLastCalledWith(null);
    render({ axis: AXIS, restValue: 0.65646, onRestChange });
    onRestChange.mockClear();
    const relabelled = documentAxis(AXIS, "air");
    expect(relabelled).not.toBe(AXIS);
    render({ axis: relabelled, restValue: 0.65646, onRestChange });
    expect(onRestChange).not.toHaveBeenCalled();
  });

  it("prefills the rest wavelength of a newly opened axis from its header", () => {
    const onRestChange = vi.fn();
    render({ axis: AXIS, restValue: 0.65646, onRestChange });
    onRestChange.mockClear();
    render({ axis: { ...AXIS, values: [2.16, 2.17, 2.18], rest_wavelength_um: 2.1661 }, restValue: 0.65646, onRestChange });
    expect(onRestChange).toHaveBeenCalledExactlyOnceWith(2.1661);
  });
});
