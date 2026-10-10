import { describe, expect, it } from "vitest";
import { INITIAL_STATE, type WizardState } from "../wizard";
import type { ChannelSource } from "../channelMapping";
import {
  SPCC_DEFAULT_WAVELENGTHS_NM,
  spccResultWavelengths,
  spccWavelengths,
  spccWavelengthsLine,
} from "../spccWavelengths";

function stateWith(files: Record<string, string[]>, extra: Partial<WizardState> = {}): WizardState {
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: files[b.id] ?? [] })),
    ...extra,
  };
}

const frame = (path: string, filter: string): ChannelSource & { path: string } => ({
  path,
  name: path.split("/").pop(),
  result: { header: { FILTER: filter } },
});

const RGB = stateWith({ r: ["/d/spcc_r.fits"], g: ["/d/spcc_g.fits"], b: ["/d/spcc_b.fits"] });

const HST_FILES = [frame("/d/spcc_r.fits", "F814W"), frame("/d/spcc_g.fits", "F555W"), frame("/d/spcc_b.fits", "F435W")];

describe("SPCC wavelengths from the channel filters", () => {
  it("takes the R, G and B wavelengths from the FILTER cards of the R, G and B bins", () => {
    expect(spccWavelengths(RGB, HST_FILES)).toEqual({ nm: [814, 555, 435], codes: ["F814W", "F555W", "F435W"] });
  });

  it("sends no wavelengths when one filter is unknown, and still lists the known codes", () => {
    const files = [frame("/d/spcc_r.fits", "F814W"), frame("/d/spcc_g.fits", "Green"), frame("/d/spcc_b.fits", "F435W")];
    expect(spccWavelengths(RGB, files)).toEqual({ nm: null, codes: ["F814W", null, "F435W"] });
  });

  it("sends no wavelengths when a colour bin is empty", () => {
    const rg = stateWith({ r: ["/d/spcc_r.fits"], g: ["/d/spcc_g.fits"] });
    expect(spccWavelengths(rg, HST_FILES)).toEqual({ nm: null, codes: ["F814W", "F555W", null] });
  });

  it("reads the first frame that is not excluded", () => {
    const state = stateWith(
      { r: ["/d/r_old.fits", "/d/spcc_r.fits"], g: ["/d/spcc_g.fits"], b: ["/d/spcc_b.fits"] },
      { excludedFiles: { r: ["/d/r_old.fits"] } },
    );
    const files = [frame("/d/r_old.fits", "F775W"), ...HST_FILES];
    expect(spccWavelengths(state, files).nm).toEqual([814, 555, 435]);
  });

  it("reads the raw frame's filter after Stack and Align replaced the channel path", () => {
    const state = stateWith(
      { r: ["/d/spcc_r.fits"], g: ["/d/spcc_g.fits"], b: ["/d/spcc_b.fits"] },
      {
        stackedPaths: { r: "/out/stack_r.fits" },
        alignedPaths: { r: "__wizard_ch_tab1_r_aligned", g: "__wizard_ch_tab1_g_aligned", b: "__wizard_ch_tab1_b_aligned" },
      },
    );
    expect(spccWavelengths(state, HST_FILES)).toEqual({ nm: [814, 555, 435], codes: ["F814W", "F555W", "F435W"] });
  });

  it("falls back to the filter code in the file name when the frame is not loaded", () => {
    const state = stateWith({ r: ["/d/u2_f814w_drz.fits"], g: ["/d/u2_f555w_drz.fits"], b: ["/d/u2_f439w_drz.fits"] });
    expect(spccWavelengths(state, [])).toEqual({ nm: [814, 555, 439], codes: ["F814W", "F555W", "F439W"] });
  });

  it("describes the wavelengths SPCC will use", () => {
    expect(spccWavelengthsLine([814, 555, 435])).toBe(
      "Blackbody approximation at R/G/B 814/555/435 nm (from the channel filters)",
    );
    expect(spccWavelengthsLine(null)).toBe(
      "Blackbody approximation at R/G/B 640/530/460 nm (default; filter wavelengths unknown)",
    );
    expect(SPCC_DEFAULT_WAVELENGTHS_NM).toEqual([640, 530, 460]);
  });

  it("formats the wavelengths the backend used for the result grid", () => {
    expect(spccResultWavelengths({ wavelengths_nm: [814, 555, 435], wavelength_source: "filters" })).toBe(
      "814/555/435 nm (filters)",
    );
    expect(spccResultWavelengths({ wavelengths_nm: [640, 530, 460], wavelength_source: "default" })).toBe(
      "640/530/460 nm (default)",
    );
    expect(spccResultWavelengths({ wavelengths_nm: [640, 530, 460] })).toBe("640/530/460 nm");
    expect(spccResultWavelengths({})).toBeNull();
  });
});
