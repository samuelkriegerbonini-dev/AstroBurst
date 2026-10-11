import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import { themeColor } from "../themeColor";

const SOURCES = import.meta.glob<string>(["../../**/*.ts", "../../**/*.tsx", "!**/__tests__/**", "!../themeColor.ts"], {
  query: "?raw",
  import: "default",
  eager: true,
});
const CSS = readFileSync(fileURLToPath(new URL("../../index.css", import.meta.url)), "utf8");
const ROOT_BLOCK = /:root\s*\{([^}]*)\}/.exec(CSS)?.[1] ?? "";

function stubRootStyle(values: Record<string, string>) {
  const root = {};
  vi.stubGlobal("document", { documentElement: root });
  vi.stubGlobal("getComputedStyle", (el: unknown) => ({
    getPropertyValue: (name: string) => (el === root ? values[name] ?? "" : ""),
  }));
}

describe("themeColor", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("reads the token from the root element's computed style", () => {
    stubRootStyle({ "--ab-text-3": "#94949e", "--ab-text-4": "#8b8b94" });
    expect(themeColor("--ab-text-3", "#000000")).toBe("#94949e");
    expect(themeColor("--ab-text-4", "#000000")).toBe("#8b8b94");
  });

  it("trims the whitespace the computed value keeps after the colon", () => {
    stubRootStyle({ "--ab-text-3": "  #94949e " });
    expect(themeColor("--ab-text-3", "#000000")).toBe("#94949e");
  });

  it("follows a theme change instead of caching the first value", () => {
    stubRootStyle({ "--ab-text-3": "#94949e" });
    expect(themeColor("--ab-text-3", "#000000")).toBe("#94949e");
    stubRootStyle({ "--ab-text-3": "#a0a0aa" });
    expect(themeColor("--ab-text-3", "#000000")).toBe("#a0a0aa");
  });

  it("returns the fallback when the token is not defined", () => {
    stubRootStyle({});
    expect(themeColor("--ab-missing", "#8b8b94")).toBe("#8b8b94");
  });

  it("returns the fallback outside a DOM", () => {
    expect(typeof document).toBe("undefined");
    expect(themeColor("--ab-text-3", "#94949e")).toBe("#94949e");
  });
});

describe("themeColor call sites", () => {
  it("pass a literal token name and the token's own index.css value as the fallback", () => {
    const calls = Object.entries(SOURCES).flatMap(([file, text]) => {
      const total = text.split("themeColor(").length - 1;
      const literal = [...text.matchAll(/themeColor\(\s*"(--[\w-]+)"\s*,\s*"([^"]+)"\s*\)/g)];
      return [
        ...(literal.length === total ? [] : [{ file, issue: "non-literal call" }]),
        ...literal.map((m) => ({ file, name: m[1], fallback: m[2] })),
      ];
    });
    expect(calls.length).toBeGreaterThan(0);
    const mismatched = calls.filter((c) => {
      if (!("name" in c)) return true;
      const token = new RegExp(`(?:^|[\\s;{])${c.name}\\s*:\\s*([^;]+);`).exec(ROOT_BLOCK)?.[1].trim();
      return token?.toLowerCase() !== c.fallback.toLowerCase();
    });
    expect(mismatched).toEqual([]);
  });
});
