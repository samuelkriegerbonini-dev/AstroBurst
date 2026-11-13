import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import SpectrumSourcePicker from "../SpectrumSourcePicker";
import { spectrumSelectionStore, type ActiveSpectrumDocument, type SpectrumSelection } from "../../../hooks/useActiveSpectrumDocument";
import type { SpectrumSeriesOption, SpectrumSeriesSet, SpectrumTableOption } from "../../../services/spectrum1d";
import type { Spectrum1d, TsoSeriesPick } from "../../../shared/types/spectrum1d";

type Props = Record<string, unknown> & { children?: ReactNode };
type PickerProps = { active: ActiveSpectrumDocument };

const pickerFunction = (SpectrumSourcePicker as unknown as { type: (props: PickerProps) => ReactElement | null }).type;

const X1D_PATH = "C:/data/jw01345-o063_t001_nirspec_g395m-f290lp_x1d.fits";
const SOSS_PATH = "C:/data/jw07073-o004_t001_niriss_clear-gr700xd-substrip256_x1d.fits";
const X1DINTS_PATH = "C:/data/jw03375-o001_t001_nirspec_f290lp-g395h-s1600a1-sub2048_x1dints.fits";
const ARRAY_PATH = "C:/data/hd37795.fits";
const NO_SOLUTION_ERROR =
  "CTYPE1 missing, no IRAF WAT1 wavelength label and no wavelength CUNIT1: the array has no wavelength solution";
const LOG_LAMBDA_ERROR = "CTYPE1 = 'WAVE-LOG': log-lambda dispersion is not supported";
const NO_TIMES_WARNING = "this file has no usable integration times: the abscissa is the integration number";
const UNIT_HINT = "use this when the header gives no unit for the wavelength axis; it does not change files that state their unit";

const keyOf = (path: string) => `f|${path}`;

function spectrumOf(extra: Partial<Spectrum1d> = {}): Spectrum1d {
  return {
    source: { kind: "x1d", path: X1D_PATH, hdu: 1, extver: 1 },
    axis: {
      kind: "wave",
      ctype: "WAVE",
      unit: "um",
      header_unit: "um",
      header_scale: 1,
      values: [1.6, 1.7, 1.8, 1.9, 2.0],
      crval: null,
      cdelt: null,
      crpix: null,
      rest_wavelength_um: null,
      rest_frequency_hz: null,
      specsys: null,
      velosys: null,
      notes: [],
    },
    wavelength_frame: "vacuum",
    precision: "f64",
    flux: [1, 2, 3, 4, 5],
    err: [0.1, 0.1, 0.1, 0.1, 0.1],
    dq: [0, 1, 0, 4, 1],
    dq_table: "jwst",
    flux_unit: "Jy",
    n_rows: 1,
    cards: { INSTRUME: "NIRSPEC", GRATING: "G235H", FILTER: "F170LP", TARGNAME: "NGC 7469", SRCTYPE: "POINT" },
    notes: [],
    wavelength_unit_source: "file",
    ...extra,
  } as Spectrum1d;
}

function arrayOf(extra: Partial<Spectrum1d> = {}): Spectrum1d {
  return spectrumOf({ source: { kind: "wcs1d", path: ARRAY_PATH, hdu: 0 }, cards: { OBJECT: "HD 37795" }, err: null, dq: null, ...extra });
}

function activeOf(extra: Partial<ActiveSpectrumDocument> = {}): ActiveSpectrumDocument {
  return {
    content: "spectrum",
    fileKey: keyOf(X1D_PATH),
    path: X1D_PATH,
    hdu: 1,
    viewKey: `1|${X1D_PATH}`,
    doc: { loading: false, spectrum: null, error: null },
    document: null,
    kind: { kind: "x1d", hdu: 1, n_tables: 1 },
    selection: { kind: "table", hdu: 1, wavelengthUnit: null },
    request: null,
    options: { tables: null, series: null, loading: false, error: null },
    ...extra,
  };
}

function loaded(spectrum: Spectrum1d): Pick<ActiveSpectrumDocument, "doc" | "document"> {
  return { doc: { loading: false, spectrum, error: null }, document: { source: { kind: "file", hdu: null }, spectrum } };
}

function failed(error: string): Pick<ActiveSpectrumDocument, "doc" | "document"> {
  return { doc: { loading: false, spectrum: null, error }, document: null };
}

function table(hdu: number, label: string): SpectrumTableOption {
  return {
    hdu,
    extver: hdu - 1,
    n_rows: 1,
    n_channels: 2048,
    sporder: hdu - 1,
    detector: "NIS",
    slit_name: null,
    source_id: null,
    source_name: null,
    int_num: null,
    srctype: "POINT",
    label,
  };
}

function seriesOption(detector: string, nIntegrations: number, label = detector): SpectrumSeriesOption {
  return { key: { detector, sporder: -1 }, label, n_integrations: nIntegrations, n_channels: 1271, flux_unit: "Jy" };
}

function seriesSet(series: SpectrumSeriesOption[], extra: Partial<SpectrumSeriesSet> = {}): SpectrumSeriesSet {
  return { set_id: "set-1", series, notes: [], warnings: [], ...extra };
}

const SERIES = seriesSet([seriesOption("NRS1", 8), seriesOption("NRS2", 8)]);

function seriesSelection(detector: string, pick: TsoSeriesPick): SpectrumSelection {
  return { kind: "series", detector, sporder: -1, pick };
}

function seriesActive(selection: SpectrumSelection, set: SpectrumSeriesSet = SERIES): ActiveSpectrumDocument {
  return activeOf({
    fileKey: keyOf(X1DINTS_PATH),
    path: X1DINTS_PATH,
    hdu: null,
    kind: { kind: "x1dints", layout: "rows", n_series: set.series.length, n_integrations: 8, n_channels: 1271 },
    selection,
    options: { tables: null, series: set, loading: false, error: null },
  });
}

function arrayActive(extra: Partial<ActiveSpectrumDocument> = {}): ActiveSpectrumDocument {
  return activeOf({
    fileKey: keyOf(ARRAY_PATH),
    path: ARRAY_PATH,
    hdu: 0,
    kind: { kind: "wcs1d", hdu: 0 },
    selection: { kind: "table", hdu: 0, wavelengthUnit: null },
    ...extra,
  });
}

function rendered(active: ActiveSpectrumDocument): { tree: ReactNode; html: string } {
  const tree = hooks.render(pickerFunction, { active }) as ReactNode;
  return { tree, html: tree === null ? "" : renderToStaticMarkup(tree as ReactElement) };
}

function labelled(tree: ReactNode, label: string): ReactElement<Props> | undefined {
  return elementsOf<Props>(tree).find((element) => element.props["aria-label"] === label);
}

function marked(tree: ReactNode, attribute: string): ReactElement<Props>[] {
  return elementsOf<Props>(tree).filter((element) => element.props[attribute] !== undefined);
}

function optionsOf(select: ReactElement<Props> | undefined): { value: string; text: string }[] {
  return elementsOf<Props>(select?.props.children)
    .filter((element) => element.type === "option")
    .map((element) => ({ value: String(element.props.value), text: textOf(element.props.children) }));
}

function selectedText(select: ReactElement<Props> | undefined): string | undefined {
  return optionsOf(select).find((option) => option.value === String(select?.props.value))?.text;
}

function choose(select: ReactElement<Props> | undefined, text: string): void {
  const option = optionsOf(select).find((o) => o.text === text);
  expect(option, `option ${text}`).toBeDefined();
  (select?.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: option!.value } });
}

function field(active: ActiveSpectrumDocument, label: string): Props & { value: string } {
  const element = labelled(rendered(active).tree, label);
  expect(element, `field ${label}`).toBeDefined();
  return element!.props as Props & { value: string };
}

function commit(active: ActiveSpectrumDocument, label: string, text: string): void {
  (field(active, label).onChange as (text: string) => void)(text);
  (field(active, label).onBlur as () => void)();
}

function fieldInvalid(active: ActiveSpectrumDocument, label: string): boolean {
  const { tree } = rendered(active);
  const input = labelled(tree, label);
  return input !== undefined && renderToStaticMarkup(input as ReactElement).includes('aria-invalid="true"');
}

beforeEach(() => {
  hooks.mount();
  spectrumSelectionStore.reset();
});

describe("SpectrumSourcePicker", () => {
  it("shows only the details for a file with one spectrum", () => {
    const { tree, html } = rendered(activeOf(loaded(spectrumOf())));
    const details = marked(tree, "data-spectrum-details");
    expect(details).toHaveLength(1);
    expect(textOf(details[0])).toBe(
      "NIRSPEC G235H F170LP, target NGC 7469, source type POINT, errors: FLUX_ERROR column, DQ: jwst flags, 2 channels excluded",
    );
    expect(html).toContain("data-spectrum-source");
    expect(html).not.toContain("data-spectrum-tables");
    expect(html).not.toContain("data-spectrum-series");
    expect(html).not.toContain("data-wavelength-unit");

    const emptyList = rendered(activeOf({ ...loaded(spectrumOf()), options: { tables: [], series: null, loading: false, error: null } }));
    expect(emptyList.html).not.toContain("data-spectrum-tables");
    const oneTable = rendered(activeOf({ options: { tables: [table(1, "order 1")], series: null, loading: false, error: null } }));
    expect(oneTable.html).not.toContain("data-spectrum-tables");

    expect(marked(rendered(activeOf()).tree, "data-spectrum-details")).toHaveLength(0);
    expect(rendered(activeOf({ path: null, fileKey: null, kind: null, selection: null })).tree).toBeNull();
  });

  it("lists the tables of an x1d by their label and says which one is shown", () => {
    const tables = [table(2, "order 1"), table(3, "order 2"), table(4, "order 3")];
    const sossActive = (hdu: number) =>
      activeOf({
        fileKey: keyOf(SOSS_PATH),
        path: SOSS_PATH,
        hdu,
        kind: { kind: "x1d", hdu: 2, n_tables: 3 },
        selection: { kind: "table", hdu, wavelengthUnit: null },
        options: { tables, series: null, loading: false, error: null },
      });
    const { tree, html } = rendered(sossActive(2));
    const select = labelled(tree, "Spectrum table");
    expect(optionsOf(select).map((o) => o.text)).toEqual(["order 1 (HDU 2)", "order 2 (HDU 3)", "order 3 (HDU 4)"]);
    expect(selectedText(select)).toBe("order 1 (HDU 2)");
    expect(html).toContain("data-spectrum-tables");
    expect(textOf(marked(tree, "data-spectrum-tables")[0])).toContain("table 1 of 3");
    expect(textOf(marked(tree, "data-spectrum-tables")[0])).toContain("Table");

    (select?.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "4" } });
    expect(spectrumSelectionStore.get(keyOf(SOSS_PATH))).toEqual({ kind: "table", hdu: 4, wavelengthUnit: null });

    const last = rendered(sossActive(4)).tree;
    expect(textOf(marked(last, "data-spectrum-tables")[0])).toContain("table 3 of 3");
    expect(selectedText(labelled(last, "Spectrum table"))).toBe("order 3 (HDU 4)");
  });

  it("falls back to the HDU of a table and the detector of a series when the backend sends no label", () => {
    const tables = [table(1, ""), table(2, "")];
    const { tree } = rendered(activeOf({ kind: { kind: "x1d", hdu: 1, n_tables: 2 }, options: { tables, series: null, loading: false, error: null } }));
    expect(optionsOf(labelled(tree, "Spectrum table")).map((o) => o.text)).toEqual(["HDU 1", "HDU 2"]);
    const unlabelled = seriesSet([seriesOption("NRS1", 8, ""), seriesOption("NRS2", 8, "")]);
    const series = rendered(seriesActive(seriesSelection("NRS1", { kind: "median" }), unlabelled)).tree;
    expect(optionsOf(labelled(series, "Spectrum series")).map((o) => o.text)).toEqual(["NRS1 (8 integrations)", "NRS2 (8 integrations)"]);
  });

  it("offers the series of a time-series file and the three kinds of pick", () => {
    const { tree, html } = rendered(seriesActive(seriesSelection("NRS1", { kind: "median" })));
    const series = labelled(tree, "Spectrum series");
    expect(optionsOf(series).map((o) => o.text)).toEqual(["NRS1 (8 integrations)", "NRS2 (8 integrations)"]);
    expect(selectedText(series)).toBe("NRS1 (8 integrations)");
    const pick = labelled(tree, "Spectrum of the series");
    expect(optionsOf(pick).map((o) => o.text)).toEqual(["median of all integrations", "mean of integrations", "one integration"]);
    expect(selectedText(pick)).toBe("median of all integrations");
    expect(html).toContain("data-spectrum-series");
    expect(textOf(marked(tree, "data-spectrum-series")[0])).toContain("Series");
    expect(labelled(tree, "Integration")).toBeUndefined();
    expect(labelled(tree, "First integration")).toBeUndefined();
    expect(labelled(tree, "Last integration")).toBeUndefined();
    expect(html).not.toContain("data-spectrum-tables");
    expect(html).not.toContain("data-wavelength-unit");
  });

  it("stores one integration counted from zero and shows it counted from one", () => {
    const fileKey = keyOf(X1DINTS_PATH);
    choose(labelled(rendered(seriesActive(seriesSelection("NRS1", { kind: "median" }))).tree, "Spectrum of the series"), "one integration");
    expect(spectrumSelectionStore.get(fileKey)).toEqual(seriesSelection("NRS1", { kind: "integration", index: 0 }));

    const first = seriesActive(seriesSelection("NRS1", { kind: "integration", index: 0 }));
    expect(field(first, "Integration").value).toBe("1");
    expect(selectedText(labelled(rendered(first).tree, "Spectrum of the series"))).toBe("one integration");
    commit(first, "Integration", "5");
    expect(spectrumSelectionStore.get(fileKey)).toEqual(seriesSelection("NRS1", { kind: "integration", index: 4 }));

    const fifth = seriesActive(seriesSelection("NRS1", { kind: "integration", index: 4 }));
    hooks.mount();
    expect(field(fifth, "Integration").value).toBe("5");
    for (const text of ["0", "9", "2.5"]) {
      commit(fifth, "Integration", text);
      expect(spectrumSelectionStore.get(fileKey)).toEqual(seriesSelection("NRS1", { kind: "integration", index: 4 }));
      expect(fieldInvalid(fifth, "Integration")).toBe(true);
    }
    (field(fifth, "Integration").onChange as (text: string) => void)("8");
    (field(fifth, "Integration").onKeyDown as (e: { key: string }) => void)({ key: "Enter" });
    expect(spectrumSelectionStore.get(fileKey)).toEqual(seriesSelection("NRS1", { kind: "integration", index: 7 }));
  });

  it("stores a mean over a range of integrations", () => {
    const fileKey = keyOf(X1DINTS_PATH);
    choose(labelled(rendered(seriesActive(seriesSelection("NRS1", { kind: "median" }))).tree, "Spectrum of the series"), "mean of integrations");
    expect(spectrumSelectionStore.get(fileKey)).toEqual(seriesSelection("NRS1", { kind: "mean", first: 0, last: 7 }));

    const whole = seriesActive(seriesSelection("NRS1", { kind: "mean", first: 0, last: 7 }));
    expect(field(whole, "First integration").value).toBe("1");
    expect(field(whole, "Last integration").value).toBe("8");
    commit(whole, "First integration", "3");
    expect(spectrumSelectionStore.get(fileKey)).toEqual(seriesSelection("NRS1", { kind: "mean", first: 2, last: 7 }));
    hooks.mount();
    commit(seriesActive(seriesSelection("NRS1", { kind: "mean", first: 2, last: 7 })), "Last integration", "6");
    expect(spectrumSelectionStore.get(fileKey)).toEqual(seriesSelection("NRS1", { kind: "mean", first: 2, last: 5 }));
    hooks.mount();
    commit(seriesActive(seriesSelection("NRS1", { kind: "mean", first: 2, last: 5 })), "First integration", "8");
    expect(spectrumSelectionStore.get(fileKey)).toEqual(seriesSelection("NRS1", { kind: "mean", first: 5, last: 5 }));
    hooks.mount();
    commit(seriesActive(seriesSelection("NRS1", { kind: "mean", first: 3, last: 5 })), "Last integration", "2");
    expect(spectrumSelectionStore.get(fileKey)).toEqual(seriesSelection("NRS1", { kind: "mean", first: 3, last: 3 }));

    hooks.mount();
    const single = seriesSet([seriesOption("NRS1", 1)]);
    choose(labelled(rendered(seriesActive(seriesSelection("NRS1", { kind: "median" }), single)).tree, "Spectrum of the series"), "mean of integrations");
    expect(spectrumSelectionStore.get(fileKey)).toEqual(seriesSelection("NRS1", { kind: "mean", first: 0, last: 0 }));
    const one = seriesActive(seriesSelection("NRS1", { kind: "mean", first: 0, last: 0 }), single);
    expect(field(one, "First integration").value).toBe("1");
    expect(field(one, "Last integration").value).toBe("1");
  });

  it("resets the numbers when the series changes", () => {
    const set = seriesSet([seriesOption("NRS1", 8), seriesOption("NRS2", 4)]);
    const { tree } = rendered(seriesActive(seriesSelection("NRS1", { kind: "integration", index: 6 }), set));
    choose(labelled(tree, "Spectrum series"), "NRS2 (4 integrations)");
    expect(spectrumSelectionStore.get(keyOf(X1DINTS_PATH))).toEqual({
      kind: "series",
      detector: "NRS2",
      sporder: -1,
      pick: { kind: "integration", index: 0 },
    });
    hooks.mount();
    const mean = rendered(seriesActive(seriesSelection("NRS1", { kind: "mean", first: 2, last: 5 }), set)).tree;
    choose(labelled(mean, "Spectrum series"), "NRS2 (4 integrations)");
    expect(spectrumSelectionStore.get(keyOf(X1DINTS_PATH))).toEqual(seriesSelection("NRS2", { kind: "mean", first: 0, last: 3 }));
  });

  it("shows the warnings of the series set", () => {
    const set = seriesSet([seriesOption("NIS", 5, "NIS, order 1")], { warnings: [NO_TIMES_WARNING], notes: ["note one", "note two"] });
    const { tree, html } = rendered(seriesActive(seriesSelection("NIS", { kind: "median" }), set));
    const warnings = marked(tree, "data-series-warning");
    expect(warnings).toHaveLength(1);
    expect(textOf(warnings[0])).toBe(NO_TIMES_WARNING);
    expect(html).toContain("notes (2)");
    expect(html).toContain("note one");
    expect(html).toContain("note two");
  });

  it("says when the spectra of a file are being listed or could not be", () => {
    const listing = rendered(activeOf({ kind: { kind: "x1d", hdu: 1, n_tables: 3 }, options: { tables: null, series: null, loading: true, error: null } }));
    expect(listing.html).toContain("listing the spectra of this file");
    const failure = rendered(activeOf({ kind: { kind: "x1d", hdu: 1, n_tables: 3 }, options: { tables: null, series: null, loading: false, error: "x" } }));
    expect(failure.html).toContain("could not list the spectra of this file: x");
    expect(failure.html).not.toContain("listing the spectra of this file");
  });

  it("offers a wavelength unit when the file gave none, when the read found no wavelength solution and while one is set", () => {
    const fileKey = keyOf(ARRAY_PATH);
    const assumed = rendered(arrayActive(loaded(arrayOf({ wavelength_unit_source: "assumed" })))).tree;
    const select = labelled(assumed, "Wavelength unit of the file");
    expect(optionsOf(select).map((o) => o.text)).toEqual(["as read", "Angstrom", "nm", "um"]);
    expect(selectedText(select)).toBe("as read");
    const unitBlock = marked(assumed, "data-wavelength-unit");
    expect(unitBlock).toHaveLength(1);
    expect(textOf(unitBlock[0])).toContain("Wavelength unit of the file");
    expect(textOf(unitBlock[0])).toContain(UNIT_HINT);
    choose(select, "nm");
    expect(spectrumSelectionStore.get(fileKey)).toEqual({ kind: "table", hdu: 0, wavelengthUnit: "nm" });

    const overridden = arrayActive({ ...loaded(arrayOf({ wavelength_unit_source: "file" })), selection: { kind: "table", hdu: 0, wavelengthUnit: "nm" } });
    const shown = labelled(rendered(overridden).tree, "Wavelength unit of the file");
    expect(selectedText(shown)).toBe("nm");
    choose(shown, "as read");
    expect(spectrumSelectionStore.get(fileKey)).toEqual({ kind: "table", hdu: 0, wavelengthUnit: null });

    expect(labelled(rendered(arrayActive(failed(NO_SOLUTION_ERROR))).tree, "Wavelength unit of the file")).toBeDefined();
    expect(rendered(arrayActive(failed(LOG_LAMBDA_ERROR))).html).not.toContain("data-wavelength-unit");
    expect(rendered(arrayActive(loaded(arrayOf()))).html).not.toContain("data-wavelength-unit");
    expect(rendered(arrayActive()).html).not.toContain("data-wavelength-unit");
    expect(rendered(activeOf(loaded(spectrumOf({ wavelength_unit_source: "assumed" })))).html).not.toContain("data-wavelength-unit");
    expect(rendered(activeOf(failed(NO_SOLUTION_ERROR))).html).not.toContain("data-wavelength-unit");
  });
});
