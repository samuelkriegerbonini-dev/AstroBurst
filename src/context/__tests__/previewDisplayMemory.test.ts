import { beforeEach, describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../CompositeContext";
import { PreviewProvider, useDisplayContext } from "../PreviewContext";
import { displayMemory } from "../../utils/displayMemoryStore";
import { emptyDisplayMemory, rememberLook, setDefaults, setSameGroup } from "../../utils/displayMemory";
import type { DisplayLook } from "../../shared/types/display";
import type { ProcessedFile } from "../../shared/types";

const FACTORY: DisplayLook = {
  stretch: "mtf",
  limits: "minmax",
  percentileLow: 1,
  percentileHigh: 99.5,
  userLo: null,
  userHi: null,
  zscaleContrast: 0.25,
  asinhA: 0.1,
  power: 2,
  colormap: "gray",
  invert: false,
  symmetric: false,
  centre: 0,
};
const ASINH_MAGMA: DisplayLook = { ...FACTORY, stretch: "asinh", limits: "zscale", colormap: "magma" };
const LINEAR_MAGMA: DisplayLook = { ...FACTORY, stretch: "linear", limits: "minmax", colormap: "magma" };

function fits(id: string, path: string, header: Record<string, string>): ProcessedFile {
  return {
    id,
    name: path.slice(path.lastIndexOf("/") + 1),
    path,
    sourcePath: path,
    imageRef: null,
    size: 0,
    status: "done",
    result: { png_path: `${path}.png`, previewUrl: `asset://${id}.png`, dimensions: [4, 4], elapsed_ms: 1, header },
    error: null,
    startedAt: null,
    finishedAt: null,
  } as ProcessedFile;
}

const N502 = fits("1", "C:/data/502nmos.fits", { INSTRUME: "WFPC2", FILTNAM1: "F502N" });
const N502B = fits("2", "C:/data/502nmos_b.fits", { INSTRUME: "WFPC2", FILTNAM1: "F502N" });
const N656 = fits("3", "C:/data/656nmos.fits", { INSTRUME: "WFPC2", FILTNAM1: "F656N" });
const NO_INSTRUMENT = fits("4", "C:/data/502_bare.fits", { FILTNAM1: "F502N" });

function Probe() {
  const { display, linear } = useDisplayContext();
  return createElement("output", null, `${display.stretch}|${display.colormap}|${linear ? "L" : "-"}`);
}

function shown(file: ProcessedFile | null): string {
  const preview = createElement(PreviewProvider, { file, doneFiles: [], children: createElement(Probe) });
  const html = renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
  return /<output>([^<]*)<\/output>/.exec(html)?.[1] ?? "";
}

beforeEach(() => {
  displayMemory.update(() => emptyDisplayMemory(null));
});

describe("PreviewProvider per-file display", () => {
  it("a file with a stored look renders with it on the first render", () => {
    displayMemory.update((d) => rememberLook(d, N502.path, "WFPC2|F502N", ASINH_MAGMA, null));
    expect(shown(N502)).toBe("asinh|magma|-");
    displayMemory.update((d) => rememberLook(d, N502.path, "WFPC2|F502N", LINEAR_MAGMA, { stretch: "asinh", limits: "zscale" }));
    expect(shown(N502)).toBe("linear|magma|L");
    expect(shown(null)).toBe("mtf|gray|-");
  });

  it("an untouched file takes its group look with sameGroup on and the defaults with it off", () => {
    displayMemory.update((d) => rememberLook(d, N502.path, "WFPC2|F502N", ASINH_MAGMA, null));
    expect(shown(N502B)).toBe("asinh|magma|-");
    displayMemory.update((d) => rememberLook(d, N502.path, "WFPC2|F502N", LINEAR_MAGMA, { stretch: "mtf", limits: "minmax" }));
    expect(shown(N502B)).toBe("linear|magma|L");
    displayMemory.update((d) => setSameGroup(d, false));
    expect(shown(N502B)).toBe("mtf|gray|-");
    displayMemory.update((d) => setDefaults(d, { ...FACTORY, colormap: "heat" }));
    expect(shown(N502B)).toBe("mtf|heat|-");
    expect(shown(N502)).toBe("linear|magma|L");
  });

  it("a file of another filter does not take the group look", () => {
    displayMemory.update((d) => rememberLook(d, N502.path, "WFPC2|F502N", ASINH_MAGMA, null));
    expect(shown(N656)).toBe("mtf|gray|-");
    expect(shown(NO_INSTRUMENT)).toBe("mtf|gray|-");
  });
});
