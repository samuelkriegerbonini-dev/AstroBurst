import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { DqFlagTableResult } from "../../../services/display";

vi.mock("../../../context/PreviewContext", () => ({
  useDqContext: () => {
    throw new Error("DqFlagMenu takes its state as props");
  },
}));

import { DqFlagMenu } from "../DqControls";

const UNKNOWN: DqFlagTableResult = {
  table: "unknown",
  label: "unknown (raw bits)",
  flags: [],
  default_mask: 0,
  exclusion_mask: 0,
  dq_ref: "hsc.fits#hdu=2",
  err_ref: null,
  bits_present: 0b101,
};

const JWST: DqFlagTableResult = {
  table: "jwst",
  label: "jwst",
  flags: [
    { bit: 1, name: "DO_NOT_USE" },
    { bit: 2, name: "SATURATED" },
  ],
  default_mask: 7,
  exclusion_mask: 1,
  dq_ref: "cal.fits#hdu=3",
  err_ref: "cal.fits#hdu=2",
  bits_present: null,
};

function menu(flagTable: DqFlagTableResult, mask: number): string {
  return renderToStaticMarkup(
    createElement(DqFlagMenu, {
      flagTable,
      overlay: { enabled: false, mask },
      setOverlay: () => {},
      excludeDq: false,
      setExcludeDq: () => {},
      dqMaskError: null,
    }),
  );
}

function flagRows(markup: string): { name: string; checked: boolean }[] {
  return [...markup.matchAll(/<input([^>]*)\/?>\s*<span[^>]*>([^<]*)<\/span>\s*<span[^>]*>\d+<\/span>/g)].map((m) => ({
    name: m[2],
    checked: /\schecked=""/.test(m[1]),
  }));
}

describe("DqFlagMenu", () => {
  it("lists the raw bits present in a plane of unknown origin, none checked, and says nothing is excluded", () => {
    const markup = menu(UNKNOWN, UNKNOWN.default_mask);
    expect(flagRows(markup)).toEqual([
      { name: "BIT 0", checked: false },
      { name: "BIT 2", checked: false },
    ]);
    expect(markup).not.toContain("DO_NOT_USE");
    expect(markup).toMatch(/title="[^"]*unknown origin[^"]*nothing is excluded[^"]*"/);
  });

  it("says so when an unknown plane has no bit set", () => {
    const markup = menu({ ...UNKNOWN, bits_present: 0 }, 0);
    expect(flagRows(markup)).toEqual([]);
    expect(markup).toContain("no bits set in this plane");
  });

  it("keeps the named flags of a known table", () => {
    const markup = menu(JWST, 1);
    expect(flagRows(markup)).toEqual([
      { name: "DO_NOT_USE", checked: true },
      { name: "SATURATED", checked: false },
    ]);
    expect(markup).toContain("exclude DO_NOT_USE from stats");
    expect(markup).not.toContain("unknown origin");
  });
});
