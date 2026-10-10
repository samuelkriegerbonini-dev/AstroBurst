import { describe, expect, it } from "vitest";
import { gwcsSuffix, orientationLine, sipSuffix } from "../wcsOrientation";
import type { WcsInfo } from "../../shared/types/astrometry";

function info(overrides: Partial<WcsInfo> = {}): WcsInfo {
  return {
    center_ra: 274.729893924,
    center_dec: -13.851839479,
    pixel_scale_arcsec: 0.0312,
    fov_arcmin: [7.459, 4.466],
    field_of_view_w_arcmin: 7.459,
    field_of_view_h_arcmin: 4.466,
    naxis1: 14344,
    naxis2: 8589,
    projection: "TAN",
    pixel_scale_x_arcsec: 0.0312,
    pixel_scale_y_arcsec: 0.0312,
    rotation_deg: -0.5,
    flipped: false,
    sip_present: false,
    ...overrides,
  };
}

describe("orientationLine", () => {
  it("ends with the gWCS fit residual when the SIP header carries SIPMXERR", () => {
    const line = orientationLine(info({ sip_present: true, sip_max_err_px: 0.0098 }));
    expect(line).toBe('TAN - 0.031"/px x 0.031"/px - rot -0.5 deg - E left - SIP (gWCS fit ≤0.010 px)');
    expect(line!.endsWith("- SIP (gWCS fit ≤0.010 px)")).toBe(true);
  });

  it("ends with a bare SIP marker when the SIP header has no residual card", () => {
    const line = orientationLine(info({ sip_present: true, sip_max_err_px: null, sip_inv_err_px: null }));
    expect(line!.endsWith(" - SIP")).toBe(true);
    expect(line).not.toContain("gWCS");
  });

  it("has no SIP text without SIP terms, even when a residual is reported", () => {
    const line = orientationLine(info({ sip_present: false, sip_max_err_px: 0.0098 }));
    expect(line).not.toContain("SIP");
    expect(line).toMatch(/^TAN - 0\.031"\/px x 0\.031"\/px - rot .* - E (left|right)$/);
  });

  it("keeps the i2d orientation line of the f200w mosaic unchanged", () => {
    expect(orientationLine(info({ flipped: true, rotation_deg: 1.25 }))).toBe(
      'TAN - 0.031"/px x 0.031"/px - rot 1.3 deg - E right',
    );
  });

  it("returns null when an orientation field is missing", () => {
    expect(orientationLine(info({ projection: undefined }))).toBeNull();
    expect(orientationLine(info({ rotation_deg: undefined }))).toBeNull();
  });
});

describe("sipSuffix", () => {
  it("is empty without SIP terms", () => {
    expect(sipSuffix({ sip_present: false })).toBe("");
    expect(sipSuffix({})).toBe("");
  });

  it("names SIP alone when no finite residual is known", () => {
    expect(sipSuffix({ sip_present: true })).toBe(" - SIP");
    expect(sipSuffix({ sip_present: true, sip_max_err_px: Number.NaN, sip_inv_err_px: 0.012 })).toBe(" - SIP");
  });

  it("formats the forward residual with three decimals", () => {
    expect(sipSuffix({ sip_present: true, sip_max_err_px: 0.0098 })).toBe(" - SIP (gWCS fit ≤0.010 px)");
    expect(sipSuffix({ sip_present: true, sip_max_err_px: 0.0123, sip_inv_err_px: null })).toBe(" - SIP (gWCS fit ≤0.012 px)");
  });

  it("adds the inverse residual when both are finite", () => {
    expect(sipSuffix({ sip_present: true, sip_max_err_px: 0.0098, sip_inv_err_px: 0.0123 })).toBe(
      " - SIP (gWCS fit ≤0.010 px, inverse ≤0.012 px)",
    );
  });
});

const NIRCAM_FRAMES = ["detector", "v2v3", "v2v3vacorr", "world"];

function gwcsInfo(overrides: Partial<WcsInfo> = {}): WcsInfo {
  return info({
    projection: "gWCS",
    wcs_kind: "gwcs",
    gwcs_steps: 3,
    gwcs_frames: NIRCAM_FRAMES,
    gwcs_source: "ASDF HDU 8",
    pixel_scale_x_arcsec: 0.0312,
    pixel_scale_y_arcsec: 0.0312,
    rotation_deg: 92.4,
    flipped: false,
    sip_present: false,
    sip_max_err_px: 0.008736956112743353,
    sip_inv_err_px: 0.008801265065873224,
    gwcs_vs_header_sip_max_mas: 0.266,
    gwcs_vs_header_sip_max_px: 0.0085,
    gwcs_refusal: null,
    ...overrides,
  });
}

const TABULAR_REFUSAL = "gWCS transform 'tabular' at steps[0].transform is not supported by the evaluator";
const DIVIDE_REFUSAL =
  "gWCS transform 'divide' at steps[2].transform/forward[0]/forward[0]/forward[0]/forward[0]/forward[0]/forward[1]/forward[0] is not supported by the evaluator";

describe("orientationLine with a gWCS", () => {
  it("renders the gWCS orientation line", () => {
    expect(orientationLine(gwcsInfo())).toBe(
      'gWCS (3 steps: detector→v2v3→v2v3vacorr→world) - 0.031"/px x 0.031"/px - rot 92.4 deg - E left - header SIP within 0.009 px (0.27 mas)',
    );
  });

  it("matches the Roman Info line of the CDP check, below the residual resolution", () => {
    const line = orientationLine(
      gwcsInfo({
        gwcs_source: "roman.meta.wcs",
        pixel_scale_x_arcsec: 0.11034,
        pixel_scale_y_arcsec: 0.10828,
        rotation_deg: 0.14,
        gwcs_vs_header_sip_max_px: 3.6e-7,
        gwcs_vs_header_sip_max_mas: 4.0e-5,
      }),
    );
    expect(line).toBe(
      'gWCS (3 steps: detector→v2v3→v2v3vacorr→world) - 0.110"/px x 0.108"/px - rot 0.1 deg - E left - header SIP within <0.001 px (0.00 mas)',
    );
    expect(line).toMatch(
      /^gWCS \(3 steps: detector→v2v3→v2v3vacorr→world\) - 0\.110"\/px x 0\.108"\/px - rot 0\.1 deg - E left - header SIP within <0\.001 px \(0\.00 mas\)$/,
    );
  });

  it("matches the NIRCam Info line of the CDP check", () => {
    expect(orientationLine(gwcsInfo({ gwcs_vs_header_sip_max_mas: 0.2661699, gwcs_vs_header_sip_max_px: 0.008531 }))).toMatch(
      /^gWCS \(3 steps: detector→v2v3→v2v3vacorr→world\) - 0\.031"\/px x 0\.031"\/px - rot .* deg - E (left|right) - header SIP within 0\.00[89] px \(0\.2\d mas\)$/,
    );
  });

  it("ends at the east side when there is no header SIP to compare with", () => {
    expect(orientationLine(gwcsInfo({ gwcs_vs_header_sip_max_mas: null, gwcs_vs_header_sip_max_px: null, flipped: true }))).toBe(
      'gWCS (3 steps: detector→v2v3→v2v3vacorr→world) - 0.031"/px x 0.031"/px - rot 92.4 deg - E right',
    );
  });

  it("never shows the SIP marker or a refusal in gWCS mode", () => {
    const line = orientationLine(gwcsInfo({ sip_present: true, gwcs_refusal: TABULAR_REFUSAL }));
    expect(line).not.toContain(" - SIP");
    expect(line).not.toContain("gWCS not used");
    expect(line!.endsWith(" - header SIP within 0.009 px (0.27 mas)")).toBe(true);
  });

  it("falls back to the projection name when the step list is missing", () => {
    expect(orientationLine(gwcsInfo({ gwcs_steps: null, gwcs_frames: null }))).toBe(
      'gWCS - 0.031"/px x 0.031"/px - rot 92.4 deg - E left - header SIP within 0.009 px (0.27 mas)',
    );
  });
});

describe("orientationLine on a header fallback with a gWCS refusal", () => {
  it("cuts the refusal suffix to 80 characters with an ellipsis", () => {
    const plain = orientationLine(info({ wcs_kind: "header" }))!;
    const line = orientationLine(info({ wcs_kind: "header", gwcs_refusal: TABULAR_REFUSAL }))!;
    expect(line).toBe(
      `TAN - 0.031"/px x 0.031"/px - rot -0.5 deg - E left - gWCS not used: gWCS transform 'tabular' at steps[0].transform is not supporte…`,
    );
    expect(line).toMatch(/^TAN - .* - E (left|right) - gWCS not used: gWCS transform 'tabular' at steps\[0\]\.transform.*$/);
    expect(line.length).toBe(plain.length + 80 + 1);
  });

  it("keeps a refusal suffix of exactly 80 characters whole", () => {
    const fits = "r".repeat(62);
    expect(` - gWCS not used: ${fits}`.length).toBe(80);
    const whole = orientationLine(info({ wcs_kind: "header", gwcs_refusal: fits }))!;
    expect(whole.endsWith(` - gWCS not used: ${fits}`)).toBe(true);
    expect(whole).not.toContain("…");
    const cut = orientationLine(info({ wcs_kind: "header", gwcs_refusal: `${fits}s` }))!;
    expect(cut.endsWith(` - gWCS not used: ${fits}…`)).toBe(true);
  });

  it("puts the SIP residual before the refusal of a tweakreg-corrected exposure", () => {
    const line = orientationLine(
      info({
        wcs_kind: "header",
        sip_present: true,
        sip_max_err_px: 0.008736956112743353,
        sip_inv_err_px: 0.008801265065873224,
        gwcs_refusal: DIVIDE_REFUSAL,
      }),
    );
    expect(line).toBe(
      `TAN - 0.031"/px x 0.031"/px - rot -0.5 deg - E left - SIP (gWCS fit ≤0.009 px, inverse ≤0.009 px) - gWCS not used: gWCS transform 'divide' at steps[2].transform/forward[0]/forwa…`,
    );
  });

  it("shows no refusal without a header kind or without a reason", () => {
    expect(orientationLine(info({ gwcs_refusal: TABULAR_REFUSAL }))).toBe('TAN - 0.031"/px x 0.031"/px - rot -0.5 deg - E left');
    expect(orientationLine(info({ wcs_kind: "header", gwcs_refusal: null }))).toBe(
      'TAN - 0.031"/px x 0.031"/px - rot -0.5 deg - E left',
    );
  });
});

describe("gwcsSuffix", () => {
  it("is empty when the header SIP comparison is not finite", () => {
    expect(gwcsSuffix({})).toBe("");
    expect(gwcsSuffix({ gwcs_vs_header_sip_max_px: null, gwcs_vs_header_sip_max_mas: 0.266 })).toBe("");
    expect(gwcsSuffix({ gwcs_vs_header_sip_max_px: Number.NaN, gwcs_vs_header_sip_max_mas: 0.266 })).toBe("");
    expect(gwcsSuffix({ gwcs_vs_header_sip_max_px: 0.0085, gwcs_vs_header_sip_max_mas: null })).toBe("");
  });

  it("prints three decimals from half a thousandth of a pixel up", () => {
    expect(gwcsSuffix({ gwcs_vs_header_sip_max_px: 0.0005, gwcs_vs_header_sip_max_mas: 0.057 })).toBe(
      " - header SIP within 0.001 px (0.06 mas)",
    );
    expect(gwcsSuffix({ gwcs_vs_header_sip_max_px: 0.00049, gwcs_vs_header_sip_max_mas: 0.054 })).toBe(
      " - header SIP within <0.001 px (0.05 mas)",
    );
  });
});
