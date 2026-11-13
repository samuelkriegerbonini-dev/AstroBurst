import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { Spectrum1d } from "../../../shared/types/spectrum1d";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

import { hooks } from "../../../test/hookRuntime";
import SpectrumViewControls from "../SpectrumViewControls";
import { spectrumViewStore } from "../../../hooks/useSpectrumViewStore";
import { buildSpectrumPlotModel, type SpectrumPlotModel } from "../../../utils/spectrumPlotModel";
import type { LineMeasurement } from "../../../shared/types/spectral";

type Props = Record<string, unknown>;
type ControlsProps = { viewKey: string; spectrum: Spectrum1d | null; model: SpectrumPlotModel | null; disabled?: boolean };

const controlsFunction = (SpectrumViewControls as unknown as { type: (props: ControlsProps) => ReactElement | null }).type;

const KEY = "0|C:/data/ha_bess.fits";
const VALUES = Array.from({ length: 40 }, (_, i) => 0.6555 + i * 1e-4);
const MORTON = "air and vacuum wavelengths are converted with Morton (2000), for the plot and for line velocities";

function documentOf(extra: Partial<Spectrum1d> = {}): Spectrum1d {
  return {
    source: { kind: "wcs1d", path: "C:/data/ha_bess.fits", hdu: 0 },
    axis: {
      kind: "awav",
      ctype: "WAVE",
      unit: "um",
      header_unit: "Angstrom",
      header_scale: 1e-4,
      values: VALUES,
      crval: VALUES[0],
      cdelt: 1e-4,
      crpix: 1,
      rest_wavelength_um: null,
      rest_frequency_hz: null,
      specsys: null,
      velosys: null,
      notes: [],
    },
    wavelength_frame: "air",
    precision: "f64",
    flux: VALUES.map((_, i) => 1 + Math.exp(-((i - 20) ** 2) / 8)),
    err: VALUES.map(() => 0.05),
    dq: null,
    dq_table: "unknown",
    flux_unit: "erg/s/cm2/A",
    n_rows: VALUES.length,
    cards: {},
    notes: ["spectrum file, 1D array in HDU 0: air wavelengths"],
    ...extra,
  };
}

function rendered(props: Partial<ControlsProps> = {}): { tree: ReactNode; html: string } {
  const tree = hooks.render(controlsFunction, { viewKey: KEY, spectrum: documentOf(), model: null, ...props }) as ReactNode;
  return { tree, html: tree === null ? "" : renderToStaticMarkup(tree as ReactElement) };
}

function findProps(node: ReactNode, match: (props: Props) => boolean): Props | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findProps(child, match);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Props>(node)) return null;
  if (match(node.props)) return node.props;
  return findProps(node.props.children as ReactNode, match);
}

beforeEach(() => {
  hooks.mount();
  spectrumViewStore.reset();
});

describe("SpectrumViewControls frame row", () => {
  it("shows the frame of a document and writes the choice to the store", () => {
    const first = rendered();
    expect(first.html).toContain("data-document-frame");
    expect(first.html).toContain(">Frame<");
    expect(first.html).toContain('data-frame-choice="air"');
    expect(first.html).toContain('data-frame-choice="vacuum"');
    expect(first.html).toMatch(/data-frame-choice="air" aria-pressed="true"/);
    expect(first.html).toMatch(/data-frame-choice="vacuum" aria-pressed="false"/);
    expect(first.html).toContain("air wavelengths, as the file reads");
    expect(first.html).toContain(MORTON);

    const vacuum = findProps(first.tree, (p) => p["data-frame-choice"] === "vacuum");
    expect(vacuum).not.toBeNull();
    (vacuum!.onClick as () => void)();
    expect(spectrumViewStore.get(KEY).frameChoice).toBe("vacuum");

    const second = rendered();
    expect(second.html).toMatch(/data-frame-choice="vacuum" aria-pressed="true"/);
    expect(second.html).toContain("vacuum wavelengths, set here (the file reads as air)");
    expect(second.html).not.toContain("Morton");
  });

  it("locks the frame while a measurement runs", () => {
    const { tree } = rendered({ disabled: true });
    expect(findProps(tree, (p) => p["data-frame-choice"] === "air")?.disabled).toBe(true);
    expect(findProps(tree, (p) => p["data-frame-choice"] === "vacuum")?.disabled).toBe(true);
    const { html } = rendered({ disabled: true });
    expect(html.match(/disabled=""/g)).toHaveLength(2);
    const enabled = rendered();
    expect(enabled.html).not.toContain('disabled=""');
  });

  it("offers no frame for an axis that is not a wavelength, and renders nothing without a document", () => {
    spectrumViewStore.patch(KEY, { frameChoice: "air" });
    const notWavelength = rendered({ spectrum: documentOf({ wavelength_frame: "not_wavelength" }) });
    expect(notWavelength.html).toContain("data-document-frame");
    expect(notWavelength.html).toContain("the spectral axis is not a wavelength: no air or vacuum frame applies");
    expect(notWavelength.html).not.toContain("data-frame-choice");
    expect(notWavelength.html).not.toContain("Morton");
    expect(rendered({ spectrum: null }).html).toBe("");
  });

  it("says where the frame comes from, and nothing for an assumed frame", () => {
    const named = rendered({ spectrum: documentOf({ notes: ["CTYPE1 'AWAV': air wavelengths"] }) });
    expect(named.html).toContain("data-frame-origin");
    expect(named.html).toContain('<span data-frame-origin="true" class="text-zinc-600">frame from: CTYPE1 &#x27;AWAV&#x27;</span>');
    const frameLineAt = named.html.indexOf("air wavelengths, as the file reads");
    const originAt = named.html.indexOf("frame from:");
    expect(frameLineAt).toBeGreaterThan(-1);
    expect(originAt).toBeGreaterThan(frameLineAt);
    expect(named.html.indexOf(MORTON)).toBeGreaterThan(originAt);
    expect(findProps(named.tree, (p) => "data-frame-origin" in p)?.children).toEqual(["frame from: ", "CTYPE1 'AWAV'"]);

    const assumed = rendered({
      spectrum: documentOf({
        notes: [
          "CTYPE1 'LINEAR' with no JWST, HST or Roman TELESCOP and no AIRORVAC or VACUUM card naming the frame: air wavelengths assumed, the ground-based default",
        ],
      }),
    });
    expect(assumed.html).toContain("air wavelengths, assumed (no card of the file names the frame)");
    expect(assumed.html).not.toContain("data-frame-origin");
    expect(rendered({ spectrum: null, model: plotModelOf() }).html).not.toContain("data-frame-origin");
  });
});

function plotModelOf(extra: Partial<SpectrumPlotModel> = {}): SpectrumPlotModel {
  const model = buildSpectrumPlotModel({
    series: VALUES.map((_, i) => 1 + i),
    dqExcluded: null,
    axis: { formatted: null, mode: "wavelength_vac", correction: "none", correctionResult: null, fallbackWavelengths: null, fallbackUnit: null },
    errors: null,
    readoutErr: null,
    fluxUnit: "DN",
    color: "#a855f7",
    yLabel: null,
    errorsWidenRange: true,
  });
  return { ...model, ...extra };
}

const OVERLAY = { result: { z0: 15, z1: 25 } as LineMeasurement, measurementAxis: VALUES };

describe("SpectrumViewControls plot view", () => {
  it("toggles log y and the limits in the store", () => {
    const model = plotModelOf();
    expect(model.n).toBe(40);
    const first = rendered({ model });
    const log = findProps(first.tree, (p) => "data-view-log" in p);
    expect(log?.["aria-pressed"]).toBe(false);
    (log!.onClick as () => void)();
    expect(spectrumViewStore.get(KEY).logY).toBe(true);
    expect(findProps(rendered({ model }).tree, (p) => "data-view-log" in p)?.["aria-pressed"]).toBe(true);

    expect(first.html).toContain(">Limits<");
    expect(findProps(first.tree, (p) => p["data-view-limits"] === "full")?.["aria-pressed"]).toBe(true);
    const robust = findProps(first.tree, (p) => p["data-view-limits"] === "robust");
    expect(robust?.["aria-pressed"]).toBe(false);
    expect(robust?.title).toBe("y limits from the 1st to the 99th percentile of the visible samples");
    expect(findProps(first.tree, (p) => p["data-view-limits"] === "full")?.title).toBe("y limits from the smallest to the largest visible sample");
    (robust!.onClick as () => void)();
    expect(spectrumViewStore.get(KEY).yLimits).toBe("robust");
    const next = rendered({ model });
    expect(findProps(next.tree, (p) => p["data-view-limits"] === "robust")?.["aria-pressed"]).toBe(true);
    expect(findProps(next.tree, (p) => p["data-view-limits"] === "full")?.["aria-pressed"]).toBe(false);
  });

  it("offers reset zoom only while zoomed", () => {
    const model = plotModelOf();
    expect(rendered({ model }).html).not.toContain("data-view-reset-zoom");
    spectrumViewStore.patch(KEY, { xDomain: [0.656, 0.657] });
    const zoomed = rendered({ model });
    expect(zoomed.html).toContain(">reset zoom<");
    const reset = findProps(zoomed.tree, (p) => "data-view-reset-zoom" in p);
    expect(reset?.title).toBe("Show the whole axis (the wheel zooms, Shift and drag pans)");
    (reset!.onClick as () => void)();
    expect(spectrumViewStore.get(KEY).xDomain).toBeNull();
  });

  it("says what the limits hide", () => {
    expect(rendered({ model: plotModelOf({ offScale: 2 }) }).html).toContain("2 samples off scale");
    expect(rendered({ model: plotModelOf({ offScale: 1 }) }).html).toContain("1 sample off scale");
    expect(rendered({ model: plotModelOf() }).html).not.toContain("off scale");
    const far = "y range set by 2 samples far from the rest: try robust limits";
    expect(rendered({ model: plotModelOf({ farOutliers: 2 }) }).html).toContain(far);
    expect(rendered({ model: plotModelOf({ farOutliers: 1 }) }).html).toContain("y range set by 1 sample far from the rest: try robust limits");
    spectrumViewStore.patch(KEY, { yLimits: "robust" });
    expect(rendered({ model: plotModelOf({ farOutliers: 2 }) }).html).not.toContain("far from the rest");

    const linearNote = "log y needs a sample above 0: shown linear";
    expect(rendered({ model: plotModelOf() }).html).not.toContain(linearNote);
    spectrumViewStore.patch(KEY, { logY: true });
    expect(rendered({ model: plotModelOf({ logY: false }) }).html).toContain(linearNote);
    expect(rendered({ model: plotModelOf({ logY: true }) }).html).not.toContain(linearNote);
    expect(rendered({ model: plotModelOf({ logY: true, logHidden: 3 }) }).html).toContain("3 samples at or below 0 are not drawn on a log axis");
    expect(rendered({ model: plotModelOf({ logY: true, logHidden: 1 }) }).html).toContain("1 sample at or below 0 is not drawn on a log axis");

    const overlayNote = "the measured line overlay is not drawn on a log axis";
    expect(rendered({ model: plotModelOf({ logY: true }) }).html).not.toContain(overlayNote);
    spectrumViewStore.setOverlay(KEY, OVERLAY);
    expect(rendered({ model: plotModelOf({ logY: true }) }).html).toContain(overlayNote);
    expect(rendered({ model: plotModelOf({ logY: false }) }).html).not.toContain(overlayNote);
  });

  it("shows the plot toggles for a cube spectrum and nothing without a plot", () => {
    const cube = rendered({ spectrum: null, model: plotModelOf() });
    expect(cube.html).toContain("data-view-log");
    expect(cube.html).toContain('data-view-limits="full"');
    expect(cube.html).toContain('data-view-limits="robust"');
    expect(cube.html).toContain(">log y<");
    expect(cube.html).not.toContain("data-document-frame");
    expect(rendered({ spectrum: null, model: null }).html).toBe("");
    const empty = plotModelOf({ n: 0, series: [] });
    expect(rendered({ spectrum: null, model: empty }).html).toBe("");
    const docOnly = rendered({ model: empty });
    expect(docOnly.html).toContain("data-document-frame");
    expect(docOnly.html).not.toContain("data-view-log");
  });
});

function selectOf(tree: ReactNode, hook: string): Props | null {
  return findProps(tree, (p) => hook in p);
}

function optionsOf(select: Props): { value: unknown; text: unknown }[] {
  return (select.children as ReactElement<Props>[]).map((option) => ({ value: option.props.value, text: option.props.children }));
}

function velocityDocument(extra: Partial<Spectrum1d> = {}): Spectrum1d {
  const base = documentOf();
  return documentOf({
    axis: { ...base.axis, kind: "vopt", ctype: "VOPT", unit: "km/s", header_unit: "km/s", header_scale: 1, values: VALUES.map((_, i) => -200 + 10 * i) },
    wavelength_frame: "not_wavelength",
    flux_unit: "Jy",
    ...extra,
  });
}

describe("SpectrumViewControls units", () => {
  it("offers the wavelength unit on a wavelength axis and stores the choice", () => {
    const model = plotModelOf({ wavelengthUnit: "um" });
    const first = rendered({ model });
    expect(first.html).toContain("data-view-units");
    expect(first.html).toContain(">Units<");
    const select = selectOf(first.tree, "data-view-wavelength-unit");
    expect(select?.["aria-label"]).toBe("Wavelength unit");
    expect(optionsOf(select!).map((o) => o.text)).toEqual(["μm", "nm", "Å"]);
    expect(optionsOf(select!).map((o) => o.value)).toEqual(["um", "nm", "A"]);
    expect(select?.value).toBe("um");
    (select!.onChange as (e: unknown) => void)({ target: { value: "A" } });
    expect(spectrumViewStore.get(KEY).wavelengthUnit).toBe("A");
    expect(selectOf(rendered({ model: plotModelOf({ wavelengthUnit: null }) }).tree, "data-view-wavelength-unit")).toBeNull();
    expect(selectOf(rendered({ spectrum: null, model }).tree, "data-view-wavelength-unit")).not.toBeNull();
  });

  it("offers the flux units the file can be shown in", () => {
    const model = plotModelOf({ wavelengthUnit: "um" });
    const first = rendered({ model });
    const select = selectOf(first.tree, "data-view-flux-unit");
    expect(select?.["aria-label"]).toBe("Flux unit");
    const options = optionsOf(select!);
    expect(options).toHaveLength(7);
    expect(options[0]).toEqual({ value: "native", text: "file unit (erg/s/cm2/A)" });
    expect(select?.value).toBe("native");
    (select!.onChange as (e: unknown) => void)({ target: { value: "ab" } });
    expect(spectrumViewStore.get(KEY).fluxUnit).toBe("ab");
    expect(selectOf(rendered({ model, spectrum: documentOf({ flux_unit: "MJy/sr" }) }).tree, "data-view-flux-unit")).toBeNull();
    expect(selectOf(rendered({ model, spectrum: null }).tree, "data-view-flux-unit")).toBeNull();
    const velocity = optionsOf(selectOf(rendered({ model, spectrum: velocityDocument() }).tree, "data-view-flux-unit")!);
    expect(velocity).toHaveLength(5);
    expect(velocity.map((o) => o.text)).not.toContain("erg s-1 cm-2 Angstrom-1");
    spectrumViewStore.patch(KEY, { fluxUnit: "flam_cgs" });
    expect(selectOf(rendered({ model, spectrum: velocityDocument() }).tree, "data-view-flux-unit")?.value).toBe("native");
  });

  it("says what the unit choice changed", () => {
    const noted = plotModelOf({ fluxState: { choice: "native", scale: 1, invertY: false, note: "Jy is not available for flux in DN/s" } });
    expect(rendered({ model: noted }).html).toContain("flux shown in the file unit: Jy is not available for flux in DN/s");
    expect(rendered({ model: plotModelOf() }).html).not.toContain("flux shown in the file unit");

    const overlayNote = "the measured line overlay is drawn only in the file unit or a multiple of it";
    const unscaled = plotModelOf({ fluxState: { choice: "uJy", scale: null, invertY: false, note: null } });
    const scaled = plotModelOf({ fluxState: { choice: "mJy", scale: 1000, invertY: false, note: null } });
    expect(rendered({ model: unscaled }).html).not.toContain(overlayNote);
    spectrumViewStore.setOverlay(KEY, OVERLAY);
    expect(rendered({ model: unscaled }).html).toContain(overlayNote);
    expect(rendered({ model: scaled }).html).not.toContain(overlayNote);

    spectrumViewStore.patch(KEY, { logY: true });
    const magnitudes = rendered({ model: plotModelOf({ logY: false, fluxState: { choice: "ab", scale: null, invertY: true, note: null } }) }).html;
    expect(magnitudes).toContain("log y is not used for AB magnitudes");
    expect(magnitudes).not.toContain("log y needs a sample above 0: shown linear");
    expect(rendered({ model: plotModelOf({ logY: false }) }).html).not.toContain("log y is not used for AB magnitudes");
  });

  it("leaves the unit selects usable while a measurement runs", () => {
    const { tree } = rendered({ model: plotModelOf({ wavelengthUnit: "um" }), disabled: true });
    const wavelength = selectOf(tree, "data-view-wavelength-unit");
    const flux = selectOf(tree, "data-view-flux-unit");
    expect(wavelength).not.toBeNull();
    expect(flux).not.toBeNull();
    expect(wavelength?.disabled).toBeFalsy();
    expect(flux?.disabled).toBeFalsy();
  });
});

const BIN_NOTE = "binned by 4: 10 bins; mean of the unmasked samples, errors assume independent channels";
const UNBINNED_NOTE = "the line range, the continuum windows and Measure line use the unbinned channels";

function binField(tree: ReactNode): Props | null {
  return findProps(tree, (p) => "data-view-bin" in p);
}

describe("SpectrumViewControls binning", () => {
  it("offers a bin factor for a spectrum file and stores a whole number", () => {
    const model = plotModelOf();
    const field = () => binField(rendered({ model }).tree)!;
    const type = (text: string) => (field().onChange as (text: string) => void)(text);
    const press = (key: string) => (field().onKeyDown as (e: unknown) => void)({ key });
    const blur = () => (field().onBlur as () => void)();

    const first = rendered({ model });
    expect(first.html).toContain(">Bin<");
    expect(binField(first.tree)?.["aria-label"]).toBe("Bin factor");
    expect(field().value).toBe("1");

    type("4");
    expect(field().value).toBe("4");
    expect(spectrumViewStore.get(KEY).binFactor).toBe(1);
    blur();
    expect(spectrumViewStore.get(KEY).binFactor).toBe(4);
    expect(field().value).toBe("4");

    type("8");
    press("Enter");
    expect(spectrumViewStore.get(KEY).binFactor).toBe(8);

    for (const draft of ["0", "41", "2.5"]) {
      type(draft);
      blur();
      press("Enter");
      expect(spectrumViewStore.get(KEY).binFactor).toBe(8);
      expect(field().value).toBe(draft);
      expect(rendered({ model }).html).toContain('aria-invalid="true"');
    }

    type("16");
    press("Escape");
    expect(spectrumViewStore.get(KEY).binFactor).toBe(8);
    expect(field().value).toBe("8");
    expect(rendered({ model }).html).not.toContain("aria-invalid");

    expect(binField(rendered({ spectrum: null, model }).tree)).toBeNull();
    expect(rendered({ spectrum: null, model }).html).not.toContain("data-view-bin");
    expect(binField(rendered({ model: plotModelOf({ n: 1, series: [1] }) }).tree)).toBeNull();
  });

  it("says what binning did and what it leaves alone", () => {
    const bins = {
      factor: 4,
      first: Array.from({ length: 10 }, (_, k) => 4 * k),
      last: Array.from({ length: 10 }, (_, k) => 4 * k + 3),
      nUsed: Array.from({ length: 10 }, () => 4),
      x: Array.from({ length: 10 }, (_, k) => 4 * k + 1.5),
      y: Array.from({ length: 10 }, (_, k) => 4 * k + 2.5),
      err: null,
      note: BIN_NOTE,
    };
    const binned = rendered({ model: plotModelOf({ bins }) }).html;
    expect(binned).toContain(BIN_NOTE);
    expect(binned).toContain(UNBINNED_NOTE);
    const plain = rendered({ model: plotModelOf() }).html;
    expect(plain).not.toContain("binned by");
    expect(plain).not.toContain(UNBINNED_NOTE);
  });
});
