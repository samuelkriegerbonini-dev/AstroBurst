import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement } from "react";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { GainModel } from "../../../services/analysis";

const hooks = vi.hoisted(() => {
  type Cell = { value?: unknown; deps?: readonly unknown[]; set?: (next: unknown) => void; cleanup?: unknown };
  const run = { active: false, cells: [] as Cell[], cursor: 0, effects: [] as (() => void)[] };
  const sameDeps = (a: readonly unknown[] | undefined, b: readonly unknown[] | undefined) =>
    a !== undefined && b !== undefined && a.length === b.length && a.every((v, i) => Object.is(v, b[i]));
  const cell = (): Cell => (run.cells[run.cursor++] ??= {});
  const remember = (compute: () => unknown, deps?: readonly unknown[]) => {
    const c = cell();
    if (!("value" in c) || !sameDeps(c.deps, deps)) {
      c.value = compute();
      c.deps = deps;
    }
    return c.value;
  };
  const impl = {
    useState(init: unknown) {
      const c = cell();
      if (!("value" in c)) c.value = typeof init === "function" ? (init as () => unknown)() : init;
      c.set ??= (next: unknown) => {
        c.value = typeof next === "function" ? (next as (prev: unknown) => unknown)(c.value) : next;
      };
      return [c.value, c.set];
    },
    useRef: (init: unknown) => remember(() => ({ current: init }), []),
    useMemo: remember,
    useCallback: (fn: unknown, deps?: readonly unknown[]) => remember(() => fn, deps),
    useId: () => `id-${run.cursor++}`,
    useSyncExternalStore: (_subscribe: unknown, getSnapshot: () => unknown) => getSnapshot(),
    useEffect(effect: () => unknown, deps?: readonly unknown[]) {
      const c = cell();
      if (sameDeps(c.deps, deps)) return;
      c.deps = deps;
      run.effects.push(() => {
        if (typeof c.cleanup === "function") c.cleanup();
        c.cleanup = effect();
      });
    },
  };
  return { run, impl };
});

const gainModelMock = vi.hoisted(() => vi.fn<(path: string) => Promise<GainModel>>());
const measureMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
const batchMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
const pixelClick = vi.hoisted(() => ({ current: null as { seq: number; x: number; y: number } | null }));

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<Record<string, unknown>>();
  const fakes = hooks.impl as unknown as Record<string, (...args: unknown[]) => unknown>;
  const switched = Object.fromEntries(
    Object.entries(fakes).map(([name, fake]) => [
      name,
      (...args: unknown[]) => (hooks.run.active ? fake : (actual[name] as (...a: unknown[]) => unknown))(...args),
    ]),
  );
  return { ...actual, ...switched };
});
vi.mock("../../../services/analysis", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/analysis")>()),
  photometryGainModel: gainModelMock,
  measurePhotometry: measureMock,
  measurePhotometryBatch: batchMock,
}));
vi.mock("../../../services/astrometry", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/astrometry")>()),
  getWcsInfo: () => new Promise(() => {}),
}));
vi.mock("../../../context/ToolHostContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../context/ToolHostContext")>()),
  useToolHost: () => ({ active: true, gpuDisplay: true }),
}));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => ({ excludeDq: false }) }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "502nmos.fits", image: "original" }) }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({ useMeasurementSource: () => null }));
vi.mock("../../../hooks/useMousePixelStore", () => ({ usePixelClick: () => pixelClick.current }));

import PhotometryPanel from "../PhotometryPanel";
import PhotometryTablePanel from "../PhotometryTablePanel";

const GAIN_TITLE_TEXT = "Gain in e-/ADU; prefilled from the header when known";

vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {}, removeItem: () => {} });

function gainInput(markup: string): string {
  const input = markup.match(new RegExp(`<input[^>]*title="${GAIN_TITLE_TEXT}"[^>]*>`));
  expect(input, "gain input with the header-prefill title").not.toBeNull();
  return input![0];
}

describe.each([
  ["PhotometryPanel", () => renderToStaticMarkup(createElement(PhotometryPanel, { filePath: "C:/data/502nmos.fits" }))],
  [
    "PhotometryTablePanel",
    () => renderToStaticMarkup(createElement(PhotometryTablePanel, { filePath: "C:/data/502nmos.fits", overlayKey: null, stars: [] })),
  ],
])("%s gain field", (_name, render) => {
  it("carries the header-prefill title on a decimal text box", () => {
    const input = gainInput(render());
    expect(input).toContain('type="text"');
    expect(input).toContain('inputMode="decimal"');
  });

  it("renders no number input, so a typed comma is never read as a thousands digit", () => {
    const markup = render();
    expect(markup).not.toContain('type="number"');
    expect(markup.match(/<input[^>]*inputMode="decimal"/g)?.length).toBe(4);
  });

  it("names no gain source before the header model is read", () => {
    expect(gainInput(render())).toContain('data-gain-source=""');
  });

  it("renders the gain caption element", () => {
    expect(render()).toMatch(/<div[^>]*\sdata-gain-caption="true"[^>]*>/);
  });
});

type Panel = (props: Record<string, unknown>) => ReactNode;
type GainField = { value: string; onChange: (text: string) => void };

function findGainField(node: ReactNode): GainField | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findGainField(child);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Record<string, unknown>>(node)) return null;
  if (node.props.title === GAIN_TITLE_TEXT) return node.props as unknown as GainField;
  return findGainField(node.props.children as ReactNode);
}

function renderWithHooks(panel: Panel, props: Record<string, unknown>): GainField {
  hooks.run.cursor = 0;
  hooks.run.effects = [];
  const tree = panel(props);
  for (const effect of hooks.run.effects) effect();
  const field = findGainField(tree);
  expect(field, "gain input in the rendered tree").not.toBeNull();
  return field!;
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const pending = () => new Promise<GainModel>(() => {});

function gainModel(overrides: Partial<GainModel>): GainModel {
  return {
    gain_e_per_adu: null,
    source: null,
    gain_card: null,
    unit_class: "counts",
    ncombine: null,
    combine_method: null,
    drizzle_scale: null,
    combine_scaled: false,
    effective_gain: null,
    fallback_gain: null,
    poisson_route: "unavailable",
    note: null,
    ...overrides,
  };
}

const ATODGAIN_7 = gainModel({ gain_e_per_adu: 7, source: "ATODGAIN", effective_gain: 7, poisson_route: "header_gain" });
const EGAIN_4 = gainModel({ gain_e_per_adu: 4, source: "EGAIN", effective_gain: 4, poisson_route: "header_gain" });
const ERR_PLANE = gainModel({ unit_class: "calibrated", poisson_route: "err_plane", note: "Poisson noise from the ERR plane" });

const FIRST = "C:/data/502nmos.fits";
const SECOND = "C:/data/f200w_i2d.fits";

describe.each([
  ["PhotometryPanel", PhotometryPanel.type as unknown as Panel, {}],
  ["PhotometryTablePanel", PhotometryTablePanel.type as unknown as Panel, { overlayKey: null, stars: [] }],
])("%s gain field across a file switch", (_name, panel, props) => {
  const show = (filePath: string) => renderWithHooks(panel, { ...props, filePath });

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset();
  });

  afterEach(() => {
    hooks.run.active = false;
  });

  it("clears the previous file's prefilled gain while the next file's model is loading", async () => {
    gainModelMock.mockResolvedValueOnce(ATODGAIN_7).mockReturnValueOnce(pending());
    show(FIRST);
    await flush();
    expect(show(FIRST).value).toBe("7");
    show(SECOND);
    expect(show(SECOND).value).toBe("");
  });

  it("keeps a typed gain while the next file's model is loading", async () => {
    gainModelMock.mockResolvedValueOnce(ATODGAIN_7).mockReturnValueOnce(pending());
    show(FIRST);
    await flush();
    show(FIRST).onChange("3.2");
    expect(show(FIRST).value).toBe("3.2");
    show(SECOND);
    expect(show(SECOND).value).toBe("3.2");
  });

  it("prefills the next file's header gain once its model arrives", async () => {
    gainModelMock.mockResolvedValueOnce(ATODGAIN_7).mockResolvedValueOnce(EGAIN_4);
    show(FIRST);
    await flush();
    show(SECOND);
    await flush();
    expect(show(SECOND).value).toBe("4");
  });

  it("leaves the field empty when the next model resolves before the panel renders again", async () => {
    gainModelMock.mockResolvedValueOnce(ATODGAIN_7).mockResolvedValueOnce(ERR_PLANE);
    show(FIRST);
    await flush();
    expect(show(FIRST).value).toBe("7");
    show(SECOND);
    await flush();
    expect(show(SECOND).value).toBe("");
  });
});

type Props = Record<string, unknown>;

function findElement(node: ReactNode, match: (props: Props) => boolean): Props | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findElement(child, match);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Props>(node)) return null;
  if (match(node.props)) return node.props;
  return findElement(node.props.children as ReactNode, match);
}

function renderTree(panel: Panel, props: Props): ReactNode {
  hooks.run.cursor = 0;
  hooks.run.effects = [];
  const tree = panel(props);
  for (const effect of hooks.run.effects) effect();
  return tree;
}

const APERTURE_TITLE_PREFIX = "Aperture radius in pixels";
const COMMA_ERROR = "r_ap px: Use a dot (.) as the decimal separator";
const byTitlePrefix = (prefix: string) => (props: Props) => typeof props.title === "string" && props.title.startsWith(prefix);
const fieldErrorText = (tree: ReactNode) => findElement(tree, (props) => props["data-field-error"] === true)?.children ?? null;

describe("PhotometryPanel with a decimal comma in the aperture", () => {
  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    measureMock.mockReset().mockReturnValue(new Promise(() => {}));
    pixelClick.current = null;
  });

  afterEach(() => {
    hooks.run.active = false;
    pixelClick.current = null;
  });

  it("names the field instead of measuring 2,5 as 25, then measures 2.5 once it is typed with a dot", () => {
    const show = () => renderTree(PhotometryPanel.type as unknown as Panel, { filePath: FIRST });
    let tree = show();
    (findElement(tree, (props) => props.label === "Measure on image click")!.onChange as (on: boolean) => void)(true);
    (findElement(show(), byTitlePrefix(APERTURE_TITLE_PREFIX))!.onChange as (text: string) => void)("2,5");
    pixelClick.current = { seq: 1, x: 10, y: 12 };
    tree = show();
    expect(fieldErrorText(tree)).toBe(COMMA_ERROR);
    expect(measureMock).not.toHaveBeenCalled();

    (findElement(tree, byTitlePrefix(APERTURE_TITLE_PREFIX))!.onChange as (text: string) => void)("2.5");
    pixelClick.current = { seq: 2, x: 10, y: 12 };
    tree = show();
    expect(fieldErrorText(tree)).toBeNull();
    expect(measureMock).toHaveBeenCalledTimes(1);
    expect(measureMock.mock.calls[0][3]).toMatchObject({ apertureRadius: 2.5 });
  });
});

describe("PhotometryTablePanel with a decimal comma in the aperture", () => {
  const STARS = [{ x: 10, y: 12, flux: 100, fwhm: 3, snr: 50 }];

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gainModelMock.mockReset().mockReturnValue(pending());
    batchMock.mockReset().mockReturnValue(new Promise(() => {}));
  });

  afterEach(() => {
    hooks.run.active = false;
  });

  it("disables Measure and names the field until the radius is typed with a dot", () => {
    const show = () => renderTree(PhotometryTablePanel.type as unknown as Panel, { filePath: FIRST, overlayKey: null, stars: STARS });
    const measureButton = (tree: ReactNode) => findElement(tree, (props) => props.label === "Measure 1")!;
    (findElement(show(), byTitlePrefix(APERTURE_TITLE_PREFIX))!.onChange as (text: string) => void)("2,5");
    let tree = show();
    expect(measureButton(tree).disabled).toBe(true);
    expect(fieldErrorText(tree)).toBe(COMMA_ERROR);
    (measureButton(tree).onClick as () => void)();
    expect(batchMock).not.toHaveBeenCalled();

    (findElement(tree, byTitlePrefix(APERTURE_TITLE_PREFIX))!.onChange as (text: string) => void)("2.5");
    tree = show();
    expect(measureButton(tree).disabled).toBe(false);
    (measureButton(tree).onClick as () => void)();
    expect(batchMock).toHaveBeenCalledTimes(1);
    expect(batchMock.mock.calls[0][2]).toMatchObject({ apertureRadius: 2.5 });
  });
});
