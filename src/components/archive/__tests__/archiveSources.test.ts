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
  const start = css.indexOf("@media (prefers-reduced-motion: reduce)");
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
