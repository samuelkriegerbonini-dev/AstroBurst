import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import FirstRun from "../FirstRun";
import { SUGGESTIONS } from "../logic/suggestions";

describe("first-run suggestions", () => {
  it("keeps the three image searches first and adds a time series, a 1D spectrum and an IFU cube", () => {
    expect(SUGGESTIONS.map((s) => s.id)).toEqual(["m16-webb", "p2739-webb", "m16-hubble", "p1366-tso", "p2072-x1d", "p1266-ifu"]);
    expect(SUGGESTIONS.slice(3).map((s) => [s.mission, s.text, s.preset])).toEqual([
      ["jwst", "1366", "tso"],
      ["jwst", "2072", "spectra_1d"],
      ["jwst", "1266", "ifu_cubes"],
    ]);
    for (const s of SUGGESTIONS.slice(0, 3)) expect(s.preset).toBeUndefined();
    expect(SUGGESTIONS.find((s) => s.mission === "jwst")?.preset).toBeUndefined();
  });

  it("marks the science chips with their preset", () => {
    const html = renderToStaticMarkup(createElement(FirstRun, { ctl: { runSuggestion: vi.fn(), searchAround: vi.fn(), openRomanAccess: vi.fn() }, around: null }));
    expect(html.match(/data-testid="mast-suggestion"/g)).toHaveLength(6);
    expect(html).toContain('data-preset="tso"');
    expect(html).toContain('data-preset="spectra_1d"');
    expect(html).toContain('data-preset="ifu_cubes"');
    expect(html.match(/data-preset=/g)).toHaveLength(3);
    expect(html).toContain(">Programme 1366, Webb time series (WASP-39 b)<");
  });
});
