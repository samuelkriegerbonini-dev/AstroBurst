import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

function read(rel: string): string {
  return readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
}

const indexCss = read("../../index.css");
const viewerCss = read("../../advanced-viewer.css");
const dockCss = read("../../components/dock/dock.css");
const stylesheets = `${indexCss}\n${viewerCss}`;

const srcDir = fileURLToPath(new URL("../../", import.meta.url));
const everyStylesheet = readdirSync(srcDir, { recursive: true, encoding: "utf8" })
  .filter((rel) => rel.endsWith(".css"))
  .map((rel) => ({ file: rel.replaceAll("\\", "/"), text: readFileSync(join(srcDir, rel), "utf8").replace(/\/\*[\s\S]*?\*\//g, "") }));

function ruleBody(text: string, selector: string): string | null {
  for (const m of text.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (m[1].split(",").some((s) => s.trim() === selector)) return m[2];
  }
  return null;
}

function ruleBodies(text: string, selector: string): string[] {
  return [...text.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter((m) => m[1].split(",").some((s) => s.trim() === selector)).map((m) => m[2]);
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

function subject(selector: string): string {
  return selector.split(/[\s>+~]+/).filter(Boolean).at(-1) ?? "";
}

function declaredFor(targets: RegExp, property: string): string[] {
  const found: string[] = [];
  for (const { file, text } of everyStylesheet) {
    for (const m of text.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      const value = declarations(m[2]).get(property);
      if (value === undefined) continue;
      const hits = m[1].split(",").map(subject).filter((s) => targets.test(s) && !s.includes("::"));
      found.push(...hits.map(() => `${file}: ${value}`));
    }
  }
  return found;
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

  it("keeps touch panning to the viewer, so a pan drag does not scroll or zoom the page", () => {
    expect(rule(viewerCss, ".ab-viewer-canvas").get("touch-action")).toBe("none");
  });
});

describe("sidebar rail", () => {
  const RAIL = ".ab-dock-strip[data-workspace-rail]";

  it("declares the rail width and the caption size once, in the rail base of dock.css", () => {
    expect(declaredFor(/\.ab-dock-strip(?![\w-])/, "width")).toEqual(["components/dock/dock.css: 48px"]);
    expect(declaredFor(/\.ab-dock-strip-btn(?![\w-])/, "font-size")).toEqual(["components/dock/dock.css: 9px"]);
  });

  it("shows a thin token-coloured scrollbar on the rails, so a rail with more tools than fit shows that it scrolls", () => {
    expect(declaredFor(/\.ab-dock-strip(?![\w-])/, "scrollbar-width")).toEqual([]);
    expect(declaredFor(/\.ab-dock-strip(?![\w-])/, "scrollbar-color")).toEqual([]);
    expect(rule(dockCss, RAIL).get("overflow-y")).toBe("auto");
    const scrollbar = rule(dockCss, `${RAIL}::-webkit-scrollbar`);
    expect(scrollbar.has("display")).toBe(false);
    const [width] = px(scrollbar.get("width"));
    expect(width).toBeGreaterThanOrEqual(2);
    expect(width).toBeLessThanOrEqual(4);
    expect(rule(dockCss, `${RAIL}::-webkit-scrollbar-thumb`).get("background")).toMatch(/^var\(--ab-[\w-]+\)$/);
  });

  it("keeps the 44px rail buttons clear of that scrollbar on both rails", () => {
    const rail = rule(dockCss, RAIL);
    const [railWidth] = px(rail.get("width"));
    const [, paddingX] = (rail.get("padding") ?? "").split(/\s+/).map(parseFloat);
    const [scrollbar] = px(rule(dockCss, `${RAIL}::-webkit-scrollbar`).get("width"));
    const [button] = px(rule(dockCss, `${RAIL} .ab-dock-strip-btn`).get("width"));
    for (const [side, edge] of [["left", "border-right"], ["right", "border-left"]]) {
      const border = parseFloat(rule(dockCss, `${RAIL}[data-side="${side}"]`).get(edge) ?? "");
      expect(railWidth - border - 2 * paddingX - scrollbar).toBeGreaterThanOrEqual(button);
    }
  });
});

describe("section pills", () => {
  it.each([".ab-step-pill", ".ab-processing-pill"])("%s leaves the background to the bg- utilities of the active pill", (selector) => {
    const backgrounds = ruleBodies(indexCss, selector).flatMap((body) => [...declarations(body).keys()].filter((p) => p.startsWith("background")));
    expect(backgrounds).toEqual([]);
  });

  it("dims a blocked Compose step that stays focusable, instead of a disabled one", () => {
    const blocked = rule(indexCss, '.ab-step-pill[aria-disabled="true"]');
    expect(Number(blocked.get("opacity"))).toBeLessThan(0.6);
    expect(blocked.get("cursor")).toBe("not-allowed");
    expect(ruleBody(indexCss, ".ab-step-pill:disabled")).toBeNull();
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
