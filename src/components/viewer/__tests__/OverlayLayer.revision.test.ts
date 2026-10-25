import { beforeEach, describe, expect, it, vi } from "vitest";

const FILE = "C:\\data\\m16.fits";
const WCS_COPY = "C:\\out\\m16_wcs.fits";

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
  const effect = (run_: () => unknown, deps?: readonly unknown[]) => {
    const c = cell();
    if (sameDeps(c.deps, deps)) return;
    c.deps = deps;
    run.effects.push(() => {
      if (typeof c.cleanup === "function") c.cleanup();
      c.cleanup = run_();
    });
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
    useEffect: effect,
    useLayoutEffect: effect,
    useSyncExternalStore: (_subscribe: unknown, snapshot: () => unknown) => snapshot(),
  };
  return { run, impl };
});

const gridLinesMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
const wcsInfoMock = vi.hoisted(() => vi.fn<(path: string) => Promise<unknown>>());

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
vi.mock("../../../context/PreviewContext", () => ({
  useDisplayContext: () => ({ display: { grid: true, gridFrame: "icrs", gridDensity: 3, compass: true } }),
  useDisplayedImage: () => ({ path: WCS_COPY, dimensions: [2000, 1500], isProcessed: true, label: null, previewOnly: false, previewUrl: null }),
}));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => FILE }));
vi.mock("../../../hooks/useOverlayStore", () => ({ useOverlayDoc: () => ({ layers: [] }) }));
vi.mock("../../../services/astrometry", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/astrometry")>()),
  gridLines: gridLinesMock,
  getWcsInfo: wcsInfoMock,
}));

import OverlayLayer from "../OverlayLayer";
import { rememberSolvedWcs } from "../../../utils/solvedWcs";

type Component = (props: Record<string, unknown>) => unknown;

const PROPS = {
  containerRef: { current: null },
  transform: { scale: 1, x: 0, y: 0 },
  renderW: 2000,
  renderH: 1500,
  fitsW: 2000,
  fitsH: 1500,
  enabled: true,
};

function render(): void {
  const inner = (OverlayLayer as unknown as { type: Component }).type;
  hooks.run.cursor = 0;
  hooks.run.effects = [];
  inner(PROPS);
  for (const effect of hooks.run.effects) effect();
}

const GRID = { lines: [], labels: [], frame: "icrs" };
const INFO = { north_vec: [0, -1], east_vec: [-1, 0], pixel_scale_arcsec: 2.4, naxis1: 2000, naxis2: 1500 };

describe("viewer overlay after Write WCS rewrites the displayed copy", () => {
  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    gridLinesMock.mockReset().mockResolvedValue(GRID);
    wcsInfoMock.mockReset().mockResolvedValue(INFO);
  });

  it("re-requests the grid and the compass of the displayed file when its WCS is rewritten in place", () => {
    render();
    expect(gridLinesMock).toHaveBeenCalledTimes(1);
    expect(wcsInfoMock).toHaveBeenCalledTimes(1);
    render();
    expect(gridLinesMock).toHaveBeenCalledTimes(1);
    rememberSolvedWcs(FILE, WCS_COPY);
    render();
    expect(gridLinesMock).toHaveBeenCalledTimes(2);
    expect(gridLinesMock.mock.calls[1]).toEqual([WCS_COPY, "icrs", 3]);
    expect(wcsInfoMock).toHaveBeenCalledTimes(2);
    hooks.run.active = false;
  });

  it("ignores a WCS written for another file", () => {
    render();
    rememberSolvedWcs("C:\\data\\other.fits", "C:\\out\\other_wcs.fits");
    render();
    expect(gridLinesMock).toHaveBeenCalledTimes(1);
    hooks.run.active = false;
  });
});
