import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const dialog = vi.hoisted(() => ({ open: vi.fn(async (): Promise<string | null> => "C:/lists/targets.csv") }));
const depths = vi.hoisted(() => ({ current: {} as Record<string, unknown> }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: dialog.open }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: async () => "53.1 -28.1 a" }));
vi.mock("../../../hooks/useDepthStore", () => ({
  depthStore: { subscribe: () => () => {}, latest: (path: string) => depths.current[path] ?? null },
}));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { forcedPhotometryStore } from "../../../hooks/useForcedPhotometryStore";
import { parseForcedPositions, type ForcedFields } from "../../../utils/forcedPhotometryRequest";
import { DecimalNumberField, Toggle } from "../../ui";

const { default: ForcedPhotometryInputs, MAX_SHOWN_POSITION_ERRORS, BandNoiseOffer } = await import("../ForcedPhotometryInputs");
const { noiseOfferText } = await import("../forcedPhotometryInputsModel");

type Props = Record<string, unknown> & { children?: ReactNode };
type Element = ReactElement<Props>;
type Inputs = (props: Record<string, unknown>) => unknown;

const inner = (ForcedPhotometryInputs as unknown as { type: Inputs }).type;

const FILES = [
  { name: "brick-g.fits", path: "C:/d/brick-g.fits" },
  { name: "brick-r.fits", path: "C:/d/brick-r.fits" },
  { name: "cut.fits", path: "C:/d/cut.fits#hdu=0&plane=1" },
];

function props(extra: Record<string, unknown> = {}) {
  const fields = forcedPhotometryStore.getSnapshot().fields;
  return { fields, positions: parseForcedPositions(fields.positionsText), files: FILES, ...extra };
}

const fields = (): ForcedFields => forcedPhotometryStore.getSnapshot().fields;
const show = (extra: Record<string, unknown> = {}) => elementsOf<Props>(hooks.render(inner, props(extra))) as Element[];
const byData = (shown: Element[], name: string, value: unknown) => shown.filter((e) => e.props[name] === value);
const field = (shown: Element[], name: string) => byData(shown, "data-field", name)[0];
const action = (shown: Element[], name: string) => byData(shown, "data-action", name);
const change = (element: Element, value: unknown) =>
  (element.props.onChange as (event: { target: Record<string, unknown> }) => void)({ target: { value, checked: value } });
const click = (element: Element) => (element.props.onClick as () => void)();
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

const POSITIONS_HINT =
  "Decimal values are ICRS degrees; sexagesimal RA is hours unless written with d. Each position is measured where it is given: nothing is recentred.";
const BANDS_EMPTY =
  "Open the image of each band in Files, then add it here. Every band keeps its own pixel grid: nothing is resampled.";
const BANDS_HINT =
  "Label names the band in the table. Dataset names the filter set, for example DECAM, when the header does not. Noise factor is f_corr of the Depth section, measured with the same aperture.";
const AB_CAPTION =
  "Used only when the header of this band gives no calibration: flux in Jy = image unit x 10^((8.90 - zero point) / 2.5).";
const VEGA_CAPTION =
  "Used only when the header of this band gives no calibration. A Vega zero point gives no Jy and no AB magnitude: the band stays out of the colours and of the SED.";
const PER_SECOND_TITLE =
  "The zero point is for one count per second: the backend divides the counts by the exposure time of the header";

const R_PATH = "C:/d/brick-r.fits";
const DEPTH = { radius_arcsec: 1.0, radius_px: 3.8167938931297707, f_corr: 1.647 };

const bandRow = () => elementsOf<Props>(byData(show(), "data-forced-band", R_PATH)[0]) as Element[];
const typeValue = (element: Element, value: number | null) => (element.props.onValueChange as (value: number | null) => void)(value);
const offer = (extra: Record<string, unknown> = {}) =>
  hooks.render(BandNoiseOffer as unknown as Inputs, { path: R_PATH, label: "r", fCorr: null, radius: 1, unit: "arcsec", ...extra }) as Element | null;
const offerElements = (extra: Record<string, unknown> = {}) => elementsOf<Props>(offer(extra)) as Element[];
const offerTexts = (shown: Element[]) =>
  shown.filter((e) => e.type !== "button" && typeof e.props.children === "string").map((e) => e.props.children as string);

describe("ForcedPhotometryInputs", () => {
  beforeEach(() => {
    hooks.mount();
    forcedPhotometryStore.reset();
    dialog.open.mockReset().mockResolvedValue("C:/lists/targets.csv");
    depths.current = {};
  });

  afterEach(() => {
    hooks.mount();
    forcedPhotometryStore.reset();
  });

  it("shows the four blocks with their titles and fields", () => {
    const markup = renderToStaticMarkup(createElement(ForcedPhotometryInputs, props()));
    const blocks = [...markup.matchAll(/data-forced-block="([a-z]+)"/g)].map((m) => m[1]);
    expect(blocks).toEqual(["positions", "bands", "aperture", "options"]);
    for (const title of ["Positions", "Bands (0 of 12)", "Aperture", "Options"]) expect(markup).toContain(`>${title}<`);
    for (const name of ["positions", "radius", "radiusUnit", "annulusInner", "annulusOuter", "growthCorrection", "limitSigma", "gain"]) {
      expect(markup).toContain(`data-field="${name}"`);
    }
    expect(markup).not.toContain('data-field="referenceRadiusArcsec"');
    expect(markup).not.toContain(BANDS_EMPTY);
    expect(markup).toContain(POSITIONS_HINT);
    expect(markup).not.toContain('type="number"');
    const added = action(show(), "add-band").map((e) => textOf(e.props.children));
    expect(added).toEqual(["+ brick-g.fits", "+ brick-r.fits", "+ cut.fits #hdu=0&plane=1"]);
    expect(markup).toContain(">+ cut.fits #hdu=0&amp;plane=1<");
    expect(renderToStaticMarkup(createElement(ForcedPhotometryInputs, props({ files: [] })))).toContain(BANDS_EMPTY);
  });

  it("typing positions updates the store and the count", () => {
    const count = () => byData(show(), "data-forced-position-count", true).map((e) => textOf(e.props.children))[0];
    change(field(show(), "positions"), "53.1 -28.1 a\n53.2 -28.2 b");
    expect(fields().positionsText).toBe("53.1 -28.1 a\n53.2 -28.2 b");
    expect(count()).toBe("2 positions");
    expect(show().some((e) => e.props.children === POSITIONS_HINT)).toBe(false);
    change(field(show(), "positions"), "53.1 -28.1 a");
    expect(count()).toBe("1 position");
    change(field(show(), "positions"), "53.1 -28.1 a\nhello\n53.2 -28.2 b");
    expect(count()).toBe("2 positions");
    const cut = parseForcedPositions("53.1 -28.1 a\n53.2 -28.2 b");
    const shown = elementsOf<Props>(hooks.render(inner, props({ positions: { ...cut, total: 7 } }))) as Element[];
    expect(byData(shown, "data-forced-position-count", true).map((e) => textOf(e.props.children))).toEqual(["2 positions of 7 read"]);
  });

  it("shows the first five problems of the list and how many more there are", () => {
    forcedPhotometryStore.setFields({ positionsText: Array.from({ length: 7 }, () => "x").join("\n") });
    const box = byData(show(), "data-forced-position-errors", true)[0];
    expect(box).toBeDefined();
    const lines = elementsOf<Props>(box.props.children).map((e) => textOf(e.props.children));
    expect(lines).toHaveLength(MAX_SHOWN_POSITION_ERRORS + 1);
    expect(lines.slice(0, 5).map((line) => line.slice(0, 7))).toEqual(["line 1:", "line 2:", "line 3:", "line 4:", "line 5:"]);
    expect(lines[5]).toBe("and 2 more");
    forcedPhotometryStore.setFields({ positionsText: "53.1 -28.1 a" });
    expect(byData(show(), "data-forced-position-errors", true)).toHaveLength(0);
  });

  it("adds one band, all bands, and removes one", () => {
    const addButton = (text: string) => action(show(), "add-band").find((e) => textOf(e.props.children) === text)!;
    click(addButton("+ brick-r.fits"));
    expect(fields().bands.map((b) => b.path)).toEqual(["C:/d/brick-r.fits"]);
    const shown = show();
    expect(shown.some((e) => e.props.children === "Bands (1 of 12)")).toBe(true);
    expect(byData(shown, "data-forced-band", "C:/d/brick-r.fits")).toHaveLength(1);
    expect(action(shown, "add-band").map((e) => textOf(e.props.children))).toEqual(["+ brick-g.fits", "+ cut.fits #hdu=0&plane=1"]);
    expect(shown.some((e) => e.props.children === BANDS_HINT)).toBe(true);
    click(action(show(), "add-all-bands")[0]);
    expect(fields().bands.map((b) => b.path)).toEqual(["C:/d/brick-r.fits", "C:/d/brick-g.fits", "C:/d/cut.fits#hdu=0&plane=1"]);
    expect(action(show(), "add-all-bands")).toHaveLength(0);
    click(show().find((e) => e.props["aria-label"] === "Remove brick-g.fits")!);
    expect(fields().bands.map((b) => b.path)).toEqual(["C:/d/brick-r.fits", "C:/d/cut.fits#hdu=0&plane=1"]);

    forcedPhotometryStore.reset();
    for (let i = 0; i < 12; i++) forcedPhotometryStore.addBand({ name: `b${i}.fits`, path: `C:/d/b${i}.fits` });
    const full = show();
    expect(full.some((e) => e.props.children === "Bands (12 of 12)")).toBe(true);
    const adds = [...action(full, "add-band"), ...action(full, "add-all-bands")];
    expect(adds).toHaveLength(4);
    expect(adds.every((e) => e.props.disabled === true)).toBe(true);
  });

  it("edits the label, the dataset and the noise factor of a band", () => {
    forcedPhotometryStore.addBand(FILES[1]);
    const row = () => elementsOf<Props>(byData(show(), "data-forced-band", "C:/d/brick-r.fits")[0]) as Element[];
    change(field(row(), "label"), "r");
    change(field(row(), "dataset"), "DECAM");
    const factor = field(row(), "fCorr");
    expect(factor.type).toBe(DecimalNumberField);
    (factor.props.onValueChange as (value: number | null) => void)(1.65);
    expect(fields().bands).toEqual([{ path: "C:/d/brick-r.fits", fileName: "brick-r.fits", label: "r", dataset: "DECAM", fCorr: 1.65 }]);
    (field(row(), "fCorr").props.onValueChange as (value: number | null) => void)(null);
    expect(fields().bands[0].fCorr).toBeNull();
  });

  it("switches the aperture unit and shows the reference radius only with the correction", () => {
    change(field(show(), "radiusUnit"), "px");
    expect(fields().radiusUnit).toBe("px");
    expect(field(show(), "referenceRadiusArcsec")).toBeUndefined();
    change(field(show(), "growthCorrection"), true);
    expect(fields().growthCorrection).toBe(true);
    const reference = field(show(), "referenceRadiusArcsec");
    expect(reference).toBeDefined();
    expect(reference.props.placeholder).toBe("3.5");
  });

  it("loads a list from a file", async () => {
    click(action(show(), "load-positions")[0]);
    await vi.waitFor(() => expect(fields().positionsText).toBe("53.1 -28.1 a"));
    expect(dialog.open).toHaveBeenCalledWith({
      multiple: false,
      filters: [{ name: "Position lists", extensions: ["csv", "txt", "tsv", "dat"] }],
      title: "Load a position list",
    });

    forcedPhotometryStore.setFields({ positionsText: "53.2 -28.2 b" });
    dialog.open.mockClear().mockResolvedValue(null);
    click(action(show(), "load-positions")[0]);
    await vi.waitFor(() => expect(dialog.open).toHaveBeenCalledTimes(1));
    await flush();
    expect(fields().positionsText).toBe("53.2 -28.2 b");

    dialog.open.mockRejectedValue(new Error("dialog closed by the system"));
    click(action(show(), "load-positions")[0]);
    await vi.waitFor(() => expect(show().some((e) => e.props.role === "alert")).toBe(true));
    const alert = show().find((e) => e.props.role === "alert");
    expect(textOf(alert?.props.children)).toBe("Could not load the list: dialog closed by the system");
    expect(fields().positionsText).toBe("53.2 -28.2 b");
  });

  it("a run in flight disables the form", () => {
    forcedPhotometryStore.addBand(FILES[0]);
    forcedPhotometryStore.setFields({ growthCorrection: true });
    const shown = show({ disabled: true });
    const controls = shown.filter(
      (e) => ["textarea", "input", "select", "button"].includes(e.type as string) || e.type === DecimalNumberField || e.type === Toggle,
    );
    expect(controls.filter((e) => e.type === "textarea")).toHaveLength(1);
    expect(controls.filter((e) => e.type === "select")).toHaveLength(2);
    expect(controls.length).toBeGreaterThan(12);
    expect(controls.filter((e) => e.props.disabled !== true).map((e) => e.props["data-field"] ?? e.props["data-action"] ?? e.type)).toEqual([]);
  });

  it("typing a zero point gives the band one, clearing removes it", () => {
    forcedPhotometryStore.addBand(FILES[1]);
    const zeroPoint = () => fields().bands[0].zeroPoint;
    const group = byData(bandRow(), "data-forced-zero-point", true)[0];
    expect(group).toBeDefined();
    const zp = field(bandRow(), "zp");
    expect(zp.type).toBe(DecimalNumberField);
    expect(zp.props.optional).toBe(true);
    expect(zp.props.min).toBe(-100);
    expect(zp.props.max).toBe(100);
    expect(zp.props.placeholder).toBe("from the header");
    expect(field(bandRow(), "zpSystem").props.disabled).toBe(true);
    expect(field(bandRow(), "zpPerSecond").props.disabled).toBe(true);
    const options = elementsOf<Props>(field(bandRow(), "zpSystem")).filter((e) => e.type === "option");
    expect(options.map((e) => [e.props.value, textOf(e.props.children)])).toEqual([
      ["ab", "AB"],
      ["vega", "Vega"],
    ]);
    const perSecond = field(bandRow(), "zpPerSecond");
    expect(perSecond.props.title).toBe(PER_SECOND_TITLE);
    expect(bandRow().some((e) => e.type === "label" && textOf(e.props.children) === "per second")).toBe(true);

    typeValue(field(bandRow(), "zp"), 25);
    expect(zeroPoint()).toEqual({ zp: 25, system: "ab", perSecond: false });
    expect(field(bandRow(), "zp").props.value).toBe(25);
    expect(field(bandRow(), "zpSystem").props.disabled).toBe(false);
    expect(field(bandRow(), "zpPerSecond").props.disabled).toBe(false);
    change(field(bandRow(), "zpSystem"), "vega");
    expect(zeroPoint()).toEqual({ zp: 25, system: "vega", perSecond: false });
    change(field(bandRow(), "zpPerSecond"), true);
    expect(zeroPoint()).toEqual({ zp: 25, system: "vega", perSecond: true });
    expect(field(bandRow(), "zpSystem").props.value).toBe("vega");
    expect(field(bandRow(), "zpPerSecond").props.checked).toBe(true);
    typeValue(field(bandRow(), "zp"), 24.5);
    expect(zeroPoint()).toEqual({ zp: 24.5, system: "vega", perSecond: true });

    typeValue(field(bandRow(), "zp"), null);
    expect(zeroPoint()).toBeNull();
    expect(field(bandRow(), "zpSystem").props.disabled).toBe(true);
    expect(field(bandRow(), "zpPerSecond").props.disabled).toBe(true);

    typeValue(field(bandRow(), "zp"), 0);
    expect(zeroPoint()).toEqual({ zp: 0, system: "ab", perSecond: false });
    expect(field(bandRow(), "zpSystem").props.disabled).toBe(false);
  });

  it("says what a typed zero point does", () => {
    forcedPhotometryStore.addBand(FILES[1]);
    const has = (text: string) => bandRow().some((e) => e.props.children === text);
    expect(has(AB_CAPTION)).toBe(false);
    expect(has(VEGA_CAPTION)).toBe(false);
    forcedPhotometryStore.updateBand(R_PATH, { zeroPoint: { zp: 25, system: "ab", perSecond: false } });
    expect(has(AB_CAPTION)).toBe(true);
    expect(has(VEGA_CAPTION)).toBe(false);
    forcedPhotometryStore.updateBand(R_PATH, { zeroPoint: { zp: 22.5, system: "vega", perSecond: true } });
    expect(has(AB_CAPTION)).toBe(false);
    expect(has(VEGA_CAPTION)).toBe(true);
    forcedPhotometryStore.updateBand(R_PATH, { zeroPoint: { zp: 0, system: "ab", perSecond: false } });
    expect(has(AB_CAPTION)).toBe(true);
    forcedPhotometryStore.updateBand(R_PATH, { zeroPoint: null });
    expect(has(AB_CAPTION)).toBe(false);
    expect(has(VEGA_CAPTION)).toBe(false);
  });

  it("offers the depth factor measured with the same aperture", () => {
    forcedPhotometryStore.addBand(FILES[1]);
    forcedPhotometryStore.setFields({ radius: 1, radiusUnit: "arcsec" });
    depths.current[R_PATH] = DEPTH;
    const shown = offerElements();
    expect(shown[0].props["data-forced-noise-offer"]).toBe("usable");
    expect(offerTexts(shown)).toEqual(["Depth section: f_corr 1.65 for this image at r = 1 arcsec."]);
    const use = action(shown, "use-depth-fcorr");
    expect(use).toHaveLength(1);
    expect(use[0].type).toBe("button");
    expect(textOf(use[0].props.children)).toBe("Use");
    expect(use[0].props["aria-label"]).toBe("Use the Depth noise factor for r");
    click(use[0]);
    expect(fields().bands[0].fCorr).toBe(1.647);
    const applied = offerElements({ fCorr: 1.647 });
    expect(applied[0].props["data-forced-noise-offer"]).toBe("usable");
    expect(offerTexts(applied)).toEqual(["Noise factor from the Depth section (r = 1 arcsec)."]);
    expect(action(applied, "use-depth-fcorr")).toHaveLength(0);
    expect(action(offerElements({ disabled: true }), "use-depth-fcorr")[0].props.disabled).toBe(true);
  });

  it("names a depth measured with another aperture and does not offer it", () => {
    depths.current[R_PATH] = DEPTH;
    const other = offerElements({ radius: 2 });
    expect(other[0].props["data-forced-noise-offer"]).toBe("other_radius");
    const [text] = offerTexts(other);
    expect(
      text.startsWith(
        "The Depth section measured this image at r = 1 arcsec, not with this aperture: its f_corr does not apply.",
      ),
    ).toBe(true);
    expect(text).toBe(
      "The Depth section measured this image at r = 1 arcsec, not with this aperture: its f_corr does not apply. Measure the depth again with this radius to get one.",
    );
    expect(action(other, "use-depth-fcorr")).toHaveLength(0);
    const typed = offerElements({ radius: 2, fCorr: 1.647 });
    expect(typed[0].props["data-forced-noise-offer"]).toBe("other_radius");
    expect(offerTexts(typed)[0].endsWith("Check that the noise factor in the field belongs to this aperture.")).toBe(true);
    expect(action(typed, "use-depth-fcorr")).toHaveLength(0);
    const pixels = offerElements({ radius: 3.8168, unit: "px" });
    expect(pixels[0].props["data-forced-noise-offer"]).toBe("usable");
    expect(offerTexts(pixels)).toEqual(["Depth section: f_corr 1.65 for this image at r = 3.817 px."]);
  });

  it("shows nothing without a depth for that image", () => {
    expect(offer()).toBeNull();
    depths.current[R_PATH] = { ...DEPTH, f_corr: null };
    expect(offer()).toBeNull();
    depths.current = { "C:/d/brick-g.fits": DEPTH };
    expect(offer()).toBeNull();
    expect(noiseOfferText({ kind: "none" }, null)).toBeNull();
    expect(noiseOfferText({ kind: "none" }, 1.647)).toBeNull();
  });

  it("every band row carries the offer with the aperture of the form", () => {
    forcedPhotometryStore.addBand(FILES[1]);
    forcedPhotometryStore.addBand(FILES[0]);
    forcedPhotometryStore.updateBand(R_PATH, { label: "r", fCorr: 1.647 });
    forcedPhotometryStore.setFields({ radius: 3.8168, radiusUnit: "px" });
    const offers = show().filter((e) => e.type === BandNoiseOffer);
    expect(offers.map((e) => e.props)).toEqual([
      { path: R_PATH, label: "r", fCorr: 1.647, radius: 3.8168, unit: "px", disabled: false },
      { path: "C:/d/brick-g.fits", label: "brick-g.fits", fCorr: null, radius: 3.8168, unit: "px", disabled: false },
    ]);
    const rows = byData(show(), "data-forced-band", R_PATH);
    expect((elementsOf<Props>(rows[0]) as Element[]).filter((e) => e.type === BandNoiseOffer)).toHaveLength(1);
    expect(show({ disabled: true }).filter((e) => e.type === BandNoiseOffer).every((e) => e.props.disabled === true)).toBe(true);
  });
});
