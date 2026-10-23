import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement } from "react";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Check } from "lucide-react";
import type { WcsInfo } from "../../../services/astrometry";

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
  };
  return { run, impl };
});

const clipboardWrite = vi.hoisted(() => vi.fn<(text: string) => Promise<void>>());
const wcsInfoMock = vi.hoisted(() => vi.fn<(path: string) => Promise<unknown>>());
const plateSolveMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());

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
vi.mock("../../../services/astrometry", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/astrometry")>()),
  getWcsInfo: wcsInfoMock,
  plateSolve: plateSolveMock,
  cancelPlateSolve: () => Promise.resolve(),
}));
vi.mock("../../../services/config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/config")>()),
  getApiKey: () => Promise.resolve({ key: "test-key" }),
  getConfig: () => Promise.resolve({ plate_solve_timeout_secs: 120 }),
}));

vi.stubGlobal("navigator", { clipboard: { writeText: clipboardWrite } });

import PlateSolvePanel, { SolveActions, SolveCentreCards } from "../PlateSolvePanel";
import { solveSummaryLine } from "../../../utils/coordFormat";

function inputTitled(markup: string, titleStart: string): string {
  const input = markup.match(new RegExp(`<input[^>]*title="${titleStart}[^"]*"[^>]*>`));
  expect(input, `input titled "${titleStart}..."`).not.toBeNull();
  return input![0];
}

function buttonEndingWith(markup: string, text: string): string {
  const buttons = markup.match(/<button[^>]*>(?:(?!<\/button>)[\s\S])*<\/button>/g) ?? [];
  const button = buttons.find((b) => b.endsWith(`${text}</button>`));
  expect(button, `button ending with "${text}"`).toBeDefined();
  return button!;
}

describe("PlateSolvePanel position hint fields", () => {
  const markup = renderToStaticMarkup(createElement(PlateSolvePanel, { filePath: "C:/x.fits", solvePath: "C:/x.fits" }));

  it.each(["Right ascension", "Declination", "Search radius"])(
    "keeps the %s field a decimal text box, so a typed comma reaches the parser instead of becoming an empty value",
    (titleStart) => {
      const input = inputTitled(markup, titleStart);
      expect(input).toContain('type="text"');
      expect(input).toContain('inputMode="decimal"');
      expect(input).not.toMatch(/\s(min|max|step)="/);
    },
  );
});

describe("PlateSolvePanel solve target", () => {
  it("renders the solve badge in the Plate Solve header", () => {
    const markup = renderToStaticMarkup(
      createElement(PlateSolvePanel, {
        filePath: "/data/frame1.fits",
        solvePath: "/out/stack.fits",
        solveBadge: createElement("b", null, "stack-badge"),
      }),
    );
    expect(markup).toMatch(/Plate Solve\s*<\/span><b>stack-badge<\/b>/);
  });

  it("disables the solve while there is no measured image to solve", () => {
    const markup = renderToStaticMarkup(createElement(PlateSolvePanel, { filePath: "/data/frame1.fits", solvePath: null }));
    expect(buttonEndingWith(markup, "Plate Solve")).toMatch(/^<button[^>]*\sdisabled=""/);
  });

  it("enables the solve on the measured image", () => {
    const markup = renderToStaticMarkup(createElement(PlateSolvePanel, { filePath: "/data/frame1.fits", solvePath: "/out/stack.fits" }));
    expect(buttonEndingWith(markup, "Plate Solve")).not.toMatch(/^<button[^>]*\sdisabled=""/);
  });
});

type Props = Record<string, unknown>;
type Component = (props: Props) => ReactNode;

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

function renderTree(component: Component, props: Props): ReactNode {
  hooks.run.cursor = 0;
  hooks.run.effects = [];
  const tree = component(props);
  for (const effect of hooks.run.effects) effect();
  return tree;
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function unescapeHtml(text: string): string {
  return text.replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&amp;/g, "&");
}

function textWith(markup: string, attribute: string): string {
  const match = markup.match(new RegExp(`<div[^>]*\\s${attribute}="[^"]*"[^>]*>([^<]*)</div>`));
  expect(match, `element with ${attribute}`).not.toBeNull();
  return unescapeHtml(match![1]);
}

const EDGE_CENTRE = {
  center_ra: 15 * (5 + 35 / 60 + 59.9996 / 3600),
  center_dec: -(5 + 23 / 60 + 59.96 / 3600),
  pixel_scale_arcsec: 0.0629,
  field_of_view_w_arcmin: 2.24,
  field_of_view_h_arcmin: 2.31,
};
const EDGE_LINE = "83.999998 -5.399989 deg ICRS | 05h36m00.00s -05°24'00.0\" | 0.06290\"/px | FOV 2.2' x 2.3'";
const COPY_CENTRE_LABEL = "Copy centre, scale and FOV";
const CLIPBOARD_DENIED = "Document is not focused.";

const cards = SolveCentreCards as unknown as Component;
const copyButton = (tree: ReactNode) => findElement(tree, (props) => props["aria-label"] === COPY_CENTRE_LABEL);
const copyStatusText = (tree: ReactNode) => findElement(tree, (props) => props["data-copy-status"] === true)?.children ?? null;
const copyErrorText = (tree: ReactNode) => findElement(tree, (props) => props["data-copy-error"] === true)?.children ?? null;
const showsCopied = (tree: ReactNode) => (copyButton(tree)!.children as ReactElement).type === Check;

describe("PlateSolvePanel centre cards", () => {
  const show = () => renderTree(cards, { centre: EDGE_CENTRE, source: "header" });

  beforeEach(() => {
    clipboardWrite.mockReset().mockResolvedValue(undefined);
    hooks.run.cells = [];
  });

  afterEach(() => {
    hooks.run.active = false;
    vi.useRealTimers();
  });

  it.each(["solve", "header"] as const)(
    "prints the %s centre with the WCS readout's rounding and adds decimal degrees",
    (source) => {
      const markup = renderToStaticMarkup(createElement(SolveCentreCards, { centre: EDGE_CENTRE, source }));
      expect(textWith(markup, "data-centre-ra")).toBe("05h36m00.00s");
      expect(textWith(markup, "data-centre-dec")).toBe("-05°24'00.0\"");
      expect(textWith(markup, "data-centre-ra-deg")).toBe("83.999998°");
      expect(textWith(markup, "data-centre-dec-deg")).toBe("-5.399989°");
      expect(unescapeHtml(markup)).not.toContain("60.0");
      expect(markup).toContain(`data-centre-source="${source}"`);
    },
  );

  it("labels the centre cards with the panel's own spelling", () => {
    const markup = renderToStaticMarkup(createElement(SolveCentreCards, { centre: EDGE_CENTRE, source: "solve" }));
    expect(markup).toContain("Centre RA");
    expect(markup).toContain("Centre Dec");
    expect(markup).not.toContain("Center");
  });

  it("keeps the pixel scale and FOV cards", () => {
    const markup = unescapeHtml(renderToStaticMarkup(createElement(SolveCentreCards, { centre: EDGE_CENTRE, source: "solve" })));
    expect(markup).toContain('0.06290"/px');
    expect(markup).toContain("2.2' x 2.3'");
  });

  it("leaves out the pixel scale and FOV cards when the WCS does not give them", () => {
    const centre = { center_ra: EDGE_CENTRE.center_ra, center_dec: EDGE_CENTRE.center_dec, pixel_scale_arcsec: null };
    const markup = renderToStaticMarkup(createElement(SolveCentreCards, { centre, source: "header" }));
    expect(markup).not.toContain("Pixel Scale");
    expect(markup).not.toContain("FOV</div>");
  });

  it("offers no copy for a centre the WCS could not compute", () => {
    const centre = { ...EDGE_CENTRE, center_ra: null, center_dec: null } as unknown as typeof EDGE_CENTRE;
    const markup = unescapeHtml(renderToStaticMarkup(createElement(SolveCentreCards, { centre, source: "header" })));
    expect(markup).not.toContain(COPY_CENTRE_LABEL);
    expect(textWith(markup, "data-centre-ra")).toBe("—");
    expect(markup).toContain('0.06290"/px');
  });

  it("copies one line with the centre in both systems, the pixel scale and the FOV", async () => {
    hooks.run.active = true;
    const copy = copyButton(show());
    expect(copy, "copy button").not.toBeNull();
    expect(copy!.type).toBe("button");
    (copy!.onClick as () => void)();
    await vi.waitFor(() => expect(clipboardWrite).toHaveBeenCalledTimes(1));
    expect(clipboardWrite.mock.calls[0][0]).toBe(solveSummaryLine(EDGE_CENTRE));
    expect(clipboardWrite.mock.calls[0][0]).toBe(EDGE_LINE);
  });

  it("names a refused clipboard write instead of leaving the previous clipboard text unannounced", async () => {
    clipboardWrite.mockRejectedValue(new DOMException(CLIPBOARD_DENIED, "NotAllowedError"));
    hooks.run.active = true;
    expect(copyErrorText(show())).toBeNull();
    (copyButton(show())!.onClick as () => void)();
    await flush();
    const tree = show();
    expect(copyErrorText(tree)).toBe(`Clipboard copy failed: ${CLIPBOARD_DENIED}`);
    expect(showsCopied(tree)).toBe(false);
    expect(copyStatusText(tree)).toBe("");
  });

  it("clears the clipboard error once a copy succeeds", async () => {
    clipboardWrite.mockRejectedValueOnce(new DOMException(CLIPBOARD_DENIED, "NotAllowedError"));
    hooks.run.active = true;
    (copyButton(show())!.onClick as () => void)();
    await flush();
    expect(copyErrorText(show())).not.toBeNull();
    (copyButton(show())!.onClick as () => void)();
    await flush();
    expect(copyErrorText(show())).toBeNull();
  });

  it("announces the copy to assistive technology", async () => {
    hooks.run.active = true;
    expect(copyStatusText(show())).toBe("");
    (copyButton(show())!.onClick as () => void)();
    await flush();
    expect(copyStatusText(show())).toBe("Copied");
  });

  it("keeps the copied mark for the full time after a second click", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    hooks.run.active = true;
    (copyButton(show())!.onClick as () => void)();
    await vi.advanceTimersByTimeAsync(1000);
    (copyButton(show())!.onClick as () => void)();
    await vi.advanceTimersByTimeAsync(1000);
    expect(showsCopied(show())).toBe(true);
    await vi.advanceTimersByTimeAsync(600);
    expect(showsCopied(show())).toBe(false);
  });
});

const HEADER_WCS = {
  ...EDGE_CENTRE,
  fov_arcmin: [2.24, 2.31],
  naxis1: 2136,
  naxis2: 2202,
} as WcsInfo;

const SOLVE_RESULT = {
  center_ra: 274.70012,
  center_dec: -13.84988,
  pixel_scale_arcsec: 0.06283,
  field_of_view_w_arcmin: 2.237,
  field_of_view_h_arcmin: 2.306,
  orientation: 12.5,
  annotations: [],
  wcs_cards: [],
};

describe("PlateSolvePanel centre blocks in the panel", () => {
  const panel = (PlateSolvePanel as unknown as { type: Component }).type;
  const show = () => renderTree(panel, { filePath: "C:/data/f444w_i2d.fits", solvePath: "C:/data/f444w_i2d.fits" });
  const centreCards = (tree: ReactNode) => findElement(tree, (props) => props.source === "header" || props.source === "solve");

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    wcsInfoMock.mockReset().mockResolvedValue(HEADER_WCS);
    plateSolveMock.mockReset().mockResolvedValue(SOLVE_RESULT);
  });

  afterEach(() => {
    hooks.run.active = false;
  });

  it("shows the header WCS centre through SolveCentreCards before any solve", async () => {
    show();
    await flush();
    const block = centreCards(show());
    expect(block, "centre cards in the panel").not.toBeNull();
    expect(block!.source).toBe("header");
    expect(block!.centre).toBe(HEADER_WCS);
    hooks.run.active = false;
    expect(renderToStaticMarkup(createElement(SolveCentreCards, block as never))).toContain('data-centre-source="header"');
  });

  it("shows the solve result through SolveCentreCards after a solve", async () => {
    show();
    await flush();
    const actions = findElement(show(), (props) => typeof props.onSolve === "function");
    expect(actions, "solve actions").not.toBeNull();
    (actions!.onSolve as () => void)();
    await flush();
    const block = centreCards(show());
    expect(plateSolveMock).toHaveBeenCalledTimes(1);
    expect(block, "centre cards in the panel").not.toBeNull();
    expect(block!.source).toBe("solve");
    expect(block!.centre).toBe(SOLVE_RESULT);
  });
});

describe("PlateSolvePanel cancel", () => {
  const noop = () => {};
  const actions = (loading: boolean) =>
    renderToStaticMarkup(
      createElement(SolveActions, { loading, disabled: loading, elapsedMs: 3000, timeoutSecs: 120, onSolve: noop, onCancel: noop }),
    );

  it("offers a Cancel button next to the running solve", () => {
    const markup = actions(true);
    const cancel = buttonEndingWith(markup, "Cancel");
    expect(cancel).toContain('type="button"');
    expect(cancel).toContain('aria-label="Cancel plate solve"');
    expect(cancel).not.toMatch(/\sdisabled=""/);
    expect(markup).toMatch(/Solving\.\.\. 0:03 \/ 2:00/);
  });

  it("shows no Cancel button while no solve runs", () => {
    const markup = actions(false);
    expect(markup).not.toContain("Cancel");
    expect(buttonEndingWith(markup, "Plate Solve")).not.toMatch(/^<button[^>]*\sdisabled=""/);
  });
});
