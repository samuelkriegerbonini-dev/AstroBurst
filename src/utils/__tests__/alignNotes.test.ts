import { describe, it, expect } from "vitest";
import {
  ALIGN_REF_AUTO,
  ALIGN_REF_AUTO_LABEL,
  alignChannelNote,
  alignNoWcsNote,
  alignNoWcsPairNote,
  alignReferenceIndex,
  alignWcsNote,
} from "../alignNotes";
import { formatAlignOffset } from "../wizard";
import type { AlignedChannel } from "../../shared/types/compose";

const CDP_AUTO_NOTE =
  /reprojected through WCS \(scale x2\.01\d\d, rotation \+0\.4\d°\); residual Δx [+−-]?\d\.\d px\s+Δy [+−-]?\d\.\d px/;
const CDP_COARSE_REF_NOTE = /scale x0\.49\d\d, rotation -0\.4\d°, pre-reduced x2\)/;

describe("alignWcsNote", () => {
  it("writes the scale, the signed rotation and the residual of a reprojected channel", () => {
    expect(alignWcsNote(2.014553, 0.486, 1, [-0.24, 0.31])).toBe(
      "reprojected through WCS (scale x2.0146, rotation +0.49°); residual Δx +0.3 px  Δy −0.2 px",
    );
  });

  it("prints a negative rotation with an ASCII minus and names the pre-reduction factor", () => {
    expect(alignWcsNote(0.496387, -0.486, 2, [0, 0])).toBe(
      "reprojected through WCS (scale x0.4964, rotation -0.49°, pre-reduced x2); residual Δx 0.0 px  Δy 0.0 px",
    );
  });

  it("always signs the rotation explicitly", () => {
    expect(alignWcsNote(1, 0.49, 1, [0, 0])).toContain("rotation +0.49°");
    expect(alignWcsNote(1, -0.49, 1, [0, 0])).toContain("rotation -0.49°");
    expect(alignWcsNote(1, 0, 1, [0, 0])).toContain("rotation +0.00°");
  });

  it("leaves the pre-reduction out when the channel was not reduced", () => {
    expect(alignWcsNote(1.0001, 0.001, 1, [0, 0])).not.toContain("pre-reduced");
  });

  it("ends with the offset text of formatAlignOffset verbatim, two spaces before Δy", () => {
    const offset: [number, number] = [-0.66, 1.04];
    const note = alignWcsNote(2.0146, 0.486, 1, offset);
    expect(note.endsWith(`; residual ${formatAlignOffset(offset)}`)).toBe(true);
    expect(note).toContain("px  Δy");
    expect(note.match(/Δx/g)).toHaveLength(1);
  });

  it("says the residual was not measured instead of printing an offset when there is none", () => {
    const note = alignWcsNote(2.014553, 0.486, 1, null);
    expect(note).toBe("reprojected through WCS (scale x2.0146, rotation +0.49°); residual not measured");
    expect(note).not.toContain("Δx");
  });

  it("matches the CDP patterns of C-A4-1 for the automatic and the coarse reference", () => {
    expect(alignWcsNote(2.01455, 0.486, 1, [0.4, -0.7])).toMatch(CDP_AUTO_NOTE);
    expect(alignWcsNote(0.4964, -0.486, 2, [0.1, 0.2])).toMatch(CDP_COARSE_REF_NOTE);
  });
});

describe("alignNoWcsNote", () => {
  it("names the channel and the registration method", () => {
    expect(alignNoWcsNote("G", "phase correlation")).toBe(
      "no celestial WCS in G; resampled by array size and registered by phase correlation",
    );
  });
});

describe("alignNoWcsPairNote", () => {
  it("names the channel, the reference and the registration method without saying which side lacks the WCS", () => {
    expect(alignNoWcsPairNote("Hα", "OIII", "phase correlation")).toBe(
      "no celestial WCS pair between Hα and the reference OIII; resampled by array size and registered by phase correlation",
    );
  });
});

describe("alignChannelNote", () => {
  const reprojected: AlignedChannel = {
    offset: [-0.24, 0.31],
    method_used: "phase_correlation",
    registered: true,
    reprojected: true,
    wcs_scale_ratio: 2.014553,
    wcs_rotation_deg: 0.486,
    prefilter_k: null,
    residual_measured: true,
  };

  it("gives the reference row no note", () => {
    expect(alignChannelNote(reprojected, "B", "phase_correlation", true)).toBeNull();
    expect(alignChannelNote(undefined, "B", "phase_correlation", false)).toBeNull();
  });

  it("describes a reprojected channel through the WCS template, pre-reduction defaulting to none", () => {
    expect(alignChannelNote(reprojected, "B", "phase_correlation", false)).toEqual({
      text: "reprojected through WCS (scale x2.0146, rotation +0.49°); residual Δx +0.3 px  Δy −0.2 px",
      wcs: true,
    });
    expect(alignChannelNote({ ...reprojected, prefilter_k: 2 }, "B", "phase_correlation", false)?.text).toContain(
      ", pre-reduced x2)",
    );
  });

  it("does not print the zero offset of a channel the WCS alone registered as a measured residual", () => {
    const wcsOnly: AlignedChannel = { ...reprojected, offset: [0, 0], method_used: "wcs", registered: true, residual_measured: false };
    expect(alignChannelNote(wcsOnly, "B", "phase_correlation", false)).toEqual({
      text: "reprojected through WCS (scale x2.0146, rotation +0.49°); residual not measured",
      wcs: true,
    });
    expect(alignChannelNote({ ...wcsOnly, registered: false }, "B", "phase_correlation", false)?.text).toMatch(/; residual not measured$/);
    expect(alignChannelNote({ ...reprojected, residual_measured: undefined }, "B", "phase_correlation", false)?.text).toMatch(
      /; residual Δx \+0\.3 px {2}Δy −0\.2 px$/,
    );
  });

  it("explains every non-reference channel that was not reprojected", () => {
    const plain: AlignedChannel = { offset: [3, -2], method_used: "phase_correlation", registered: true };
    expect(alignChannelNote(plain, "G", "phase_correlation", false)).toEqual({
      text: "no celestial WCS in G; resampled by array size and registered by phase correlation",
      wcs: false,
    });
    expect(alignChannelNote({ ...plain, reprojected: false, wcs_scale_ratio: null }, "G", "phase_correlation", false)?.wcs).toBe(false);
  });

  it("names the pair, not the channel, when the user selected the reference, which may be the side without a WCS", () => {
    const plain: AlignedChannel = { offset: [3, -2], method_used: "phase_correlation", registered: true, reprojected: false };
    expect(alignChannelNote(plain, "G", "phase_correlation", false, "selected", "B")).toEqual({
      text: "no celestial WCS pair between G and the reference B; resampled by array size and registered by phase correlation",
      wcs: false,
    });
    expect(alignChannelNote({ offset: [0, 0], method_used: "affine" }, "SII", "phase_correlation", false, "selected", "Hα")?.text).toBe(
      "no celestial WCS pair between SII and the reference Hα; resampled by array size and registered by affine",
    );
  });

  it("keeps the pinned no-WCS text when the backend chose the reference", () => {
    const plain: AlignedChannel = { offset: [3, -2], method_used: "phase_correlation", registered: true, reprojected: false };
    const pinned = "no celestial WCS in G; resampled by array size and registered by phase correlation";
    expect(alignChannelNote(plain, "G", "phase_correlation", false, "finest_wcs", "B")?.text).toBe(pinned);
    expect(alignChannelNote(plain, "G", "phase_correlation", false, "first", "B")?.text).toBe(pinned);
    expect(alignChannelNote(plain, "G", "phase_correlation", false, undefined, "B")?.text).toBe(pinned);
  });

  it("keeps the WCS note of a reprojected channel under a selected reference", () => {
    expect(alignChannelNote(reprojected, "B", "phase_correlation", false, "selected", "G")).toEqual({
      text: "reprojected through WCS (scale x2.0146, rotation +0.49°); residual Δx +0.3 px  Δy −0.2 px",
      wcs: true,
    });
  });

  it("names the method the channel was registered by, and the requested one when the backend names none", () => {
    expect(alignChannelNote({ offset: [0, 0], method_used: "affine" }, "SII", "phase_correlation", false)?.text).toBe(
      "no celestial WCS in SII; resampled by array size and registered by affine",
    );
    expect(alignChannelNote({ offset: [0, 0] }, "SII", "phase_correlation", false)?.text).toBe(
      "no celestial WCS in SII; resampled by array size and registered by phase correlation",
    );
  });
});

describe("alignReferenceIndex", () => {
  const inputs = [{ binId: "ha" }, { binId: "oiii" }, { binId: "sii" }];

  it("sends no index for the automatic choice", () => {
    expect(alignReferenceIndex(inputs, null)).toBeNull();
  });

  it("sends the position of the chosen bin among the align inputs", () => {
    expect(alignReferenceIndex(inputs, "oiii")).toBe(1);
    expect(alignReferenceIndex(inputs, "ha")).toBe(0);
  });

  it("falls back to automatic when the chosen bin is not an align input", () => {
    expect(alignReferenceIndex(inputs, "r")).toBeNull();
  });

  it("labels the automatic option", () => {
    expect(ALIGN_REF_AUTO).toBe("auto");
    expect(ALIGN_REF_AUTO_LABEL).toBe("Auto (finest pixel scale)");
  });
});
