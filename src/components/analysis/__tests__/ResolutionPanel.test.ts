import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import ResolutionPanel, { ResolutionPanelInner, type ResolutionPanelProps } from "../ResolutionPanel";
import { imagePsfStore, resolutionSettingsStore, type ImagePsfState } from "../../../hooks/useResolutionSettings";
import { DEFAULT_RESOLUTION_SETTINGS } from "../../../utils/resolutionText";
import type { ImagePsfFwhm, ResolutionSettings, SeeingCard } from "../../../shared/types/resolution";

const { live, CardStub } = vi.hoisted(() => ({
  live: {
    settings: null as unknown as ResolutionSettings,
    image: null as unknown as ImagePsfState,
    headerCards: [] as readonly SeeingCard[],
  },
  CardStub: function CardStub() {
    return "[card]";
  },
}));

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

vi.mock("../../../hooks/useResolutionSettings", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../hooks/useResolutionSettings")>()),
  useResolutionSettings: () => live.settings,
  useImagePsf: () => live.image,
  useHeaderSeeing: () => live.headerCards,
}));

vi.mock("../ResolutionCard", () => ({ default: CardStub }));

const PSF = {
  fwhm_px: 3.5057,
  fwhm_arcsec: 0.9184,
  fwhm_px_scatter: 0.1314,
  n_stars: 108,
  model: "M",
  undersampled: false,
  half_light_radius_arcsec: 0.52,
  pixel_scale_arcsec: 0.262,
  header_values: [],
  warnings: [],
  notes: ["n one"],
} as unknown as ImagePsfFwhm;

const INTRO =
  "Whether a source is resolved depends on the PSF, on its signal-to-noise and on the sampling. These settings feed every &quot;Resolved or not&quot; card.";

const FIELD_LABELS = [
  "PSF source",
  "PSF FWHM (arcsec)",
  "Redshift (optional)",
  "H0 (km/s/Mpc)",
  "Omega_m",
  "PSF FWHM uncertainty (fraction)",
  "Resolved from (resolution elements)",
  "Sampling limit (px per FWHM)",
  "Scale of interest (kpc)",
  "Source FWHM (arcsec)",
  "Half-light radius (arcsec)",
  "Signal-to-noise",
];

const FIELD_RANGES: Record<string, [number, number]> = {
  psfFwhmArcsec: [0.001, 60],
  redshift: [0.0001, 20],
  h0: [20, 200],
  omegaM: [0.01, 0.99],
  eps: [0, 0.5],
  qResolved: [0.1, 100],
  minPixelsPerFwhm: [0.5, 10],
  scaleOfInterestKpc: [0.001, 1000],
};

const PROPS: ResolutionPanelProps = { filePath: "a.fits" };

let update: ReturnType<typeof vi.fn>;
let reset: ReturnType<typeof vi.fn>;
let measure: ReturnType<typeof vi.fn>;

function markupOf(props: ResolutionPanelProps = PROPS): string {
  return renderToStaticMarkup(createElement(ResolutionPanel, props));
}

function rowText(markup: string, label: string): string | null {
  const match = new RegExp(`>${label}</span><span[^>]*>([^<]*)<`).exec(markup);
  return match === null ? null : match[1];
}

function tagWith(markup: string, tag: string, attribute: string): string {
  const match = new RegExp(`<${tag}[^>]*${attribute}[^>]*>`).exec(markup);
  if (match === null) throw new Error(`no <${tag}> with ${attribute}`);
  return match[0];
}

function render(props: ResolutionPanelProps = PROPS) {
  return elementsOf(hooks.render(ResolutionPanelInner, props));
}

function byProp(tree: ReturnType<typeof render>, name: string, value: unknown) {
  const found = tree.find((node) => node.props[name] === value);
  if (!found) throw new Error(`no element with ${name}=${String(value)}`);
  return found;
}

const field = (tree: ReturnType<typeof render>, name: string) => byProp(tree, "data-field", name);
const valueChange = (tree: ReturnType<typeof render>, name: string, value: number | null) =>
  (field(tree, name).props.onValueChange as (value: number | null) => void)(value);
const cardProps = (tree: ReturnType<typeof render>) => {
  const card = tree.find((node) => node.type === CardStub);
  if (!card) throw new Error("no card element");
  return card.props;
};

beforeEach(() => {
  live.settings = { ...DEFAULT_RESOLUTION_SETTINGS };
  live.image = { psf: null, loading: false, error: null };
  live.headerCards = [];
  update = vi.spyOn(resolutionSettingsStore, "update").mockImplementation(() => undefined) as unknown as ReturnType<typeof vi.fn>;
  reset = vi.spyOn(resolutionSettingsStore, "reset").mockImplementation(() => undefined) as unknown as ReturnType<typeof vi.fn>;
  measure = vi.spyOn(imagePsfStore, "measure").mockResolvedValue(undefined) as unknown as ReturnType<typeof vi.fn>;
  hooks.mount();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ResolutionPanel markup", () => {
  it("shows the four blocks with the stored values", () => {
    const markup = markupOf();
    expect(markup).toContain("data-resolution-panel");
    for (const legend of ["PSF", "Physical scale", "Thresholds", "Check a size"]) expect(markup).toMatch(new RegExp(`<legend[^>]*>${legend}</legend>`));
    expect(markup).toContain(INTRO);
    for (const label of FIELD_LABELS) expect(markup).toContain(label);
    expect(markup).toContain('value="67.66"');
    expect(markup).toContain('value="0.30966"');
    expect(markup).toContain('value="0.03"');
    expect(markup).toContain("Type a redshift to see sizes in kpc.");
    expect(rowText(markup, "PSF used")).toBe("Measure the PSF of this image in the Resolution section.");
    expect(markup).toContain("[card]");
  });

  it("shows the measured PSF with its estimator and the header cards", () => {
    live.image = { psf: PSF, loading: false, error: null };
    let markup = markupOf();
    expect(rowText(markup, "Image PSF")).toBe("3.51 px = 0.918 arcsec ± 0.13 px, 108 stars");
    expect(markup).toContain(">M<");
    expect(rowText(markup, "Header")).toBe("no SEEING, PSF_FWHM or PSFFWHM card");
    expect(markup).toContain("notes (1)");
    expect(markup).toContain("n one");
    expect(rowText(markup, "PSF used")).toBe("0.918 arcsec, 108 stars of this image (M)");
    expect(markup).not.toContain(">undersampled<");

    live.image = { psf: { ...PSF, header_values: [{ key: "SEEING", value_arcsec: 0.95 }] }, loading: false, error: null };
    expect(rowText(markupOf(), "Header")).toBe("SEEING 0.95 arcsec");

    live.image = { psf: { ...PSF, undersampled: true, warnings: ["PSF sampled by fewer than 2 pixels"] }, loading: false, error: null };
    markup = markupOf();
    expect(markup).toContain(">undersampled<");
    expect(markup).toContain("PSF sampled by fewer than 2 pixels");
  });

  it("the typed field is enabled only for the typed source", () => {
    expect(tagWith(markupOf(), "input", 'data-field="psfFwhmArcsec"')).toContain('disabled=""');
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, psfSource: "typed", psfFwhmArcsec: 0.8 };
    const markup = markupOf();
    const input = tagWith(markup, "input", 'data-field="psfFwhmArcsec"');
    expect(input).not.toContain('disabled=""');
    expect(input).toContain('value="0.8"');
    expect(rowText(markup, "PSF used")).toBe("0.8 arcsec, typed value");
  });

  it("a failed measurement is shown as an error", () => {
    live.image = { psf: null, loading: false, error: "No star was detected in this image" };
    expect(markupOf()).toMatch(/role="alert"[^>]*>No star was detected in this image</);
  });

  it("without a file nothing can be measured or checked", () => {
    const markup = markupOf({ filePath: null });
    expect(tagWith(markup, "button", 'data-action="measure-psf"')).toContain('disabled=""');
    expect(markup).toContain("Open an image to check a size.");
    expect(markup).not.toContain("[card]");
    expect(rowText(markup, "Header")).toBeNull();
  });

  it("header cards are listed and used before any star measurement", () => {
    live.settings = { ...DEFAULT_RESOLUTION_SETTINGS, psfSource: "header" };
    live.headerCards = [{ key: "SEEING", value_arcsec: 0.95 }];
    let markup = markupOf();
    expect(rowText(markup, "Header")).toBe("SEEING 0.95 arcsec");
    expect(rowText(markup, "PSF used")).toBe("0.95 arcsec, header card SEEING");

    live.headerCards = [];
    markup = markupOf();
    expect(rowText(markup, "Header")).toBe("no SEEING, PSF_FWHM or PSFFWHM card");
    expect(rowText(markup, "PSF used")).toBe("This file has no SEEING, PSF_FWHM or PSFFWHM card: choose another PSF source.");
  });
});

describe("ResolutionPanel interactions", () => {
  it("the PSF source select writes the setting", () => {
    const select = field(render(), "psfSource");
    expect(select.type).toBe("select");
    (select.props.onChange as (event: unknown) => void)({ target: { value: "typed" } });
    expect(update).toHaveBeenCalledTimes(1);
    expect(update).toHaveBeenCalledWith({ psfSource: "typed" });
  });

  it("number fields write their setting and an emptied redshift writes null", () => {
    const tree = render();
    valueChange(tree, "redshift", 1.5);
    expect(update).toHaveBeenLastCalledWith({ redshift: 1.5 });
    valueChange(tree, "redshift", null);
    expect(update).toHaveBeenLastCalledWith({ redshift: null });
    valueChange(tree, "psfFwhmArcsec", 0.7);
    expect(update).toHaveBeenLastCalledWith({ psfFwhmArcsec: 0.7 });
    valueChange(tree, "psfFwhmArcsec", null);
    expect(update).toHaveBeenLastCalledWith({ psfFwhmArcsec: null });
    const written: [string, number][] = [
      ["h0", 70],
      ["omegaM", 0.3],
      ["eps", 0.05],
      ["qResolved", 2],
      ["minPixelsPerFwhm", 2.5],
      ["scaleOfInterestKpc", 0.5],
    ];
    for (const [name, value] of written) {
      valueChange(tree, name, value);
      expect(update).toHaveBeenLastCalledWith({ [name]: value });
    }
    expect(update).toHaveBeenCalledTimes(10);
    for (const [name, [min, max]] of Object.entries(FIELD_RANGES)) {
      expect([name, field(tree, name).props.min, field(tree, name).props.max]).toEqual([name, min, max]);
    }
    expect(field(tree, "redshift").props.optional).toBe(true);
    expect(field(tree, "psfFwhmArcsec").props.optional).toBe(true);
    expect(field(tree, "h0").props.optional).toBeFalsy();
    expect(field(tree, "sourceFwhm").props.min).toBe(0.001);
    expect(field(tree, "sourceR50").props.min).toBe(0.001);
  });

  it("measure and reset call the stores", () => {
    const tree = render();
    (byProp(tree, "data-action", "measure-psf").props.onClick as () => void)();
    expect(measure).toHaveBeenCalledTimes(1);
    expect(measure).toHaveBeenCalledWith("a.fits");

    live.image = { psf: null, loading: true, error: null };
    const button = byProp(render(), "data-action", "measure-psf");
    expect(textOf(button)).toBe("Measuring the PSF…");
    expect(button.props.disabled).toBe(true);

    (byProp(tree, "data-action", "reset-resolution").props.onClick as () => void)();
    expect(reset).toHaveBeenCalledTimes(1);
  });

  it("typed sizes go to the card", () => {
    const tree = render();
    expect(cardProps(tree)).toEqual({ filePath: "a.fits", fwhmArcsec: null, halfLightRadiusArcsec: null, snr: null, context: "manual" });
    valueChange(tree, "sourceFwhm", 1.2);
    valueChange(tree, "sourceSnr", 50);
    expect(update).not.toHaveBeenCalled();
    expect(cardProps(render())).toEqual({ filePath: "a.fits", fwhmArcsec: 1.2, halfLightRadiusArcsec: null, snr: 50, context: "manual" });
  });

  it("the typed sizes survive a change of file", () => {
    const tree = render();
    valueChange(tree, "sourceFwhm", 1.2);
    valueChange(tree, "sourceSnr", 50);
    render();
    expect(cardProps(render({ ...PROPS, filePath: "b.fits" }))).toEqual({
      filePath: "b.fits",
      fwhmArcsec: 1.2,
      halfLightRadiusArcsec: null,
      snr: 50,
      context: "manual",
    });
  });
});
