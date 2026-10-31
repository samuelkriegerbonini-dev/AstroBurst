import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("../../../../services/compose", () => ({
  blendChannels: vi.fn(),
  lrgbCombineComposite: vi.fn(),
  renderLinearCompositePreview: vi.fn(),
}));

vi.mock("../../../../services/channelLevels", () => ({
  measureChannelLevels: vi.fn(),
}));

vi.mock("../../../../infrastructure/tauri", () => ({
  getOutputDir: vi.fn(async () => "/out"),
}));

vi.mock("../../../../hooks/usePointingOverlap", () => ({
  usePointingOverlap: () => ({ disjointPairs: [] }),
}));

vi.mock("../../../../context/CompositeContext", () => {
  const stf = { shadow: 0, midtone: 0.5, highlight: 1 };
  return {
    useCompositeStf: () => ({ compositeStfR: stf, compositeStfG: stf, compositeStfB: stf, compositeStfLinked: true }),
  };
});

import BlendStep from "../BlendStep";
import { presetBinNames, presetPlan, resolvePresetWeights } from "../../../../utils/blendWeights";
import { BLEND_PRESETS, INITIAL_STATE, type FrequencyBin, type WizardState } from "../../../../utils/wizard";
import type { ProcessedFile } from "../../../../shared/types";

function binsWith(files: Record<string, string[]>): FrequencyBin[] {
  return INITIAL_STATE.bins.map((b) => ({ ...b, files: files[b.id] ?? [] }));
}

function loadedFile(path: string, header: Record<string, string>): ProcessedFile {
  return {
    id: path,
    name: path.split("/").pop() ?? path,
    path,
    sourcePath: path,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "", previewUrl: "", dimensions: [1600, 1600], elapsed_ms: 1, header },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

const WFPC2_IN_RGB = { r: ["/m16/673nmos.fits"], g: ["/m16/656nmos.fits"], b: ["/m16/502nmos.fits"] };
const WFPC2_FILES = [
  loadedFile("/m16/673nmos.fits", { TELESCOP: "HST", INSTRUME: "WFPC2", FILTNAM1: "F673N" }),
  loadedFile("/m16/656nmos.fits", { TELESCOP: "HST", INSTRUME: "WFPC2", FILTNAM1: "F656N" }),
  loadedFile("/m16/502nmos.fits", { TELESCOP: "HST", INSTRUME: "WFPC2", FILTNAM1: "F502N" }),
];

function filled(files: Record<string, string[]>): FrequencyBin[] {
  return binsWith(files).filter((b) => b.files.length > 0);
}

const SII_HA = { sii: ["/s.fits"], ha: ["/h.fits"] };
const HA_OIII = { ha: ["/h.fits"], oiii: ["/o.fits"] };
const RGB = { r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"] };
const M16 = { sii: ["/673nmos.fits"], ha: ["/656nmos.fits"], oiii: ["/502nmos.fits"] };

interface PresetButton {
  label: string;
  disabled: boolean;
  title: string | null;
  inner: string;
}

function decode(text: string): string {
  return text.replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&amp;/g, "&");
}

function presetButtons(html: string): PresetButton[] {
  return [...html.matchAll(/<button([^>]*)>(.*?)<\/button>/g)]
    .map(([, attrs, inner]) => {
      const label = /<div class="font-semibold">([^<]*)<\/div>/.exec(inner)?.[1];
      const title = /title="([^"]*)"/.exec(attrs)?.[1];
      return label === undefined ? null : { label, disabled: /\sdisabled=""/.test(attrs), title: title === undefined ? null : decode(title), inner };
    })
    .filter((b): b is PresetButton => b !== null);
}

function renderBlend(files: Record<string, string[]>, loaded: ProcessedFile[] = []): PresetButton[] {
  const state: WizardState = { ...INITIAL_STATE, bins: binsWith(files) };
  return presetButtons(renderToStaticMarkup(createElement(BlendStep, {
    state,
    files: loaded,
    filterDetections: [],
    onLevelMatchChange: vi.fn(),
    onLevelScales: vi.fn(),
    onWeightsChange: vi.fn(),
    onCompositeReady: vi.fn(),
    onCompositeOp: vi.fn(),
  })));
}

function button(buttons: PresetButton[], label: string): PresetButton {
  const found = buttons.find((b) => b.label === label);
  if (!found) throw new Error(`no ${label} button`);
  return found;
}

describe("Blend preset availability", () => {
  it("with SII+Hα, SHO is disabled with the reason and HOO maps SII→R, Hα→G+B", () => {
    const bins = filled(SII_HA);

    expect(presetPlan(BLEND_PRESETS.sho, bins)).toEqual({ available: false, reason: "needs a third channel: SII and Hα cannot fill B" });
    expect(presetPlan(BLEND_PRESETS.rgb, bins)).toEqual({ available: false, reason: "needs a third channel: SII and Hα cannot fill B" });
    const hoo = presetPlan(BLEND_PRESETS.hoo, bins);
    expect(hoo.available && hoo.mapping).toBe("SII→R, Hα→G+B");
    expect(hoo.available && hoo.weights).toEqual([
      { channelId: "sii", r: 1, g: 0, b: 0 },
      { channelId: "ha", r: 0, g: 0.5, b: 0.5 },
    ]);
  });

  it("with Hα+OIII, SHO is disabled and HOO applies its own rows", () => {
    const bins = filled(HA_OIII);

    expect(presetPlan(BLEND_PRESETS.sho, bins)).toEqual({ available: false, reason: "needs a third channel: Hα and OIII cannot fill B" });
    expect(presetPlan(BLEND_PRESETS.rgb, bins).available).toBe(false);
    const hoo = presetPlan(BLEND_PRESETS.hoo, bins);
    expect(hoo.available && hoo.mapping).toBe("Hα→R, OIII→G+B");
  });

  it("with R/G/B, HOO names B as unused and SHO becomes the identity", () => {
    const bins = filled(RGB);

    const hoo = presetPlan(BLEND_PRESETS.hoo, bins);
    expect(hoo.available && hoo.mapping).toBe("R→R, G→G+B, B unused");
    const sho = presetPlan(BLEND_PRESETS.sho, bins);
    expect(sho.available && sho.mapping).toBe("R→R, G→G, B→B");
  });

  it("names the narrowband rows SHO applies to the M16 trio", () => {
    const sho = presetPlan(BLEND_PRESETS.sho, filled(M16));

    expect(sho.available && sho.mapping).toBe("SII→R, Hα→G, OIII→B");
  });

  it("does not call the luminance channel unused, since LRGB uses it after Blend", () => {
    const rgb = presetPlan(BLEND_PRESETS.rgb, filled({ ...RGB, l: ["/l.fits"] }));

    expect(rgb.available && rgb.mapping).toBe("R→R, G→G, B→B");
  });

  it("asks for a second channel when only one spectral channel sits next to L", () => {
    expect(presetPlan(BLEND_PRESETS.hoo, filled({ ha: ["/h.fits"], l: ["/l.fits"] }))).toEqual({
      available: false,
      reason: "needs a second channel: Hα cannot fill G and B",
    });
  });

  it("gives the same weights as resolvePresetWeights for every preset and channel set", () => {
    const channelSets: Record<string, string[]>[] = [SII_HA, HA_OIII, RGB, M16, { ...RGB, l: ["/l.fits"] }, { ha: ["/h.fits"], l: ["/l.fits"] }];
    for (const files of channelSets) {
      const bins = filled(files);
      for (const preset of Object.values(BLEND_PRESETS)) {
        const plan = presetPlan(preset, bins);
        expect(plan.available ? plan.weights : null).toEqual(resolvePresetWeights(preset, bins));
      }
    }
  });

  it("puts the weights a preset applies in its title", () => {
    const dynamic = presetPlan(BLEND_PRESETS.dynamic_hoo, filled(HA_OIII));
    const foraxx = presetPlan(BLEND_PRESETS.foraxx, filled(M16));

    expect(dynamic.available && dynamic.title).toBe("Fixed weights: Hα 0.90 R + 0.40 G; OIII 0.10 R + 0.60 G + 1.00 B");
    expect(foraxx.available && foraxx.title).toBe("Fixed weights: SII 0.80 R + 0.20 G; Hα 0.20 R + 0.70 G + 0.10 B; OIII 0.10 G + 0.90 B");
  });

  it("describes Dynamic HOO and Foraxx as fixed-weight blends", () => {
    expect(BLEND_PRESETS.dynamic_hoo.desc).toBe("Hα/OIII blend with fixed weights");
    expect(BLEND_PRESETS.foraxx.desc).toBe("SII/Hα/OIII blend with fixed weights");
    for (const preset of [BLEND_PRESETS.dynamic_hoo, BLEND_PRESETS.foraxx]) {
      expect(preset.desc).not.toMatch(/per[- ]pixel|popular|dynamic/i);
    }
  });
});

describe("Blend preset filter names", () => {
  const wfpc2State: WizardState = { ...INITIAL_STATE, bins: binsWith(WFPC2_IN_RGB) };

  it("names the filters an Auto Mapped WFPC2 trio holds in R, G and B instead of the bin labels", () => {
    const names = presetBinNames(wfpc2State, WFPC2_FILES);
    expect(names).toEqual({ r: "F673N", g: "F656N", b: "F502N" });
    const bins = filled(WFPC2_IN_RGB);

    const sho = presetPlan(BLEND_PRESETS.sho, bins, names);
    expect(sho.available && sho.mapping).toBe("F673N→R, F656N→G, F502N→B");
    const hoo = presetPlan(BLEND_PRESETS.hoo, bins, names);
    expect(hoo.available && hoo.mapping).toBe("F673N→R, F656N→G+B, F502N unused");
    expect(hoo.available && hoo.title).toBe("Fixed weights: F673N 1.00 R; F656N 0.50 G + 0.50 B");
    expect(presetPlan(BLEND_PRESETS.sho, bins, names)).toMatchObject({ weights: resolvePresetWeights(BLEND_PRESETS.sho, bins) });
  });

  it("names the filters in the reason a two-filter set cannot fill B", () => {
    const pair = { r: ["/m16/673nmos.fits"], g: ["/m16/656nmos.fits"] };
    const names = presetBinNames({ ...INITIAL_STATE, bins: binsWith(pair) }, WFPC2_FILES);
    expect(presetPlan(BLEND_PRESETS.sho, filled(pair), names)).toEqual({
      available: false,
      reason: "needs a third channel: F673N and F656N cannot fill B",
    });
  });

  it("keeps the bin label when the files of a colour bin share no filter or name none", () => {
    const mixed = { r: ["/m16/673nmos.fits", "/m16/656nmos.fits"], g: ["/rgb/green.fits"], b: ["/m16/502nmos.fits", "/m16/502_2.fits"] };
    const files = [
      ...WFPC2_FILES,
      loadedFile("/rgb/green.fits", { FILTER: "Green" }),
      loadedFile("/m16/502_2.fits", { FILTNAM1: "F502N" }),
    ];
    const state: WizardState = { ...INITIAL_STATE, bins: binsWith(mixed), excludedFiles: {} };
    expect(presetBinNames(state, files)).toEqual({ r: "R", g: "G", b: "F502N" });
    expect(presetBinNames({ ...state, excludedFiles: { r: ["/m16/656nmos.fits"] } }, files)).toMatchObject({ r: "F673N" });
  });

  it("keeps the line labels of the narrowband bins and leaves the mapping alone without loaded headers", () => {
    const m16 = { sii: ["/m16/673nmos.fits"], ha: ["/m16/656nmos.fits"], oiii: ["/m16/502nmos.fits"] };
    expect(presetBinNames({ ...INITIAL_STATE, bins: binsWith(m16) }, WFPC2_FILES)).toEqual({ sii: "SII", ha: "Hα", oiii: "OIII" });
    expect(presetBinNames(wfpc2State, [])).toEqual({ r: "R", g: "G", b: "B" });
  });

  it("shows the filter names under the presets of an Auto Mapped WFPC2 trio", () => {
    const buttons = renderBlend(WFPC2_IN_RGB, WFPC2_FILES);

    expect(button(buttons, "SHO (Hubble)").inner).toContain('data-testid="blend-preset-mapping">F673N→R, F656N→G, F502N→B<');
    expect(button(buttons, "HOO").inner).toContain('data-testid="blend-preset-mapping">F673N→R, F656N→G+B, F502N unused<');
    expect(button(buttons, "RGB").title).toBe("Fixed weights: F673N 1.00 R; F656N 1.00 G; F502N 1.00 B");
  });
});

describe("Blend preset buttons", () => {
  it("disables SHO and RGB on SII+Hα with the reason as the title and shows the HOO mapping", () => {
    const buttons = renderBlend(SII_HA);

    expect(button(buttons, "SHO (Hubble)")).toMatchObject({ disabled: true, title: "needs a third channel: SII and Hα cannot fill B" });
    expect(button(buttons, "RGB")).toMatchObject({ disabled: true, title: "needs a third channel: SII and Hα cannot fill B" });
    const hoo = button(buttons, "HOO");
    expect(hoo.disabled).toBe(false);
    expect(hoo.inner).toContain('data-testid="blend-preset-mapping">SII→R, Hα→G+B<');
    expect(button(buttons, "SHO (Hubble)").inner).not.toContain('data-testid="blend-preset-mapping"');
  });

  it("enables every preset on R/G/B and shows the mapping under each", () => {
    const buttons = renderBlend(RGB);

    for (const label of ["RGB", "SHO (Hubble)", "Hubble Legacy", "HOO", "Dynamic HOO", "Foraxx"]) {
      expect(button(buttons, label).disabled).toBe(false);
    }
    expect(button(buttons, "HOO").inner).toContain('data-testid="blend-preset-mapping">R→R, G→G+B, B unused<');
    expect(button(buttons, "SHO (Hubble)").inner).toContain('data-testid="blend-preset-mapping">R→R, G→G, B→B<');
    expect(button(buttons, "Dynamic HOO").title).toBe("Fixed weights: R 0.90 R + 0.40 G; G 0.10 R + 0.60 G + 1.00 B");
  });

  it("keeps the wavelength buttons enabled whatever the presets can fill", () => {
    const buttons = renderBlend(SII_HA);

    expect(button(buttons, "Auto (λ)").disabled).toBe(false);
    expect(button(buttons, "Balanced (λ)").disabled).toBe(false);
  });
});
