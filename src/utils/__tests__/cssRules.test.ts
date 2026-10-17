import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

function read(rel: string): string {
  return readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
}

const indexCss = read("../../index.css");
const viewerCss = read("../../advanced-viewer.css");
const stylesheets = `${indexCss}\n${viewerCss}`;

function ruleBody(text: string, selector: string): string | null {
  for (const m of text.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (m[1].split(",").some((s) => s.trim() === selector)) return m[2];
  }
  return null;
}

function declarations(body: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const m of body.matchAll(/([\w-]+)\s*:\s*([^;]+);/g)) out.set(m[1], m[2].trim());
  return out;
}

function rule(text: string, selector: string): Map<string, string> {
  const body = ruleBody(text, selector);
  if (body === null) throw new Error(`${selector} is not defined`);
  return declarations(body);
}

function px(value: string | undefined): number[] {
  return (value ?? "").split(/\s+/).map((v) => Number(/^([\d.]+)px$/.exec(v)?.[1] ?? NaN));
}

function classNames(source: string): string[] {
  const names = new Set<string>();
  for (const m of source.matchAll(/className=(?:"([^"]*)"|\{`([^`]*)`\})/g)) {
    for (const name of (m[1] ?? m[2]).matchAll(/(?<![\w-])ab-[\w-]+/g)) names.add(name[0]);
  }
  return [...names];
}

describe("viewer canvas", () => {
  it("does not start a text selection when a crosshair drag begins in the letterbox", () => {
    const decl = rule(viewerCss, ".ab-viewer-canvas");
    expect(decl.get("user-select")).toBe("none");
    expect(decl.get("-webkit-user-select")).toBe("none");
  });
});

describe("disabled primary and secondary buttons", () => {
  for (const button of [".ab-btn-primary", ".ab-btn-secondary"]) {
    it(`${button} looks disabled and does not react to hover or press when disabled`, () => {
      const disabled = rule(indexCss, `${button}:disabled`);
      expect(Number(disabled.get("opacity"))).toBeLessThan(0.6);
      expect(disabled.get("cursor")).toBe("not-allowed");
      expect(ruleBody(indexCss, `${button}:hover`)).toBeNull();
      expect(ruleBody(indexCss, `${button}:active`)).toBeNull();
      expect(rule(indexCss, `${button}:hover:not(:disabled)`).get("transform")).toBe("translateY(-1px)");
      expect(rule(indexCss, `${button}:active:not(:disabled)`).get("transform")).toBe("scale(0.98)");
    });
  }
});

describe("Run button and palette classes", () => {
  it("define a small Run button with less padding and a smaller font than the default one", () => {
    const base = rule(indexCss, ".ab-run-btn");
    const small = rule(indexCss, ".ab-run-btn.ab-run-btn-sm");
    const [baseY, baseX] = px(base.get("padding"));
    const [smallY, smallX] = px(small.get("padding"));
    expect(smallY).toBeLessThan(baseY);
    expect(smallX).toBeLessThan(baseX);
    expect(px(small.get("font-size"))[0]).toBeLessThan(px(base.get("font-size"))[0]);
  });

  it("define the palette empty state: padded, centred, muted text token", () => {
    const empty = rule(indexCss, ".ab-cmdp-empty");
    expect(px(empty.get("padding")).every((v) => v > 0)).toBe(true);
    expect(empty.get("text-align")).toBe("center");
    expect(empty.get("color")).toBe("var(--ab-text-3)");
  });

  it.each([
    ["RunButton", "../../components/ui/RunButton.tsx"],
    ["CommandPalette", "../../components/CommandPalette.tsx"],
  ])("style every ab- class that %s renders", (_name, rel) => {
    const used = classNames(read(rel));
    expect(used.length).toBeGreaterThan(0);
    const undefinedClasses = used.filter((name) => !new RegExp(`\\.${name}(?![\\w-])`).test(stylesheets));
    expect(undefinedClasses).toEqual([]);
  });
});
