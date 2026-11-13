import { describe, expect, it, vi } from "vitest";
import { createElement, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { CompareColumn, ComparePreset } from "../../../shared/types/catalogCompare";
import { formFromPreset, type CompareForm } from "../../../utils/catalogCompare";
import { elementsOf } from "../../../test/elementTree";
import CompareFormFields, { type CompareFormFieldsProps } from "../CompareFormFields";
import { columnOptions } from "../compareFormFieldsModel";

type AnyProps = Record<string, unknown>;

const TEXT_COLUMNS = new Set(["ls_id", "type"]);
const COLUMNS: CompareColumn[] = [
  "ls_id",
  "ra",
  "dec",
  "type",
  "maskbits",
  "fracflux_r",
  "apflux_r_5",
  "apflux_ivar_r_5",
].map((name) => ({ name, numeric: !TEXT_COLUMNS.has(name), unit: null }));

const LEGACY_R: ComparePreset = {
  id: "legacy_dr10",
  label: "Legacy Surveys DR10",
  band: "r",
  mapping: {
    id: "ls_id",
    ra: "ra",
    dec: "dec",
    flux: "apflux_r_5",
    flux_unit: "nanomaggy",
    zero_point: null,
    flux_err: null,
    flux_ivar: "apflux_ivar_r_5",
    class: "type",
    missing_value: null,
  },
  aperture: { radius_arcsec: 2, radius_px: null, annulus_arcsec: null, annulus_px: null },
  cuts: [
    { column: "type", op: "==", value: "PSF" },
    { column: "maskbits", op: "==", value: 0 },
    { column: "fracflux_r", op: "<", value: 0.01 },
  ],
  min_snr: null,
  max_snr: null,
  min_neighbour_arcsec: null,
  notes: [],
};

const FORM: CompareForm = formFromPreset(LEGACY_R);

function propsWith(form: Partial<CompareForm>, extra: Partial<CompareFormFieldsProps> = {}): CompareFormFieldsProps {
  return { form: { ...FORM, ...form }, columns: COLUMNS, onChange: () => undefined, ...extra };
}

function markup(props: CompareFormFieldsProps): string {
  return renderToStaticMarkup(createElement(CompareFormFields, props));
}

function tree(props: CompareFormFieldsProps): ReactElement<AnyProps>[] {
  return elementsOf<AnyProps>((CompareFormFields as unknown as { type: (p: CompareFormFieldsProps) => unknown }).type(props));
}

function fieldsOf(elements: ReactElement<AnyProps>[], name: string): ReactElement<AnyProps>[] {
  return elements.filter((element) => element.props["data-field"] === name);
}

function field(elements: ReactElement<AnyProps>[], name: string): ReactElement<AnyProps> {
  const found = fieldsOf(elements, name)[0];
  if (!found) throw new Error(`no control with data-field ${name}`);
  return found;
}

function toggle(elements: ReactElement<AnyProps>[], label: string): ReactElement<AnyProps> {
  const found = elements.find((element) => element.props.label === label);
  if (!found) throw new Error(`no toggle ${label}`);
  return found;
}

function selectSlice(html: string, name: string): string {
  const at = html.indexOf(`data-field="${name}"`);
  if (at < 0) return "";
  const start = html.lastIndexOf("<select", at);
  return html.slice(start, html.indexOf("</select>", at));
}

function optionsOf(html: string, name: string): string[] {
  return [...selectSlice(html, name).matchAll(/<option value="([^"]*)"/g)].map((match) => match[1]);
}

function optionTextsOf(html: string, name: string): string[] {
  return [...selectSlice(html, name).matchAll(/<option[^>]*>([^<]*)<\/option>/g)].map((match) => match[1]);
}

function selectedOf(html: string, name: string): string | null {
  return /<option value="([^"]*)" selected=""/.exec(selectSlice(html, name))?.[1] ?? null;
}

function inputOf(html: string, name: string): string {
  const at = html.indexOf(`data-field="${name}"`);
  if (at < 0) return "";
  const start = html.lastIndexOf("<input", at);
  return html.slice(start, html.indexOf(">", at) + 1);
}

const NUMERIC_OPTIONS = ["", "ra", "dec", "maskbits", "fracflux_r", "apflux_r_5", "apflux_ivar_r_5"];
const ALL_OPTIONS = ["", "ls_id", "ra", "dec", "type", "maskbits", "fracflux_r", "apflux_r_5", "apflux_ivar_r_5"];

describe("CompareFormFields", () => {
  it("lists the numeric columns for position, flux and error, and every column for the id, the class and the cuts", () => {
    const html = markup(propsWith({}));
    for (const name of ["raColumn", "decColumn", "fluxColumn", "errorColumn"]) expect(optionsOf(html, name)).toEqual(NUMERIC_OPTIONS);
    for (const name of ["idColumn", "classColumn", "cutColumn"]) expect(optionsOf(html, name)).toEqual(ALL_OPTIONS);
    expect(columnOptions(COLUMNS, "", true)).toHaveLength(6);
    expect(columnOptions(COLUMNS, "", false)).toHaveLength(8);
  });

  it("shows the choice of the form in every control", () => {
    const html = markup(propsWith({}));
    expect(selectedOf(html, "raColumn")).toBe("ra");
    expect(selectedOf(html, "fluxColumn")).toBe("apflux_r_5");
    expect(selectedOf(html, "fluxUnit")).toBe("nanomaggy");
    expect(selectedOf(html, "errorKind")).toBe("flux_ivar");
    expect(selectedOf(html, "errorColumn")).toBe("apflux_ivar_r_5");
    expect(selectedOf(html, "idColumn")).toBe("ls_id");
    expect(selectedOf(html, "classColumn")).toBe("type");
    expect(selectedOf(html, "radiusUnit")).toBe("arcsec");
    expect(inputOf(html, "radiusText")).toContain('value="2"');
    const rows = [...html.matchAll(/data-cut="(\d+)"/g)].map((match) => match[1]);
    expect(rows).toEqual(["0", "1", "2"]);
    const values = [...html.matchAll(/<input[^>]*data-field="cutValue"[^>]*>/g)].map((match) => /value="([^"]*)"/.exec(match[0])?.[1]);
    expect(values).toEqual(["PSF", "0", "0.01"]);
  });

  it("keeps a column the table lacks visible", () => {
    expect(columnOptions(COLUMNS, "apflux_z_5", true).at(-1)).toBe("apflux_z_5");
    expect(columnOptions(COLUMNS, "type", true).at(-1)).toBe("type");
    expect(columnOptions(COLUMNS, "ra", true).filter((name) => name === "ra")).toHaveLength(1);
  });

  it("asks for a zero point only for counts with a zero point", () => {
    expect(markup(propsWith({}))).not.toContain('data-field="zeroPointText"');
    const html = markup(propsWith({ fluxUnit: "zero_point" }));
    expect(html).toContain('data-field="zeroPointText"');
    expect(optionTextsOf(html, "fluxUnit")).toEqual(["nanomaggy", "Jy", "mJy", "uJy", "nJy", "counts with a zero point", "AB magnitude"]);
  });

  it("asks for an error column only when an error kind is chosen", () => {
    const html = markup(propsWith({ errorKind: "none" }));
    expect(html).not.toContain('data-field="errorColumn"');
    expect(optionTextsOf(html, "errorKind")).toEqual(["none", "error column", "inverse variance column"]);
  });

  it("asks for the annulus only with a local sky, in the unit of the radius", () => {
    expect(markup(propsWith({}))).not.toContain('data-field="annulusInnerText"');
    const html = markup(propsWith({ localSky: true, radiusUnit: "px" }));
    expect(html).toContain("Sky annulus inner (px)");
    expect(html).toContain("Sky annulus outer (px)");
  });

  it("every change returns the whole form with one member changed", () => {
    const selects: [keyof CompareForm, string][] = [
      ["raColumn", "dec"],
      ["decColumn", "ra"],
      ["fluxColumn", "maskbits"],
      ["fluxUnit", "uJy"],
      ["errorKind", "flux_err"],
      ["errorColumn", "maskbits"],
      ["idColumn", ""],
      ["classColumn", ""],
      ["radiusUnit", "px"],
    ];
    const decimals: [keyof CompareForm, string][] = [
      ["radiusText", "3.5"],
      ["missingValueText", "-999"],
      ["minSnrText", "50"],
      ["maxSnrText", "200"],
      ["minNeighbourText", "2"],
    ];
    const expectOneCall = (call: (elements: ReactElement<AnyProps>[]) => void, member: keyof CompareForm, value: unknown) => {
      const onChange = vi.fn();
      call(tree(propsWith({}, { onChange })));
      expect(onChange).toHaveBeenCalledTimes(1);
      expect(onChange).toHaveBeenCalledWith({ ...FORM, [member]: value });
      expect(onChange.mock.calls[0][0].presetKey).toBe("legacy_dr10:r");
    };
    for (const [member, value] of selects) {
      expectOneCall((elements) => (field(elements, member).props.onChange as (e: unknown) => void)({ target: { value } }), member, value);
    }
    for (const [member, value] of decimals) {
      expectOneCall((elements) => (field(elements, member).props.onChange as (t: string) => void)(value), member, value);
    }
    expectOneCall(
      (elements) => (field(elements, "catalogueBand").props.onChange as (e: unknown) => void)({ target: { value: "g" } }),
      "catalogueBand",
      "g",
    );
    expectOneCall((elements) => (toggle(elements, "Local sky annulus").props.onChange as (v: boolean) => void)(true), "localSky", true);
    expectOneCall((elements) => (toggle(elements, "Exclude flagged pixels").props.onChange as (v: boolean) => void)(true), "excludeDq", true);
  });

  it("cuts can be added, edited and removed", () => {
    const run = (call: (elements: ReactElement<AnyProps>[]) => void, form: Partial<CompareForm> = {}): CompareForm => {
      const onChange = vi.fn();
      call(tree(propsWith(form, { onChange })));
      expect(onChange).toHaveBeenCalledTimes(1);
      return onChange.mock.calls[0][0] as CompareForm;
    };
    const added = run((elements) => (elements.find((e) => e.props["data-action"] === "add-cut")!.props.onClick as () => void)());
    expect(added.cuts).toHaveLength(4);
    expect(added.cuts[3]).toEqual({ column: "", op: "==", value: "" });
    expect(added.presetKey).toBe("legacy_dr10:r");

    const opChanged = run((elements) => (fieldsOf(elements, "cutOp")[1].props.onChange as (e: unknown) => void)({ target: { value: "!=" } }));
    expect(opChanged.cuts[1]).toEqual({ column: "maskbits", op: "!=", value: "0" });
    expect(opChanged.cuts[0]).toBe(FORM.cuts[0]);
    expect(opChanged.cuts[2]).toBe(FORM.cuts[2]);
    expect(opChanged).toEqual({ ...FORM, cuts: opChanged.cuts });

    const valueChanged = run((elements) =>
      (fieldsOf(elements, "cutValue")[2].props.onChange as (e: unknown) => void)({ target: { value: "0.05" } }),
    );
    expect(valueChanged.cuts[2].value).toBe("0.05");

    const removed = run((elements) => (elements.find((e) => e.props["aria-label"] === "Remove cut 1")!.props.onClick as () => void)());
    expect(removed.cuts).toEqual([FORM.cuts[1], FORM.cuts[2]]);

    const html = markup(propsWith({ cuts: [] }));
    expect(html).not.toContain("data-cut=");
    expect(html).toContain('data-action="add-cut"');
    expect(html).toContain("Add a cut");
  });

  it("nothing can be changed while disabled", () => {
    const elements = tree(propsWith({ fluxUnit: "zero_point", localSky: true }, { disabled: true }));
    const controls = elements.filter((element) => element.props["data-field"] !== undefined);
    expect(controls.length).toBeGreaterThan(20);
    for (const control of controls) expect([control.props["data-field"], control.props.disabled]).toEqual([control.props["data-field"], true]);
    expect(toggle(elements, "Local sky annulus").props.disabled).toBe(true);
    expect(toggle(elements, "Exclude flagged pixels").props.disabled).toBe(true);
    expect(elements.find((e) => e.props["data-action"] === "add-cut")!.props.disabled).toBe(true);
    const removes = elements.filter((e) => typeof e.props["aria-label"] === "string" && (e.props["aria-label"] as string).startsWith("Remove cut"));
    expect(removes).toHaveLength(3);
    for (const button of removes) expect(button.props.disabled).toBe(true);
  });

  it("marks a number that cannot be read", () => {
    expect(inputOf(markup(propsWith({ radiusText: "2,5" })), "radiusText")).toContain('aria-invalid="true"');
    expect(inputOf(markup(propsWith({ radiusText: "2.5" })), "radiusText")).not.toContain("aria-invalid");
  });
});
