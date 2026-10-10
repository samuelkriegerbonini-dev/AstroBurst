import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { captured, spccCalibrateMock } = vi.hoisted(() => ({
  captured: { onClick: null as null | (() => void | Promise<void>) },
  spccCalibrateMock: vi.fn(),
}));

vi.mock("../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui")>();
  return {
    ...actual,
    RunButton: (props: { onClick: () => void | Promise<void> }) => {
      captured.onClick = props.onClick;
      return null;
    },
  };
});

vi.mock("../../../services/processing", () => ({
  spccCalibrate: spccCalibrateMock,
}));

import SpccPanel from "../SpccPanel";

const SPCC_RESPONSE = {
  r_factor: 1.08,
  g_factor: 1,
  b_factor: 0.93,
  stars_matched: 25,
  stars_total: 25,
  elapsed_ms: 40,
  wavelengths_nm: [814, 555, 435],
  wavelength_source: "filters",
  catalog_source: null,
};

function render(wavelengthsNm: [number, number, number] | null): string {
  captured.onClick = null;
  return renderToStaticMarkup(createElement(SpccPanel, {
    rPath: "/c/spcc_f814w.fits",
    gPath: "/c/spcc_f555w.fits",
    bPath: "/c/spcc_f435w.fits",
    wavelengthsNm,
  }));
}

function wavelengthsText(html: string): string | null {
  const match = /data-testid="spcc-wavelengths"[^>]*>([^<]*)</.exec(html);
  return match ? match[1] : null;
}

async function runSpcc(): Promise<void> {
  expect(captured.onClick).toBeTypeOf("function");
  await captured.onClick?.();
}

describe("SPCC panel header", () => {
  it("describes the method as a blackbody approximation and keeps the wavelength line", () => {
    const html = render([814, 555, 435]);
    expect(html).toContain(">Color calibration (blackbody approximation)<");
    expect(html).not.toMatch(/spectrophotometric/i);
    expect(wavelengthsText(html)).toBe("Blackbody approximation at R/G/B 814/555/435 nm (from the channel filters)");
  });
});

describe("SPCC panel wavelengths", () => {
  beforeEach(() => {
    spccCalibrateMock.mockReset();
    spccCalibrateMock.mockResolvedValue(SPCC_RESPONSE);
  });

  it("names the filter wavelengths and sends them to SPCC", async () => {
    const html = render([814, 555, 435]);
    expect(wavelengthsText(html)).toBe("Blackbody approximation at R/G/B 814/555/435 nm (from the channel filters)");
    await runSpcc();
    expect(spccCalibrateMock).toHaveBeenCalledTimes(1);
    expect(spccCalibrateMock).toHaveBeenCalledWith("/c/spcc_f814w.fits", "/c/spcc_f555w.fits", "/c/spcc_f435w.fits", {
      wcsPath: undefined,
      whiteReference: "average_spiral",
      minSnr: 20,
      catalog: "gaia",
      wavelengthsNm: [814, 555, 435],
    });
  });

  it("names the default wavelengths and sends null when a filter is unknown", async () => {
    const html = render(null);
    expect(wavelengthsText(html)).toBe(
      "Blackbody approximation at R/G/B 640/530/460 nm (default; filter wavelengths unknown)",
    );
    await runSpcc();
    expect(spccCalibrateMock.mock.calls[0][3]).toEqual({
      wcsPath: undefined,
      whiteReference: "average_spiral",
      minSnr: 20,
      catalog: "gaia",
      wavelengthsNm: null,
    });
  });
});
