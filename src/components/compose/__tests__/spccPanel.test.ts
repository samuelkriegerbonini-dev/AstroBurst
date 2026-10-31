import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { captured, spccCalibrateMock, headerRead } = vi.hoisted(() => ({
  captured: { onClick: null as null | (() => void | Promise<void>), disabled: null as boolean | null },
  spccCalibrateMock: vi.fn(),
  headerRead: {
    probe: null as null | { path: string; found: boolean; pixelScaleArcsec: number | null },
    paths: [] as (string | null)[],
  },
}));

vi.mock("../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui")>();
  return {
    ...actual,
    RunButton: (props: { onClick: () => void | Promise<void>; disabled?: boolean }) => {
      captured.onClick = props.onClick;
      captured.disabled = props.disabled ?? false;
      return null;
    },
  };
});

vi.mock("../../../services/processing", () => ({
  spccCalibrate: spccCalibrateMock,
}));

vi.mock("../../../hooks/useHeaderWcs", () => ({
  useHeaderWcs: (path: string | null) => {
    headerRead.paths.push(path);
    return path === null ? null : headerRead.probe;
  },
}));

import SpccPanel from "../SpccPanel";
import type { SpccHeaderWcs } from "../../../utils/solvedWcs";

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

function renderWith(
  extra: { wcsPath?: string | null; wcsReason?: string | null; headerWcs?: SpccHeaderWcs | null },
  wavelengthsNm: [number, number, number] | null = null,
): string {
  captured.onClick = null;
  captured.disabled = null;
  return renderToStaticMarkup(createElement(SpccPanel, {
    rPath: "/c/spcc_f814w.fits",
    gPath: "/c/spcc_f555w.fits",
    bPath: "/c/spcc_f435w.fits",
    wavelengthsNm,
    ...extra,
  }));
}

function render(wavelengthsNm: [number, number, number] | null): string {
  return renderWith({}, wavelengthsNm);
}

function wcsText(html: string): string | null {
  const match = /data-testid="spcc-wcs"[^>]*>([^<]*)</.exec(html);
  return match ? match[1] : null;
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

const ALIGN_REASON =
  "SPCC can use a plate-solved WCS only on the R channel's own grid; R was aligned onto G. Make R the alignment reference and run Align again.";

describe("SPCC panel WCS source", () => {
  beforeEach(() => {
    spccCalibrateMock.mockReset();
    spccCalibrateMock.mockResolvedValue(SPCC_RESPONSE);
  });

  it("names the plate-solved WCS file it sends to SPCC", async () => {
    const html = renderWith({ wcsPath: "C:\\out\\spcc_f814w_wcs.fits" });
    expect(wcsText(html)).toBe("WCS from spcc_f814w_wcs.fits");
    await runSpcc();
    expect(spccCalibrateMock.mock.calls[0][3].wcsPath).toBe("C:\\out\\spcc_f814w_wcs.fits");
  });

  it("shows why a plate-solved WCS cannot be used instead of a source", () => {
    const html = renderWith({ wcsReason: ALIGN_REASON });
    expect(wcsText(html)).toBe(ALIGN_REASON.replace(/'/g, "&#x27;"));
    expect(html).not.toContain("WCS from");
  });

  it("shows no WCS line when it is given neither a WCS source nor a header to read", () => {
    expect(wcsText(render(null))).toBeNull();
  });
});

const R_HEADER: SpccHeaderWcs = { path: "/c/spcc_f814w.fits", alignedOnto: null, solvePath: "/c/spcc_f814w.fits" };

describe("SPCC panel header WCS", () => {
  beforeEach(() => {
    headerRead.probe = null;
    headerRead.paths = [];
  });

  it("names the header WCS SPCC will use for the R channel and keeps Run enabled", () => {
    headerRead.probe = { path: "/c/spcc_f814w.fits", found: true, pixelScaleArcsec: 0.0996 };
    const html = renderWith({ headerWcs: R_HEADER });
    expect(wcsText(html)).toBe("WCS: header of spcc_f814w.fits, 0.0996″/px");
    expect(captured.disabled).toBe(false);
    expect(html).not.toContain("Solve R channel");
    expect(headerRead.paths).toContain("/c/spcc_f814w.fits");
  });

  it("without a WCS, Run is disabled with the reason and Solve R channel is offered", () => {
    headerRead.probe = { path: "/c/spcc_f814w.fits", found: false, pixelScaleArcsec: null };
    const html = renderWith({ headerWcs: R_HEADER });
    expect(wcsText(html)).toBe("No sky solution on the R channel: spcc_f814w.fits has no celestial WCS in its header and was not plate solved.");
    expect(captured.disabled).toBe(true);
    expect(html).toMatch(/<button[^>]*>Solve R channel<\/button>/);
    expect(html).not.toContain("Assign R, G, B channels above");
  });

  it("keeps Run disabled while the header is read", () => {
    const html = renderWith({ headerWcs: R_HEADER });
    expect(wcsText(html)).toBe("Reading the header WCS of spcc_f814w.fits");
    expect(captured.disabled).toBe(true);
    expect(html).not.toContain("Solve R channel");
  });

  it("reads no header once a plate-solved WCS is passed", () => {
    headerRead.probe = { path: "/c/spcc_f814w.fits", found: false, pixelScaleArcsec: null };
    const html = renderWith({ wcsPath: "C:\\out\\spcc_f814w_wcs.fits", headerWcs: R_HEADER });
    expect(wcsText(html)).toBe("WCS from spcc_f814w_wcs.fits");
    expect(captured.disabled).toBe(false);
    expect(headerRead.paths.every((p) => p === null)).toBe(true);
  });

  it("names the alignment reference's header and keeps the aligned-onto reason as a note", () => {
    headerRead.probe = { path: "/c/spcc_f555w.fits", found: true, pixelScaleArcsec: 1.08 };
    const html = renderWith({ wcsReason: ALIGN_REASON, headerWcs: { path: "/c/spcc_f555w.fits", alignedOnto: "G", solvePath: null } });
    expect(wcsText(html)).toBe("WCS: header of spcc_f555w.fits (alignment reference G), 1.08″/px");
    expect(html).toContain(`data-testid="spcc-wcs-note"`);
    expect(captured.disabled).toBe(false);
  });
});

describe("SPCC panel catalogue", () => {
  it("the catalogue select has no synthetic option", () => {
    const html = render(null);
    const options = [...html.matchAll(/<option value="([^"]*)"[^>]*>([^<]*)<\/option>/g)].map((m) => ({ value: m[1], label: m[2] }));
    expect(options).toContainEqual({ value: "gaia", label: "Gaia DR3 (VizieR, online)" });
    expect(options.some((o) => o.value === "builtin" || /synthetic/i.test(o.label))).toBe(false);
  });
});

describe("SPCC panel Min SNR field", () => {
  it("is a decimal text field rather than a number input", () => {
    const html = render(null);
    expect(html).not.toContain('type="number"');
    expect(html).toMatch(/<input[^>]*inputMode="decimal"|<input[^>]*inputmode="decimal"/);
    expect(html).toContain('value="20"');
  });
});
