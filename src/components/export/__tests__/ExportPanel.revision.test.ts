import { beforeEach, describe, expect, it, vi } from "vitest";

const FILE = "C:\\out\\m16_wcs.fits";

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
    useEffect(effect: () => unknown, deps?: readonly unknown[]) {
      const c = cell();
      if (sameDeps(c.deps, deps)) return;
      c.deps = deps;
      run.effects.push(() => {
        if (typeof c.cleanup === "function") c.cleanup();
        c.cleanup = effect();
      });
    },
    useSyncExternalStore: (_subscribe: unknown, snapshot: () => unknown) => snapshot(),
  };
  return { run, impl };
});

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
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => FILE }));
vi.mock("../../../hooks/useRegionStore", () => ({ useRegionDoc: () => ({ regions: [], selectedId: null }) }));
vi.mock("../../../services/astrometry", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/astrometry")>()),
  getWcsInfo: wcsInfoMock,
}));

import ExportPanel from "../ExportPanel";
import { rememberSolvedWcs } from "../../../utils/solvedWcs";

const PROPS = { filePath: FILE, exportPath: FILE, stfParams: null, onExport: () => Promise.resolve() };

function render(): void {
  hooks.run.cursor = 0;
  hooks.run.effects = [];
  (ExportPanel as unknown as (props: Record<string, unknown>) => unknown)(PROPS);
  for (const effect of hooks.run.effects) effect();
}

describe("ExportPanel after Write WCS rewrites the exported copy", () => {
  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    wcsInfoMock.mockReset().mockResolvedValue({ pixel_scale_arcsec: 2.4 });
  });

  it("re-reads the pixel scale of the cutout when the file's WCS is rewritten", () => {
    render();
    expect(wcsInfoMock).toHaveBeenCalledTimes(1);
    render();
    expect(wcsInfoMock).toHaveBeenCalledTimes(1);
    rememberSolvedWcs("C:\\data\\m16.fits", FILE);
    render();
    expect(wcsInfoMock).toHaveBeenCalledTimes(2);
    expect(wcsInfoMock).toHaveBeenLastCalledWith(FILE);
    hooks.run.active = false;
  });
});
