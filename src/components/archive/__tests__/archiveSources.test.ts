import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const COMPONENTS = import.meta.glob<string>("../*.tsx", { query: "?raw", import: "default", eager: true });
const MODULES = import.meta.glob<string>(["../state/*.ts", "../logic/*.ts"], { query: "?raw", import: "default", eager: true });

function read(rel: string): string {
  return readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
}

const archiveCss = read("../archive.css");
const indexCss = read("../../../index.css");

function rules(css: string): { selectors: string[]; body: string }[] {
  const out: { selectors: string[]; body: string }[] = [];
  for (const m of css.matchAll(/([^{}@]+)\{([^{}]*)\}/g)) out.push({ selectors: m[1].split(",").map((s) => s.trim()), body: m[2] });
  return out;
}

function reducedMotionBlock(css: string): string {
  return atBlock(css, css.indexOf("@media (prefers-reduced-motion: reduce)"));
}

function atBlock(css: string, start: number): string {
  if (start < 0) return "";
  let depth = 0;
  for (let i = css.indexOf("{", start); i < css.length; i += 1) {
    if (css[i] === "{") depth += 1;
    if (css[i] === "}") {
      depth -= 1;
      if (depth === 0) return css.slice(start, i + 1);
    }
  }
  return "";
}

function classesUsed(source: string): string[] {
  return [...new Set([...source.matchAll(/(?<![\w-])ab-[a-z0-9-]+/g)].map((m) => m[0]))];
}

describe("archive sources", () => {
  it("scans the archive components and modules", () => {
    expect(Object.keys(COMPONENTS).length).toBeGreaterThanOrEqual(14);
    expect(Object.keys(MODULES).length).toBeGreaterThanOrEqual(20);
  });

  it("define every ab- class they render in archive.css or index.css", () => {
    const css = `${archiveCss}\n${indexCss}`;
    const missing = Object.entries(COMPONENTS).flatMap(([file, text]) =>
      classesUsed(text)
        .filter((name) => !new RegExp(`\\.${name}(?![\\w-])`).test(css))
        .map((name) => `${file}: ${name}`),
    );
    expect(missing).toEqual([]);
  });

  it("give the loading skeleton the card height the gallery lays out with", () => {
    const heightOf = (selector: string) => /(?:^|;|\s)height:\s*(\d+)px/.exec(rules(archiveCss).find((r) => r.selectors.includes(selector))?.body ?? "")?.[1];
    const cardH = /const CARD_H = (\d+);/.exec(COMPONENTS["../ResultsGallery.tsx"])?.[1];
    expect(cardH).toBeDefined();
    expect(heightOf(".ab-mast-card")).toBe(cardH);
    expect(heightOf(".ab-mast-skeleton")).toBe(cardH);
  });

  it("keep results in view at short windows: Clear beside the active chips, chips scrolling within two rows, the note's action on its text line", () => {
    const body = (css: string, selector: string) => rules(css).find((r) => r.selectors.includes(selector))?.body ?? "";
    expect(body(archiveCss, ".ab-mast-active-row")).toMatch(/flex-wrap:\s*nowrap/);
    expect(body(archiveCss, ".ab-mast-summary-unknown")).toMatch(/display:\s*block/);
    const short = atBlock(archiveCss, archiveCss.search(/@media \(max-height: \d+px\)/));
    const chips = body(short, ".ab-mast-active-chips");
    expect(Number(/max-height:\s*(\d+)px/.exec(chips)?.[1] ?? Infinity)).toBeLessThanOrEqual(2 * 24 + 6);
    expect(chips).toMatch(/overflow-y:\s*auto/);
  });

  it("keep a whole first card in view at 900 x 600 with the filter panel open: quick filters share the filter row's lines, one row of active chips beside the open panel, a one-line unknown note, the measured short-window spacing", () => {
    const body = (list: { selectors: string[]; body: string }[], selector: string) => list.find((r) => r.selectors.includes(selector))?.body ?? "";
    const px = (text: string, prop: string) => Number(new RegExp(`(?:^|[;\\s])${prop}:\\s*(\\d+)px`).exec(text)?.[1] ?? Infinity);
    expect(body(rules(archiveCss), ".ab-mast-filters")).toMatch(/display:\s*contents/);
    const at = archiveCss.search(/@media \(max-height: \d+px\)/);
    const block = atBlock(archiveCss, at);
    const short = rules(block);
    const later = new Set(rules(archiveCss.slice(at + block.length)).flatMap((r) => r.selectors));
    expect(short.flatMap((r) => r.selectors).filter((selector) => later.has(selector))).toEqual([]);
    expect(px(body(short, '.ab-mast-querybar[data-filters-open="true"] .ab-mast-active-chips'), "max-height")).toBeLessThanOrEqual(24);
    expect(body(short, ".ab-mast-summary-unknown")).toMatch(/display:\s*flex/);
    expect(body(short, ".ab-mast-summary-unknown-text")).toMatch(/text-overflow:\s*ellipsis/);
    expect(body(short, ".ab-mast-summary-unknown-text")).toMatch(/white-space:\s*nowrap/);
    const querybar = body(short, ".ab-mast-querybar");
    const panel = body(short, ".ab-mast-filter-panel");
    expect([px(querybar, "gap"), px(querybar, "padding-top"), px(querybar, "padding-bottom")]).toEqual([4, 6, 6]);
    expect(px(body(short, ".ab-mast-filter-row"), "row-gap")).toBe(6);
    expect([px(panel, "gap"), px(panel, "padding-top"), px(panel, "padding-bottom")]).toEqual([6, 6, 6]);
    expect(px(body(short, ".ab-mast-filter-options"), "row-gap")).toBe(4);
  });

  it("use the grey tokens instead of #71717a or #52525b", () => {
    const offenders = [...Object.entries(COMPONENTS), ["archive.css", archiveCss] as const].filter(([, text]) => /#(71717a|52525b)\b/i.test(text)).map(([f]) => f);
    expect(offenders).toEqual([]);
  });

  it("stop every archive animation and transition under reduced motion", () => {
    const block = reducedMotionBlock(archiveCss);
    expect(block).not.toBe("");
    const outside = archiveCss.replace(block, "");
    const animated = rules(outside).filter((r) => /(^|;|\s)(animation|transition)\s*:/.test(r.body) && !/(animation|transition)\s*:\s*none/.test(r.body));
    expect(animated.length).toBeGreaterThan(0);
    const reduced = rules(block.slice(block.indexOf("{") + 1));
    const stopped = new Set(reduced.filter((r) => /animation\s*:\s*none/.test(r.body) && /transition\s*:\s*none/.test(r.body)).flatMap((r) => r.selectors));
    const missing = animated.flatMap((r) => r.selectors).filter((sel) => !stopped.has(sel));
    expect(missing).toEqual([]);
  });

  it("load archive.css from the archive root", () => {
    expect(COMPONENTS["../ArchiveRoot.tsx"]).toMatch(/import "\.\/archive\.css";/);
  });

  it("keep state and logic free of component imports so App can load them eagerly", () => {
    const offenders = Object.entries(MODULES).flatMap(([file, text]) =>
      [...text.matchAll(/^import\s+(?!type\b)[^;]*?from\s+"([^"]+)";/gm)]
        .map((m) => m[1])
        .filter((spec) => spec.startsWith(".") && (/\/[A-Z][A-Za-z]+$/.test(spec) || spec.endsWith(".tsx")))
        .map((spec) => `${file} imports ${spec}`),
    );
    expect(offenders).toEqual([]);
  });

  it("carry no comments, console calls or debugger statements", () => {
    const all = [...Object.entries(COMPONENTS), ...Object.entries(MODULES), ["archive.css", archiveCss] as const];
    const offenders = all.flatMap(([file, text]) => {
      const withoutUrls = text.replace(/[a-z]+:\/\/[^\s"'`]+/g, "");
      const bad = /(^|[^:"'`\w])\/\/|\/\*|\bconsole\.|\bdebugger\b/.test(withoutUrls);
      return bad ? [file] : [];
    });
    expect(offenders).toEqual([]);
  });
});
