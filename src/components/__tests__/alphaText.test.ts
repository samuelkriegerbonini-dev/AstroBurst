import { describe, expect, it } from "vitest";

const SOURCES = import.meta.glob<string>("../**/*.tsx", { query: "?raw", import: "default", eager: true });

const DIMMED_TEXT = /text-(red-400\/(60|70)|blue-400\/(60|70)|sky-400\/60|violet-400\/(60|70)|emerald-500\/(60|70)|emerald-400\/60|cyan-400\/60|teal-400\/60|green-400\/60)\b/;

function componentPath(key: string): string {
  return key.replace(/^(\.\.?\/)+/, "");
}

describe("dimmed status text", () => {
  it("scans the component sources", () => {
    expect(Object.keys(SOURCES).length).toBeGreaterThan(100);
  });

  it("keeps informational text at 4.5:1 or more: no 60 or 70 percent alpha on the colours that fall below it", () => {
    const offenders = Object.entries(SOURCES)
      .map(([key, text]) => [componentPath(key), text] as const)
      .filter(([path]) => !path.split("/").includes("__tests__"))
      .flatMap(([path, text]) =>
        text.split(/\r?\n/).flatMap((line, i) => (DIMMED_TEXT.test(line) && !line.includes("size={") ? [`${path}:${i + 1}`] : [])),
      );
    expect(offenders).toEqual([]);
  });
});
