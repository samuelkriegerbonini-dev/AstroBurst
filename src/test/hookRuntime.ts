type Deps = readonly unknown[] | undefined;

interface Cell {
  value?: unknown;
  deps?: Deps;
  set?: (next: unknown) => void;
  cleanup?: unknown;
  subscribed?: unknown;
  unsubscribe?: () => void;
}

export interface HookRun {
  active: boolean;
  cells: Cell[];
  cursor: number;
  effects: (() => void)[];
  notified: number;
}

const run: HookRun = { active: false, cells: [], cursor: 0, effects: [], notified: 0 };

const sameDeps = (a: Deps, b: Deps) => a !== undefined && b !== undefined && a.length === b.length && a.every((v, i) => Object.is(v, b[i]));

const cell = (): Cell => (run.cells[run.cursor++] ??= {});

function remember(compute: () => unknown, deps?: Deps): unknown {
  const c = cell();
  if (!("value" in c) || !sameDeps(c.deps, deps)) {
    c.value = compute();
    c.deps = deps;
  }
  return c.value;
}

function effect(create: () => unknown, deps?: Deps): void {
  const c = cell();
  if (sameDeps(c.deps, deps)) return;
  c.deps = deps;
  run.effects.push(() => {
    if (typeof c.cleanup === "function") c.cleanup();
    c.cleanup = create();
  });
}

const fakes: Record<string, (...args: never[]) => unknown> = {
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
  useCallback: (fn: unknown, deps?: Deps) => remember(() => fn, deps),
  useId: () => `id-${run.cursor++}`,
  useEffect: effect,
  useLayoutEffect: effect,
  useSyncExternalStore(subscribe: (onChange: () => void) => () => void, getSnapshot: () => unknown) {
    const c = cell();
    if (c.subscribed !== subscribe) {
      c.unsubscribe?.();
      c.subscribed = subscribe;
      c.unsubscribe = subscribe(() => void run.notified++);
    }
    return getSnapshot();
  },
};

export const hooks = {
  run,
  mount(): void {
    for (const c of run.cells) {
      if (typeof c?.cleanup === "function") c.cleanup();
      c?.unsubscribe?.();
    }
    run.cells = [];
    run.notified = 0;
  },
  render<P>(component: (props: P) => unknown, props: P): unknown {
    run.cursor = 0;
    run.effects = [];
    const wasActive = run.active;
    run.active = true;
    let tree: unknown;
    try {
      tree = component(props);
    } finally {
      run.active = wasActive;
    }
    const pending = run.effects;
    run.effects = [];
    for (const flush of pending) flush();
    return tree;
  },
};

export function reactWithFakeHooks(actual: Record<string, unknown>): Record<string, unknown> {
  const switched = Object.fromEntries(
    Object.entries(fakes).map(([name, fake]) => [
      name,
      (...args: never[]) => (run.active ? fake : (actual[name] as (...a: never[]) => unknown))(...args),
    ]),
  );
  return { ...actual, ...switched };
}
