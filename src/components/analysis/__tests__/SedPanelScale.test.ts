import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: { id: "f1", path: "C:/data/cube.fits", name: "cube.fits" } }),
}));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { formatWithUncertainty } from "../../../utils/formatUncertainty";
import { measurementLog } from "../../../utils/measurementLog";
import type { PhotometricPlane } from "../../../shared/types/cube";
import type { SedPoint, SedSource } from "../../../utils/sedAxis";
import { CSV_LINE_END } from "../../../utils/catalogCsv";
import SedPanel from "../SedPanel";

type Props = Record<string, unknown> & { children?: ReactNode };
type Element = ReactElement<Props>;
type Component = (props: Props) => unknown;

const G: SedPoint = { index: 0, filter: "g", pivotUm: 0.4808, value: 2, err: 0.1 };
const R: SedPoint = { index: 1, filter: "r", pivotUm: 0.6418, value: 0.3, err: null, limit: true };
const POINTS = { points: [G, R], unit: "uJy", label: "source 7" };

function plane(filter: string, pivot_um: number): PhotometricPlane {
  return {
    filter,
    dataset: "JWST",
    instrument: "NIRCam",
    pivot_um,
    pixscale_arcsec: 0.04,
    bunit_in: "Jy",
    fluxconv: 1,
    psf_fwhm_arcsec: null,
    kernel: "",
  };
}

const PLANES = [plane("F150W", 1.500744), plane("F277W", 2.761), plane("F444W", 4.404315)];
const PIXEL: SedSource = { spectrum: [3, 1, 2], err: [0.3, 0.1, 0.2], coord: { x: 2, y: 3 }, region: null };

function viewOf(props: Props): Element {
  const panel = (SedPanel as unknown as { type: Component }).type;
  hooks.mount();
  const form = hooks.render(panel, props) as Element;
  hooks.mount();
  const view = hooks.render(form.type as Component, form.props) as Element;
  hooks.mount();
  return view;
}

const show = (view: Element) => elementsOf<Props>(hooks.render(view.type as Component, view.props)) as Element[];
const button = (tree: Element[], action: string) => tree.find((e) => e.props["data-action"] === action);
const click = (element: Element | undefined) => (element!.props.onClick as () => void)();
const ticks = (tree: Element[]) =>
  tree.filter((e) => e.props["data-sed-ytick"] !== undefined).map((e) => ({ y: Number(e.props.y), text: textOf(e) }));
const yTitle = (tree: Element[]) => textOf(tree.find((e) => e.type === "text" && String(e.props.transform ?? "").startsWith("rotate")));
const svgLabel = (tree: Element[]) => tree.find((e) => e.type === "svg")?.props["aria-label"];
const countOf = (tree: Element[], attribute: string) => tree.filter((e) => e.props[attribute] !== undefined).length;
const tickNumbers = (tree: Element[]) =>
  ticks(tree)
    .sort((a, b) => a.y - b.y)
    .map((t) => Number(t.text));
const cellTexts = (row: Element) => elementsOf<Props>(row.props.children).filter((e) => e.type === "th" || e.type === "td").map((e) => textOf(e));

function columnCell(tree: Element[], header: string, filter: string): string | undefined {
  const headerRow = tree.find((e) => e.type === "thead");
  const headers = elementsOf<Props>(headerRow?.props.children).filter((e) => e.type === "th").map((e) => textOf(e));
  const row = tree.find((e) => e.props["data-sed-row"] !== undefined && cellTexts(e)[0] === filter);
  return row ? cellTexts(row)[headers.indexOf(header)] : undefined;
}

const decreasing = (values: number[]) => values.every((v, i) => i === 0 || v < values[i - 1]);
const increasing = (values: number[]) => values.every((v, i) => i === 0 || v > values[i - 1]);
const tick = async () => {
  for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
};

describe("SedPanel scale switches", () => {
  beforeEach(() => {
    hooks.mount();
  });

  afterEach(() => {
    hooks.mount();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("the scale switches start on the linear view", () => {
    const tree = show(viewOf(POINTS));
    const logY = button(tree, "sed-log-y")!;
    expect(textOf(logY)).toBe("Log y");
    expect(logY.props["aria-pressed"]).toBe(false);
    expect(logY.props.disabled).toBe(false);
    const ab = button(tree, "sed-ab")!;
    expect(textOf(ab)).toBe("AB mag");
    expect(ab.props["aria-pressed"]).toBe(false);
    expect(yTitle(tree)).toBe("Flux (uJy)");
    const numbers = tickNumbers(tree);
    expect(numbers.length).toBeGreaterThan(1);
    expect(decreasing(numbers)).toBe(true);
    const empty = show(viewOf({ points: [], unit: "uJy", label: "s" }));
    expect(button(empty, "sed-log-y")).toBeUndefined();
    expect(button(empty, "sed-ab")).toBeUndefined();
  });

  it("Log y draws the values on a log axis", () => {
    const view = viewOf(POINTS);
    click(button(show(view), "sed-log-y"));
    const tree = show(view);
    expect(button(tree, "sed-log-y")!.props["aria-pressed"]).toBe(true);
    expect(
      ticks(tree)
        .sort((a, b) => b.y - a.y)
        .map((t) => t.text),
    ).toEqual(["0.5", "1", "2"]);
    expect(countOf(tree, "data-sed-point")).toBe(2);
    expect(countOf(tree, "data-sed-limit")).toBe(1);
    expect(countOf(tree, "data-sed-err")).toBe(1);
    expect(yTitle(tree)).toBe("Flux (uJy)");
    expect(svgLabel(tree)).toBe("SED: Flux (uJy) on a log scale against pivot wavelength");
    click(button(tree, "sed-log-y"));
    expect(button(show(view), "sed-log-y")!.props["aria-pressed"]).toBe(false);
  });

  it("Log y is refused with its reason when a value minus its error is not above zero", () => {
    const tree = show(viewOf({ ...POINTS, points: [G, { ...G, index: 2, filter: "z", pivotUm: 0.9, value: 0.05, err: 0.1 }] }));
    const logY = button(tree, "sed-log-y")!;
    expect(logY.props.disabled).toBe(true);
    expect(logY.props.title).toBe("A log scale needs every plotted value minus its error above 0");
  });

  it("AB mag draws magnitudes with the brightest at the top", () => {
    const view = viewOf(POINTS);
    click(button(show(view), "sed-ab"));
    const tree = show(view);
    expect(button(tree, "sed-ab")!.props["aria-pressed"]).toBe(true);
    expect(button(tree, "sed-log-y")!.props["aria-pressed"]).toBe(false);
    expect(yTitle(tree)).toBe("AB magnitude");
    const numbers = tickNumbers(tree);
    expect(numbers.length).toBeGreaterThan(1);
    expect(increasing(numbers)).toBe(true);
    const limits = tree.filter((e) => e.props["data-sed-limit"] !== undefined);
    expect(limits).toHaveLength(1);
    expect(countOf(elementsOf<Props>(limits[0].props.children) as Element[], "data-sed-err")).toBe(0);
    expect(columnCell(tree, "AB mag", "g")).toBe(formatWithUncertainty(23.147425010840045, 0.05428681023790645));
    expect(columnCell(tree, "AB mag", "r")).toBe(`> ${formatWithUncertainty(25.207196863200842, null)}`);
    expect(svgLabel(tree)).toBe("SED: AB magnitude against pivot wavelength");
  });

  it("the AB view is offered only for a flux density and names the points it cannot plot", () => {
    const cube = show(viewOf({ planes: PLANES, bunit: "MJy/sr", source: PIXEL }));
    expect(button(cube, "sed-ab")).toBeUndefined();
    expect(button(cube, "sed-log-y")).toBeDefined();

    const negative = { ...G, index: 2, filter: "z", pivotUm: 0.9, value: -0.4, err: 0.2 };
    const view = viewOf({ ...POINTS, points: [G, negative] });
    click(button(show(view), "sed-ab"));
    const tree = show(view);
    expect(countOf(tree, "data-sed-point")).toBe(1);
    expect(textOf(tree.find((e) => e.props["data-sed-ab-note"] !== undefined))).toBe(
      "Not plotted in AB magnitudes: 1 point with a flux of zero or below.",
    );
    expect(columnCell(tree, "AB mag", "z")).toBe("--");

    const zero = { ...G, index: 3, filter: "i", pivotUm: 0.78, value: 0, err: 0.2 };
    const three = viewOf({ ...POINTS, points: [G, negative, zero] });
    click(button(show(three), "sed-ab"));
    expect(textOf(show(three).find((e) => e.props["data-sed-ab-note"] !== undefined))).toBe(
      "Not plotted in AB magnitudes: 2 points with a flux of zero or below.",
    );

    const none = button(show(viewOf({ ...POINTS, points: [{ ...G, value: -1 }] })), "sed-ab")!;
    expect(none.props.disabled).toBe(true);
    expect(none.props.title).toBe("No point has a flux above zero: there is no AB magnitude to plot");
  });

  it("the CSV and the log row keep the fluxes in every view", async () => {
    const writeText = vi.fn((_text: string) => Promise.resolve());
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const append = vi.spyOn(measurementLog, "append");
    const copy = (tree: Element[]) => tree.find((e) => e.type === "button" && textOf(e).includes("Copy CSV"));
    const view = viewOf(POINTS);
    click(copy(show(view)));
    await tick();
    click(button(show(view), "sed-ab"));
    click(copy(show(view)));
    await tick();
    expect(writeText).toHaveBeenCalledTimes(2);
    const fromHeader = (text: string) => {
      const lines = text.split(CSV_LINE_END);
      return lines.slice(lines.indexOf("filter,pivot_um,flux,flux_err,limit"));
    };
    const first = fromHeader(writeText.mock.calls[0][0]);
    const second = fromHeader(writeText.mock.calls[1][0]);
    expect(first[0]).toBe("filter,pivot_um,flux,flux_err,limit");
    expect(second).toEqual(first);
    expect(first.filter((line) => line !== "").slice(-2)).toEqual(["g,0.4808,2,0.1,false", "r,0.6418,0.3,,true"]);
    expect(append).toHaveBeenCalledTimes(2);
    const draft = append.mock.calls[1][0];
    expect(draft.unit).toBe("uJy");
    expect(draft.values.flux_g).toBe(2);
  });
});
