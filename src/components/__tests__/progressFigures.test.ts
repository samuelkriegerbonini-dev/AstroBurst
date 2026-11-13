import { describe, expect, it } from "vitest";

const SOURCES = import.meta.glob<string>("../**/*.tsx", { query: "?raw", import: "default", eager: true });

const LIVE_PERCENT = /(?<!\$)\{progress\.percent\}%/;

function componentPath(key: string): string {
  return key.replace(/^(\.\.?\/)+/, "");
}

function proportionalPercentLines(path: string, text: string): string[] {
  const lines = text.split(/\r?\n/);
  return lines.flatMap((line, i) => {
    if (!LIVE_PERCENT.test(line) || line.includes("width")) return [];
    const previous = lines.slice(0, i).reverse().find((l) => l.trim() !== "") ?? "";
    return line.includes("tabular-nums") || previous.includes("tabular-nums") ? [] : [`${path}:${i + 1}`];
  });
}

describe("live progress percentages", () => {
  it("scans the component sources", () => {
    expect(Object.keys(SOURCES).length).toBeGreaterThan(100);
  });

  it("draws every live percentage with tabular figures, so the counter keeps its width", () => {
    const offenders = Object.entries(SOURCES)
      .map(([key, text]) => [componentPath(key), text] as const)
      .filter(([path]) => !path.split("/").includes("__tests__"))
      .flatMap(([path, text]) => proportionalPercentLines(path, text));
    expect(offenders).toEqual([]);
  });
});
