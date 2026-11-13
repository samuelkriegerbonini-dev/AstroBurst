import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import LineListControls, { type LineListControlsProps } from "../LineListControls";
import { BAND_SPECIES, LINE_FAMILIES } from "../../../utils/lineList";

const controlsFunction = (LineListControls as unknown as { type: (props: LineListControlsProps) => ReactElement }).type;

function props(extra: Partial<LineListControlsProps> = {}): LineListControlsProps {
  return {
    visible: true,
    onVisibleChange: () => {},
    redshift: 0,
    onRedshiftChange: () => {},
    convention: "optical",
    families: LINE_FAMILIES,
    onFamiliesChange: () => {},
    frameLabel: "z and v_sys in the stored frame",
    availability: { ok: true },
    clickHint: "",
    pickedLabel: null,
    drawnCount: 3,
    ...extra,
  };
}

function markup(extra: Partial<LineListControlsProps> = {}): string {
  return renderToStaticMarkup(hooks.render(controlsFunction, props(extra)) as ReactElement);
}

function tree(extra: Partial<LineListControlsProps>) {
  return elementsOf(hooks.render(controlsFunction, props(extra)));
}

function speciesGroup(html: string): string {
  const start = html.indexOf('aria-label="Band species"');
  const end = html.indexOf("</div>", start);
  return html.slice(start, end);
}

function chipTag(html: string, species: string): string {
  const group = speciesGroup(html);
  const close = group.indexOf(`>${species}</button>`);
  const open = group.lastIndexOf("<button", close);
  return group.slice(open, close + 1);
}

const BANDS_ON: Partial<LineListControlsProps> = { onBandSpeciesChange: () => {}, bandSpecies: ["H2O", "CO2"], bandDrawnCount: 4 };
const NOTE = "band origins and atomic lines from NIST; the shaded widths of molecular bands are display guides";

beforeEach(() => {
  hooks.mount();
});

describe("LineListControls band controls", () => {
  it("renders the line controls as before when no band props are passed", () => {
    const html = markup();
    expect(html).toContain("Rest lines");
    expect(html).toContain("3 of 47 in range");
    expect(html).not.toContain("Bands");
  });

  it("shows the band switch unchecked and nothing else while the list is empty", () => {
    const html = markup({ onBandSpeciesChange: () => {}, bandSpecies: [] });
    expect(html).toContain("Bands");
    expect(html).not.toContain('aria-label="Band species"');
    expect(html).not.toContain(">H2O</button>");
    expect(html.split("in range").length - 1).toBe(1);
    expect(html).not.toContain(NOTE);
  });

  it("shows the count and one chip per species while bands are on", () => {
    const html = markup(BANDS_ON);
    expect(html).toContain("4 of 21 in range");
    expect(html).toContain('aria-label="Band species"');
    const group = speciesGroup(html);
    expect(group.split("<button").length - 1).toBe(14);
    expect(BAND_SPECIES).toHaveLength(14);
    for (const species of BAND_SPECIES) expect(group).toContain(`>${species}</button>`);
  });

  it("marks the chosen species as pressed", () => {
    const html = markup(BANDS_ON);
    expect(chipTag(html, "H2O")).toContain('aria-pressed="true"');
    expect(chipTag(html, "CO2")).toContain('aria-pressed="true"');
    expect(chipTag(html, "CH4")).toContain('aria-pressed="false"');
  });

  it("gives each chip the sources of its rows", () => {
    const h2o = tree(BANDS_ON).find((node) => node.type === "button" && textOf(node) === "H2O")!;
    const title = h2o.props.title as string;
    expect(title).toContain("NIST WebBook C7732185");
    expect(title).toContain("3756 cm-1");
    expect(title).toContain("1595 cm-1");
    expect(title.split(" | ")).toHaveLength(2);
    expect(chipTag(markup(BANDS_ON), "H2O")).toContain("title=");
  });

  it("states that the ranges are display guides", () => {
    expect(markup(BANDS_ON)).toContain(NOTE);
    const unavailable = markup({ ...BANDS_ON, availability: { ok: false, reason: "x" } });
    expect(unavailable).not.toContain("of 21 in range");
    expect(unavailable).toContain('aria-label="Band species"');
    expect(unavailable).toContain(">H2O</button>");
  });

  it("toggles a species through the table order and the switch through the defaults", () => {
    const change = vi.fn();
    const on = tree({ ...BANDS_ON, onBandSpeciesChange: change });
    const chip = (species: string) => on.find((node) => node.type === "button" && textOf(node) === species)!;
    (chip("CH4").props.onClick as () => void)();
    expect(change).toHaveBeenLastCalledWith(["H2O", "CO2", "CH4"]);
    (chip("H2O").props.onClick as () => void)();
    expect(change).toHaveBeenLastCalledWith(["CO2"]);

    const bandsSwitch = (nodes: ReturnType<typeof tree>) => {
      const label = nodes.find((node) => node.type === "label" && textOf(node).includes("Bands"))!;
      return elementsOf(label.props.children).find((node) => node.type === "input")!;
    };
    (bandsSwitch(on).props.onChange as (event: unknown) => void)({ target: { checked: false } });
    expect(change).toHaveBeenLastCalledWith([]);

    const off = tree({ onBandSpeciesChange: change, bandSpecies: [] });
    expect(bandsSwitch(off).props.checked).toBe(false);
    (bandsSwitch(off).props.onChange as (event: unknown) => void)({ target: { checked: true } });
    expect(change).toHaveBeenLastCalledWith(["H2O", "CO2", "CO", "CH4", "SO2"]);
  });
});
