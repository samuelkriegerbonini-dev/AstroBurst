import { describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

import { hooks } from "../../../test/hookRuntime";
import DecimalField, { DecimalNumberField, type DecimalFieldProps, type DecimalNumberFieldProps } from "../DecimalField";
import { DecimalField as ExportedDecimalField, DecimalNumberField as ExportedDecimalNumberField } from "..";

const COMMA = "Use a dot (.) as the decimal separator";
const INVALID_RING = "ring-1 ring-red-500/70";

function render(props: Partial<DecimalFieldProps> & { value: string }): string {
  return renderToStaticMarkup(createElement(DecimalField, { onChange: () => {}, ...props }));
}

type AnyProps = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...elements(node.props.children)];
}

function describedByIds(markup: string): string[] {
  return (/<input[^>]*\saria-describedby="([^"]*)"/.exec(markup)?.[1] ?? "").split(/\s+/).filter(Boolean);
}

function messageSpans(markup: string): { id: string; hidden: boolean; className: string; text: string }[] {
  return [...markup.matchAll(/<span id="([^"]+)"( hidden="")? class="([^"]*)">([^<]*)<\/span>/g)].map(([, id, hidden, className, text]) => ({
    id,
    hidden: hidden !== undefined,
    className,
    text,
  }));
}

function textSkippingHidden(markup: string): string {
  return markup.replace(/<(\w+)\b[^>]*\shidden=""[^>]*>[^<]*<\/\1>/g, "").replace(/<[^>]*>/g, "");
}

describe("DecimalField", () => {
  it("is exported from the ui barrel", () => {
    expect(ExportedDecimalField).toBe(DecimalField);
  });

  it("renders a decimal text box with no native number constraints, so a typed comma reaches the parser", () => {
    const markup = render({ value: "2.5", min: 0, max: 10 });
    expect(markup).toContain('type="text"');
    expect(markup).toContain('inputMode="decimal"');
    expect(markup).toContain('value="2.5"');
    expect(markup).not.toMatch(/\s(min|max|step)="/);
    expect(markup).not.toContain('type="number"');
  });

  it("marks 2,5 invalid with the error as its title and a red ring", () => {
    const markup = render({ value: "2,5", className: "base", title: "Aperture radius" });
    expect(markup).toContain('aria-invalid="true"');
    expect(markup).toContain(`title="${COMMA}"`);
    expect(markup).toContain(`class="base ${INVALID_RING}"`);
  });

  it("leaves a valid value unmarked and keeps its own title and class", () => {
    const markup = render({ value: "2.5", className: "base", title: "Aperture radius" });
    expect(markup).not.toContain("aria-invalid");
    expect(markup).toContain('title="Aperture radius"');
    expect(markup).toContain('class="base"');
  });

  it("applies the optional, range and integer rules of decimalFieldState", () => {
    expect(render({ value: "", optional: true })).not.toContain("aria-invalid");
    expect(render({ value: "" })).toContain('aria-invalid="true"');
    expect(render({ value: "150", max: 100 })).toContain('title="Use a value of at most 100"');
    expect(render({ value: "3.5", integer: true })).toContain('title="Use a whole number"');
    expect(render({ value: "3", integer: true })).not.toContain("aria-invalid");
  });

  it("shows a caller error when the text itself parses, and the parse error first otherwise", () => {
    const outOfRange = render({ value: "-2", error: "Redshift must be above -1" });
    expect(outOfRange).toContain('aria-invalid="true"');
    expect(outOfRange).toContain('title="Redshift must be above -1"');
    expect(render({ value: "2,5", error: "Redshift must be above -1" })).toContain(`title="${COMMA}"`);
    expect(render({ value: "0.1", error: null })).not.toContain("aria-invalid");
  });

  it("passes id, placeholder, disabled, aria-label and data attributes through", () => {
    const markup = renderToStaticMarkup(
      createElement(DecimalField, {
        value: "",
        optional: true,
        onChange: () => {},
        id: "gain-field",
        placeholder: "none",
        disabled: true,
        "aria-label": "Gain e-/ADU",
        "data-gain-source": "header",
      } as DecimalFieldProps),
    );
    expect(markup).toContain('id="gain-field"');
    expect(markup).toContain('placeholder="none"');
    expect(markup).toContain('disabled=""');
    expect(markup).toContain('aria-label="Gain e-/ADU"');
    expect(markup).toContain('data-gain-source="header"');
  });

  it("hands the raw text to onChange, comma included", () => {
    hooks.mount();
    const onChange = vi.fn();
    const input = elements(hooks.render(DecimalField, { value: "", onChange }) as ReactNode).find((el) => el.type === "input");
    (input?.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "2,5" } });
    expect(onChange).toHaveBeenCalledWith("2,5");
  });

  it("links the message of an invalid value to the input, hidden by default and shown with showError", () => {
    const hidden = render({ value: "2,5" });
    const [message] = messageSpans(hidden);
    expect(message).toMatchObject({ hidden: true, text: COMMA });
    expect(describedByIds(hidden)).toEqual([message.id]);
    const shown = render({ value: "2,5", showError: true });
    const [visible] = messageSpans(shown);
    expect(visible).toMatchObject({ hidden: false, text: COMMA });
    expect(visible.className).toContain("text-red-400");
    expect(describedByIds(shown)).toEqual([visible.id]);
  });

  it("keeps the hidden message out of the name of a label that wraps the field", () => {
    const markup = renderToStaticMarkup(
      createElement("label", null, createElement("span", null, "lo%"), createElement(DecimalField, { value: "0,5", onChange: () => {} })),
    );
    const [message] = messageSpans(markup);
    expect(message.text).toBe(COMMA);
    expect(describedByIds(markup)).toEqual([message.id]);
    expect(textSkippingHidden(markup)).toBe("lo%");
  });

  it("keeps a caller's aria-describedby and adds the message id only while the value is invalid", () => {
    const invalid = render({ value: "150", max: 100, "aria-describedby": "radius-hint" } as DecimalFieldProps);
    const [message] = messageSpans(invalid);
    expect(message.text).toBe("Use a value of at most 100");
    expect(describedByIds(invalid)).toEqual(["radius-hint", message.id]);
    const valid = render({ value: "50", max: 100, "aria-describedby": "radius-hint" } as DecimalFieldProps);
    expect(messageSpans(valid)).toEqual([]);
    expect(describedByIds(valid)).toEqual(["radius-hint"]);
    expect(render({ value: "50", max: 100 })).not.toContain("aria-describedby");
  });

  it("links a caller error the same way", () => {
    const markup = render({ value: "-2", error: "Redshift must be above -1" });
    const [message] = messageSpans(markup);
    expect(message.text).toBe("Redshift must be above -1");
    expect(describedByIds(markup)).toEqual([message.id]);
  });
});

function renderNumber(props: Partial<DecimalNumberFieldProps> & { value: number | null }): string {
  return renderToStaticMarkup(createElement(DecimalNumberField, { onValueChange: () => {}, ...props }));
}

describe("DecimalNumberField", () => {
  it("is exported from the ui barrel", () => {
    expect(ExportedDecimalNumberField).toBe(DecimalNumberField);
  });

  it("shows the bound number as a dot-decimal draft in a text box", () => {
    const markup = renderNumber({ value: 0.1, min: 0.0001, "aria-label": "Asinh softening parameter a", className: "base" });
    expect(markup).toContain('type="text"');
    expect(markup).toContain('inputMode="decimal"');
    expect(markup).toContain('value="0.1"');
    expect(markup).toContain('aria-label="Asinh softening parameter a"');
    expect(markup).not.toContain("aria-invalid");
    expect(markup).not.toMatch(/\s(min|max|step)="/);
  });

  it("leaves an unset optional value blank and valid", () => {
    const markup = renderNumber({ value: null, optional: true, placeholder: "min" });
    expect(markup).toContain('value=""');
    expect(markup).toContain('placeholder="min"');
    expect(markup).not.toContain("aria-invalid");
  });
});

const B14_PANELS = import.meta.glob<string>(
  [
    "../../preview/DisplayControls.tsx",
    "../../analysis/PhotometryTablePanel.tsx",
    "../../analysis/PhotometryPanel.tsx",
    "../../analysis/ContourPanel.tsx",
    "../../analysis/CatalogPanel.tsx",
    "../../analysis/LineListControls.tsx",
    "../../analysis/SpectrumComparisonSection.tsx",
    "../../analysis/SpectralAxisControls.tsx",
    "../../analysis/LineFitSection.tsx",
    "../../compose/steps/CropStep.tsx",
    "../../compose/steps/ChannelStep.tsx",
  ],
  { query: "?raw", import: "default", eager: true },
);

function offendingLines(pattern: RegExp): string[] {
  return Object.entries(B14_PANELS).flatMap(([name, text]) =>
    text.split(/\r?\n/).flatMap((line, i) => (pattern.test(line) ? [`${name}:${i + 1}: ${line.trim()}`] : [])),
  );
}

describe("decimal inputs of the B14 panels", () => {
  it("scans every converted panel", () => {
    expect(Object.keys(B14_PANELS)).toHaveLength(11);
  });

  it("use decimal text fields instead of number inputs or the parseFloat and parseInt copies", () => {
    expect(offendingLines(/type="number"|parseOptionalNumber|parseNumberOr|parseInt\(/)).toEqual([]);
  });

  it("use the muted text tokens instead of the #71717a and #52525b literals", () => {
    expect(offendingLines(/#(71717a|52525b)\b/i)).toEqual([]);
  });

  it("use the muted text token instead of text-zinc-700 outside the decorative icons", () => {
    const informationalZinc = /^(?!.*<(ChevronRight|GripVertical|ArrowRight|Grid3X3)\b).*(^|[\s"'`:])text-zinc-700\b/;
    expect(offendingLines(informationalZinc)).toEqual([]);
  });
});
