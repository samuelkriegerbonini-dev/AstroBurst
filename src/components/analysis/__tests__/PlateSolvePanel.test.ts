import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Fragment, createElement, isValidElement } from "react";
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
const solveHintsMock = vi.hoisted(() => vi.fn<(path: string) => Promise<unknown>>());
const apiKeyMock = vi.hoisted(() => vi.fn<() => Promise<{ key: string | null }>>(() => Promise.resolve({ key: "test-key" })));
const cancelSolveMock = vi.hoisted(() => vi.fn<() => Promise<boolean>>(() => Promise.resolve(true)));
const refineMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());
const cancelRefineMock = vi.hoisted(() => vi.fn<() => Promise<boolean>>(() => Promise.resolve(true)));
const refineProgress = vi.hoisted(() => ({ active: false, percent: 0, stage: "", current: 0, total: 0, reset: vi.fn() }));
const writeWcsMock = vi.hoisted(() => vi.fn<(...args: unknown[]) => Promise<unknown>>());

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
  getSolveHints: solveHintsMock,
  plateSolve: plateSolveMock,
  cancelPlateSolve: cancelSolveMock,
  refineWithGaia: refineMock,
  cancelGaiaRefine: cancelRefineMock,
  writeSolvedWcs: writeWcsMock,
}));
vi.mock("../../../hooks/useProgress", () => ({ useProgress: () => refineProgress }));
vi.mock("../../../services/config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/config")>()),
  getApiKey: apiKeyMock,
  getConfig: () => Promise.resolve({ plate_solve_timeout_secs: 120, astrometry_api_url: "https://nova.astrometry.net" }),
}));

vi.stubGlobal("navigator", { clipboard: { writeText: clipboardWrite } });

import PlateSolvePanel, { GaiaRefineBlock, SolveActions, SolveCentreCards } from "../PlateSolvePanel";
import { GAIA_REFINE_CANCELLED_NOTE, GAIA_REFINE_CONSENT, NO_WCS_TO_REFINE } from "../../../utils/solvedWcs";
import { solveSummaryLine } from "../../../utils/coordFormat";
import { TauriCommandError } from "../../../shared/types/errors";

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
  return text.replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
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
    solveHintsMock.mockReset().mockRejectedValue(new Error("no solve_hints_cmd"));
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

const LOCAL_RESULT = {
  ...SOLVE_RESULT,
  orientation: null,
  wcs_cards: [["CTYPE1", "'RA---TAN-SIP'"]],
  solver: "local",
  catalog: "tier0-tycho2",
  matched: 293,
  predicted: 410,
  rms_px: 0.0684,
  sip_order: 3,
  sip_max_err_px: 0.0041,
  hint_source: "OBJCTRA/OBJCTDEC",
  elapsed_ms: 778,
  parity: "normal",
  warnings: ["SIP order 4 was not tried: the field is under 5 degrees"],
  credit: null,
};

const DEAD_VIZIER = "VizieR (127.0.0.1:9) refused the connection; check the internet connection or a firewall, then try again";

const REVIEWED_SIRIL_CREDIT = {
  text: "Gaia DR3, ESA/Gaia/DPAC, CC BY-NC 3.0 IGO, via the Siril HEALPix extract (Zenodo 14692304)",
  citation: "Gaia DR3 (Gaia Collaboration, Vallenari et al. 2023, A&A 674, A1), ESA/Gaia/DPAC, CC BY-NC 3.0 IGO, via the Siril HEALPix extract (Zenodo 14692304)",
};

const NO_SOLUTION_TEXT = "No solution: the best candidate matched 14 stars with an RMS of 6.27 px";

const OBJCT_HINTS = {
  centre: { ra: 274.7, dec: -13.816667, source: "OBJCTRA/OBJCTDEC" },
  scale: { low_arcsec: 2.1537, high_arcsec: 2.6324, source: "XPIXSZ 2.9 µm / FOCALLEN 250 mm" },
  fov_deg: 3.6,
  epoch_year: 2024.5,
  cfa: null,
  object: null,
  warnings: ["XBINNING 2 read as both binnings"],
};

function markupOf(tree: ReactNode): string {
  const wasActive = hooks.run.active;
  hooks.run.active = false;
  try {
    return unescapeHtml(renderToStaticMarkup(createElement(Fragment, null, tree)));
  } finally {
    hooks.run.active = wasActive;
  }
}

describe("PlateSolvePanel local-first solve (M08)", () => {
  const panel = (PlateSolvePanel as unknown as { type: Component }).type;
  const show = () => renderTree(panel, { filePath: "C:/data/m16.fits", solvePath: "C:/data/m16.fits" });
  const solveActions = (tree: ReactNode) => findElement(tree, (props) => typeof props.onSolve === "function");
  const fallbackBlock = (tree: ReactNode) => findElement(tree, (props) => typeof props.onFallback === "function");

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    wcsInfoMock.mockReset().mockRejectedValue(new Error("no celestial WCS"));
    solveHintsMock.mockReset().mockResolvedValue(OBJCT_HINTS);
    plateSolveMock.mockReset().mockResolvedValue(LOCAL_RESULT);
    apiKeyMock.mockReset().mockResolvedValue({ key: "test-key" });
    cancelSolveMock.mockClear();
  });

  afterEach(() => {
    hooks.run.active = false;
    vi.useRealTimers();
  });

  const solveOnce = async () => {
    show();
    await flush();
    (solveActions(show())!.onSolve as () => void)();
    await flush();
    return show();
  };

  it("labels the centre fields with both accepted forms", () => {
    const markup = unescapeHtml(renderToStaticMarkup(createElement(PlateSolvePanel, { filePath: "C:/x.fits", solvePath: "C:/x.fits" })));
    expect(markup).toContain("Centre RA (deg or h:m:s)");
    expect(markup).toContain("Centre Dec (deg or d:m:s)");
  });

  it("does not ask for an astrometry.net key before a local solve has failed", async () => {
    apiKeyMock.mockResolvedValue({ key: null });
    show();
    await flush();
    expect(markupOf(show())).not.toMatch(/API key/);
  });

  it("solves on this computer by default through the auto chain, which may add the online Gaia tier, and never names astrometry.net", async () => {
    show();
    await flush();
    (solveActions(show())!.onSolve as () => void)();
    await flush();
    expect(plateSolveMock).toHaveBeenCalledTimes(1);
    const [path, options] = plateSolveMock.mock.calls[0] as [string, Record<string, unknown>];
    expect(path).toBe("C:/data/m16.fits");
    expect(options).toMatchObject({ solver: "auto", centerRa: 274.7, centerDec: -13.816667 });
  });

  it("caps the offline near-search radius at 15 degrees and keeps the full radius for astrometry.net", async () => {
    wcsInfoMock.mockResolvedValue({
      center_ra: 90,
      center_dec: -66,
      pixel_scale_arcsec: 21,
      fov_arcmin: [1440, 1440],
      naxis1: 4096,
      naxis2: 4096,
    } as WcsInfo);
    solveHintsMock.mockRejectedValue(new Error("no solve_hints_cmd"));
    show();
    await flush();
    expect(markupOf(show())).toContain('value="16.9706"');
    (solveActions(show())!.onSolve as () => void)();
    await flush();
    expect(plateSolveMock.mock.calls[0][1]).toMatchObject({ solver: "auto", radius: 16.9706, nearRadiusDeg: 15 });
  });

  it.each([
    [
      "local_missing_hint",
      "The offline solver needs an approximate centre and none was found in the header or the panel; type it in the hint fields, or try astrometry.net, which can solve without hints",
      "Not enough hints to solve offline",
    ],
    [
      "local_no_catalogue",
      "No offline star catalogue is available (none installed); install one in Settings > Catalogs",
      "No offline catalogue",
    ],
  ])("offers astrometry.net after the backend's %s error", async (code, message, heading) => {
    plateSolveMock.mockRejectedValueOnce(
      new TauriCommandError("plate_solve_cmd", message, code, { canFallback: true, apiKeyStored: true, missing: ["centre"] }),
    );
    const tree = await solveOnce();
    expect(fallbackBlock(tree), "fallback offered").not.toBeNull();
    const markup = markupOf(tree);
    expect(markup).toContain(heading);
    expect(markup).toContain(message);
    expect(markup).not.toContain("No offline solution");
    expect(buttonEndingWith(markup, "Try astrometry.net")).not.toMatch(/^<button[^>]*\sdisabled=""/);
  });

  it("re-reads the saved key at the click, so a key saved in Settings after the failure is used", async () => {
    apiKeyMock.mockResolvedValue({ key: null });
    plateSolveMock.mockRejectedValueOnce(
      new TauriCommandError("plate_solve_cmd", NO_SOLUTION_TEXT, "local_no_solution", { canFallback: true, apiKeyStored: false }),
    );
    const failed = await solveOnce();
    const markup = markupOf(failed);
    expect(markup).toContain("needs an astrometry.net API key: save it in Settings, then click Try astrometry.net.");
    expect(buttonEndingWith(markup, "Try astrometry.net")).not.toMatch(/^<button[^>]*\sdisabled=""/);
    apiKeyMock.mockResolvedValue({ key: "saved-later" });
    plateSolveMock.mockResolvedValueOnce(SOLVE_RESULT);
    (fallbackBlock(failed)!.onFallback as () => void)();
    await flush();
    expect(plateSolveMock).toHaveBeenCalledTimes(2);
    expect(plateSolveMock.mock.calls[1][1]).toMatchObject({ solver: "astrometry.net" });
  });

  it("uploads nothing and says why when no key is saved at the click", async () => {
    apiKeyMock.mockResolvedValue({ key: null });
    plateSolveMock.mockRejectedValueOnce(new TauriCommandError("plate_solve_cmd", NO_SOLUTION_TEXT, "local_no_solution", {}));
    const failed = await solveOnce();
    (fallbackBlock(failed)!.onFallback as () => void)();
    await flush();
    expect(plateSolveMock).toHaveBeenCalledTimes(1);
    const tree = show();
    expect(fallbackBlock(tree)).not.toBeNull();
    expect(markupOf(tree)).toContain("No astrometry.net API key is saved, so nothing was uploaded.");
  });

  it("takes the stored-key state from the backend's failure", async () => {
    apiKeyMock.mockResolvedValue({ key: null });
    plateSolveMock.mockRejectedValueOnce(
      new TauriCommandError("plate_solve_cmd", NO_SOLUTION_TEXT, "local_no_solution", { canFallback: true, apiKeyStored: true }),
    );
    expect(markupOf(await solveOnce())).toContain("Uploads this image to nova.astrometry.net with your API key; the solve runs there.");
  });

  it("cancels the offline solve when the panel's deadline fires, and offers astrometry.net", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    plateSolveMock.mockReturnValueOnce(new Promise(() => {}));
    show();
    await vi.advanceTimersByTimeAsync(0);
    (solveActions(show())!.onSolve as () => void)();
    await vi.advanceTimersByTimeAsync(119_000);
    expect(cancelSolveMock).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1_000);
    expect(cancelSolveMock).toHaveBeenCalledTimes(1);
    const tree = show();
    expect(fallbackBlock(tree), "fallback offered after the deadline").not.toBeNull();
    const markup = markupOf(tree);
    expect(markup).toContain("The offline solve ran out of time");
    expect(markup).toContain("The offline solve gave up after 2:00 (Settings > Timeout).");
  });

  it("fills the hints from solve_hints_cmd and names their source under the fields", async () => {
    show();
    await flush();
    const markup = markupOf(show());
    expect(solveHintsMock).toHaveBeenCalledWith("C:/data/m16.fits");
    expect(markup).toContain("Hints: centre from OBJCTRA/OBJCTDEC; scale from XPIXSZ 2.9 µm / FOCALLEN 250 mm");
    expect(markup).toContain('value="274.700000"');
    expect(markup).toContain('value="2.15"');
    expect(markup).toContain("XBINNING 2 read as both binnings");
  });

  it("shows which tier and catalogue solved it, matched/predicted, RMS, SIP order and -- for a missing orientation", async () => {
    show();
    await flush();
    (solveActions(show())!.onSolve as () => void)();
    await flush();
    const markup = markupOf(show());
    expect(markup).toContain("Offline solver on this computer");
    expect(markup).toContain("Tier 0: Tycho-2 (bundled)");
    expect(markup).toContain("293 of 410 predicted");
    expect(markup).toContain("0.068 px");
    expect(markup).toContain("SIP order 3 (SIPMXERR 0.0041 px)");
    expect(markup).toMatch(/Orientation<\/div><div[^>]*>--<\/div>/);
    expect(markup).toContain("SIP order 4 was not tried: the field is under 5 degrees");
    expect(markup).not.toContain("Try astrometry.net");
  });

  it("shows the no-solution reason and offers astrometry.net once, saying it uploads the image", async () => {
    plateSolveMock.mockRejectedValueOnce(
      new TauriCommandError("plate_solve_cmd", NO_SOLUTION_TEXT, "local_no_solution", {
        best: { matched: 14, predicted: 120, rms_px: 6.27 },
        expectedTier0Stars: 20.4,
        centresTried: 397,
        canFallback: true,
      }),
    );
    show();
    await flush();
    expect(fallbackBlock(show())).toBeNull();
    (solveActions(show())!.onSolve as () => void)();
    await flush();
    const failed = show();
    const markup = markupOf(failed);
    expect(markup).toContain(NO_SOLUTION_TEXT);
    expect(markup).toContain("Best candidate: 14 of 120 predicted stars matched, RMS 6.27 px");
    expect(markup).toContain("add a deeper catalogue in Settings > Catalogs, such as a Siril Gaia DR3 extract");
    expect(markup).toContain("Uploads this image to nova.astrometry.net with your API key; the solve runs there.");
    expect(markup).toMatch(/<button[^>]*>[\s\S]*Try astrometry\.net<\/button>/);
    plateSolveMock.mockRejectedValueOnce(new Error("astrometry.net: job failed"));
    (fallbackBlock(failed)!.onFallback as () => void)();
    await flush();
    expect(plateSolveMock).toHaveBeenCalledTimes(2);
    expect(plateSolveMock.mock.calls[1][1]).toMatchObject({ solver: "astrometry.net" });
    const after = show();
    expect(fallbackBlock(after)).toBeNull();
    expect(markupOf(after)).toContain("astrometry.net: job failed");
  });

  it("offers no fallback for an error that is not a local no-solution", async () => {
    plateSolveMock.mockRejectedValueOnce(new Error("the image has 3 planes"));
    show();
    await flush();
    (solveActions(show())!.onSolve as () => void)();
    await flush();
    const tree = show();
    expect(fallbackBlock(tree)).toBeNull();
    expect(markupOf(tree)).toContain("the image has 3 planes");
  });
});

const TIE_CARDS: [string, string][] = [
  ["CTYPE1", "'RA---TAN-SIP'"],
  ["CRVAL1", "274.7003"],
];

const APPLIED_TIE = {
  decision: "applied",
  reason: null,
  wcs_cards: TIE_CARDS,
  history: ["Gaia DR3 tie: rshift, 61 stars, RMS 38.2 mas", "Gaia DR3, ESA/Gaia/DPAC, via VizieR"],
  model: "rshift",
  wcs_kind: "header",
  n_catalog: 412,
  n_detected: 200,
  n_matched: 64,
  n_used: 61,
  rms_before_mas: 512.4,
  rms_after_mas: 38.2,
  shift_east_mas: -412.5,
  shift_north_mas: 230.1,
  shift_err_east_mas: 4.9,
  shift_err_north_mas: 5.1,
  rotation_deg: -0.0021,
  rotation_err_deg: 0.0006,
  scale_ppm: null,
  scale_err_ppm: null,
  epoch_year: 2024.5,
  epoch_source: "header",
  catalog: "Gaia DR3 (I/355/gaiadr3) via VizieR, CDS",
  catalog_source: "network",
  attribution: "Gaia DR3, ESA/Gaia/DPAC, via VizieR",
  citation: "Gaia DR3 (Gaia Collaboration, Vallenari et al. 2023, A&A 674, A1), ESA/Gaia/DPAC, CC BY-NC 3.0 IGO, via VizieR (CDS)",
  warnings: ["2 saturated stars dropped"],
};

const REFUSED_TIE = {
  ...APPLIED_TIE,
  decision: "refused",
  reason: "9 < 15 used pairs for rshift",
  wcs_cards: [],
  history: [],
  n_matched: 9,
  n_used: 9,
  rms_after_mas: null,
  shift_east_mas: null,
  shift_north_mas: null,
  rotation_deg: null,
  warnings: [],
};

const WRITTEN = {
  fits_path: "C:/out/m16_wcs.fits",
  png_path: "C:/out/m16_wcs.png",
  dimensions: [2000, 1500],
  center_ra: 274.7,
  center_dec: -13.82,
  pixel_scale_arcsec: 2.4,
  sip_present: true,
  elapsed_ms: 12,
};

describe("PlateSolvePanel Gaia DR3 refinement (C21)", () => {
  const panel = (PlateSolvePanel as unknown as { type: Component }).type;
  const show = () => renderTree(panel, { filePath: "C:/data/m16.fits", solvePath: "C:/data/m16.fits" });
  const solveActions = (tree: ReactNode) => findElement(tree, (props) => typeof props.onSolve === "function");
  const refineBlock = (tree: ReactNode) => findElement(tree, (props) => typeof props.onRefine === "function");
  const writeButton = (tree: ReactNode) => findElement(tree, (props) => props["data-write-wcs"] === true);

  beforeEach(() => {
    hooks.run.active = true;
    hooks.run.cells = [];
    wcsInfoMock.mockReset().mockRejectedValue(new Error("no celestial WCS"));
    solveHintsMock.mockReset().mockResolvedValue(OBJCT_HINTS);
    plateSolveMock.mockReset().mockResolvedValue(LOCAL_RESULT);
    refineMock.mockReset().mockResolvedValue(APPLIED_TIE);
    writeWcsMock.mockReset().mockResolvedValue(WRITTEN);
    apiKeyMock.mockReset().mockResolvedValue({ key: "test-key" });
    Object.assign(refineProgress, { active: false, percent: 0, stage: "", current: 0, total: 0 });
    refineProgress.reset.mockClear();
  });

  afterEach(() => {
    hooks.run.active = false;
  });

  const solveOnce = async () => {
    show();
    await flush();
    (solveActions(show())!.onSolve as () => void)();
    await flush();
    return show();
  };

  const refineOnce = async () => {
    await solveOnce();
    (refineBlock(show())!.onRefine as () => void)();
    await flush();
    return show();
  };

  it("offers no refinement before there is a WCS to refine, and says why", async () => {
    show();
    await flush();
    const block = refineBlock(show());
    expect(block, "refine block").not.toBeNull();
    expect(block!.blocker).toBe(NO_WCS_TO_REFINE);
  });

  it("offers the refinement after a successful solve, with the consent text in front of the button", async () => {
    const tree = await solveOnce();
    expect(refineBlock(tree)!.blocker).toBeNull();
    const markup = markupOf(tree);
    expect(markup).toContain(GAIA_REFINE_CONSENT);
    expect(markup.indexOf(GAIA_REFINE_CONSENT)).toBeLessThan(markup.indexOf("Refine with Gaia DR3</button>"));
    expect(markup).toMatch(/<button[^>]*data-gaia-refine="true"[^>]*>(?:(?!<\/button>)[\s\S])*Refine with Gaia DR3<\/button>/);
  });

  it("offers the refinement on a frame whose header already carries a WCS", async () => {
    wcsInfoMock.mockResolvedValue(HEADER_WCS);
    show();
    await flush();
    expect(refineBlock(show())!.blocker).toBeNull();
    (refineBlock(show())!.onRefine as () => void)();
    await flush();
    expect(refineMock).toHaveBeenCalledWith("C:/data/m16.fits", { wcsCards: null, epochYear: null });
  });

  it("refines the solve's own cards, not the header, with the header epoch unless one is typed", async () => {
    await refineOnce();
    expect(refineMock).toHaveBeenCalledTimes(1);
    expect(refineMock).toHaveBeenCalledWith("C:/data/m16.fits", { wcsCards: LOCAL_RESULT.wcs_cards, epochYear: null });
    (refineBlock(show())!.onEpochChange as (text: string) => void)("1990,5");
    show();
    (refineBlock(show())!.onRefine as () => void)();
    await flush();
    expect(refineMock.mock.calls[1][1]).toEqual({ wcsCards: LOCAL_RESULT.wcs_cards, epochYear: 1990.5 });
  });

  it("shows matched, RMS in mas, shift and rotation and the Gaia DR3 attribution", async () => {
    const markup = markupOf(await refineOnce());
    expect(markup).toContain("Refined with Gaia DR3");
    expect(markup).toContain("61 used of 64 matched (412 Gaia DR3 stars)");
    expect(markup).toContain("512.4 mas before, 38.2 mas after");
    expect(markup).toContain("-412.5 mas E, +230.1 mas N");
    expect(markup).toContain("-0.00210°");
    expect(markup).toContain("2 saturated stars dropped");
    expect(markup).toMatch(/<div[^>]*data-gaia-attribution="true"[^>]*>Gaia DR3, ESA\/Gaia\/DPAC, via VizieR<\/div>/);
  });

  it("writes the refined WCS through Write WCS with the tie's HISTORY lines", async () => {
    const onWcsWritten = vi.fn();
    const props = { filePath: "C:/data/m16.fits", solvePath: "C:/data/m16.fits", onWcsWritten };
    const view = () => renderTree(panel, props);
    view();
    await flush();
    (solveActions(view())!.onSolve as () => void)();
    await flush();
    (refineBlock(view())!.onRefine as () => void)();
    await flush();
    const button = writeButton(view());
    expect(button!.disabled).toBe(false);
    (button!.onClick as () => void)();
    await flush();
    expect(writeWcsMock).toHaveBeenCalledWith("C:/data/m16.fits", TIE_CARDS, undefined, APPLIED_TIE.history, null);
    expect(onWcsWritten).toHaveBeenCalledWith(WRITTEN);
  });

  it("keeps Write WCS on the solve's cards after a refusal, which names its numbers", async () => {
    refineMock.mockResolvedValue(REFUSED_TIE);
    const markup = markupOf(await refineOnce());
    expect(markup).toContain("Not refined");
    expect(markup).toContain("9 &lt; 15 used pairs for rshift");
    (writeButton(show())!.onClick as () => void)();
    await flush();
    expect(writeWcsMock).toHaveBeenCalledWith("C:/data/m16.fits", LOCAL_RESULT.wcs_cards, undefined, null, null);
  });

  it("lets a header-WCS frame write its refined WCS without a solve", async () => {
    wcsInfoMock.mockResolvedValue(HEADER_WCS);
    show();
    await flush();
    expect(writeButton(show())!.disabled).toBe(true);
    (refineBlock(show())!.onRefine as () => void)();
    await flush();
    expect(writeButton(show())!.disabled).toBe(false);
  });

  it("shows a failed query as an error and keeps the solve", async () => {
    refineMock.mockRejectedValue(new Error(DEAD_VIZIER));
    const markup = markupOf(await refineOnce());
    expect(markup).toContain(DEAD_VIZIER);
    expect(markup).toContain("293 of 410 predicted");
  });

  it("drops the refinement when a new solve starts", async () => {
    await refineOnce();
    expect(markupOf(show())).toContain("Refined with Gaia DR3");
    (solveActions(show())!.onSolve as () => void)();
    await flush();
    expect(markupOf(show())).not.toContain("Refined with Gaia DR3");
  });

  it("refines the written copy's own tied header, not the solve's untied cards", async () => {
    const onWcsWritten = vi.fn();
    const at = (solvePath: string) => () => renderTree(panel, { filePath: "C:/data/m16.fits", solvePath, onWcsWritten });
    const source = at("C:/data/m16.fits");
    source();
    await flush();
    (solveActions(source())!.onSolve as () => void)();
    await flush();
    (refineBlock(source())!.onRefine as () => void)();
    await flush();
    (writeButton(source())!.onClick as () => void)();
    await flush();
    wcsInfoMock.mockResolvedValue(HEADER_WCS);
    refineMock.mockResolvedValue({ ...APPLIED_TIE, decision: "no_significant_correction", wcs_cards: [], history: [] });
    const copy = at(WRITTEN.fits_path);
    copy();
    await flush();
    (refineBlock(copy())!.onRefine as () => void)();
    await flush();
    expect(refineMock).toHaveBeenLastCalledWith(WRITTEN.fits_path, { wcsCards: null, epochYear: null });
    expect(writeButton(copy())!.disabled).toBe(true);
  });

  it("refines a JWST gWCS instead of the local solve, so its distortion model is kept", async () => {
    wcsInfoMock.mockResolvedValue({ ...HEADER_WCS, wcs_kind: "gwcs" });
    await refineOnce();
    expect(refineMock).toHaveBeenCalledWith("C:/data/m16.fits", { wcsCards: null, epochYear: null });
  });

  it("names the WCS the tie refined in the result block", async () => {
    refineMock.mockResolvedValue({ ...APPLIED_TIE, wcs_kind: "solve" });
    const markup = markupOf(await refineOnce());
    expect(markup).toMatch(/Refined WCS[\s\S]*Plate solve/);
  });

  it("keeps Write WCS off while a refinement runs, and leaves it usable once the refinement is applied", async () => {
    let resolveRefine: (value: unknown) => void = () => {};
    refineMock.mockImplementation(() => new Promise((resolve) => (resolveRefine = resolve)));
    await solveOnce();
    (refineBlock(show())!.onRefine as () => void)();
    await flush();
    const during = writeButton(show());
    expect(during!.disabled).toBe(true);
    (during!.onClick as () => void)();
    await flush();
    expect(writeWcsMock).not.toHaveBeenCalled();
    resolveRefine(APPLIED_TIE);
    await flush();
    const after = writeButton(show());
    expect(after!.disabled).toBe(false);
    (after!.onClick as () => void)();
    await flush();
    expect(writeWcsMock).toHaveBeenCalledWith("C:/data/m16.fits", TIE_CARDS, undefined, APPLIED_TIE.history, null);
    expect(markupOf(show())).not.toContain("Writing WCS...");
  });

  it("cancels a running refinement, keeps the WCS as it was and ignores the late answer", async () => {
    let resolveRefine: (value: unknown) => void = () => {};
    refineMock.mockImplementation(() => new Promise((resolve) => (resolveRefine = resolve)));
    cancelRefineMock.mockClear();
    await solveOnce();
    (refineBlock(show())!.onRefine as () => void)();
    await flush();
    expect(refineBlock(show())!.loading).toBe(true);
    (refineBlock(show())!.onCancel as () => void)();
    await flush();
    const block = refineBlock(show());
    expect(cancelRefineMock).toHaveBeenCalledTimes(1);
    expect(block!.loading).toBe(false);
    expect(block!.note).toBe(GAIA_REFINE_CANCELLED_NOTE);
    resolveRefine(APPLIED_TIE);
    await flush();
    expect(markupOf(show())).not.toContain("Refined with Gaia DR3");
    (writeButton(show())!.onClick as () => void)();
    await flush();
    expect(writeWcsMock).toHaveBeenCalledWith("C:/data/m16.fits", LOCAL_RESULT.wcs_cards, undefined, null, null);
  });

  it("shows the stage the refine reports while it runs and resets the progress once per refine", async () => {
    let resolveRefine: (value: unknown) => void = () => {};
    refineMock.mockImplementation(() => new Promise((resolve) => (resolveRefine = resolve)));
    await solveOnce();
    expect(refineProgress.reset).not.toHaveBeenCalled();
    (refineBlock(show())!.onRefine as () => void)();
    await flush();
    expect(refineProgress.reset).toHaveBeenCalledTimes(1);
    Object.assign(refineProgress, { active: true, percent: 40.6, stage: "measure", current: 812, total: 2000 });
    expect(refineBlock(show())!.progressText).toBe("Measuring stars: 812 of 2000");
    Object.assign(refineProgress, { percent: 27.3, stage: "vizier", current: 41, total: 150 });
    expect(refineBlock(show())!.progressText).toBe("Waiting for VizieR: 0:41 / 2:30");
    resolveRefine(APPLIED_TIE);
    await flush();
    expect(refineBlock(show())!.progressText).toBeNull();
    (refineBlock(show())!.onRefine as () => void)();
    await flush();
    expect(refineProgress.reset).toHaveBeenCalledTimes(2);
  });

  it("cancels a running refinement when another file is selected", async () => {
    refineMock.mockImplementation(() => new Promise(() => {}));
    cancelRefineMock.mockClear();
    await solveOnce();
    (refineBlock(show())!.onRefine as () => void)();
    await flush();
    expect(refineBlock(show())!.loading).toBe(true);
    renderTree(panel, { filePath: "C:/data/other.fits", solvePath: "C:/data/other.fits" });
    await flush();
    expect(cancelRefineMock).toHaveBeenCalledTimes(1);
  });

  it("shows the credit the solve result carries, as the backend words it, and passes the tier to Write WCS", async () => {
    plateSolveMock.mockResolvedValue({ ...LOCAL_RESULT, catalog: "tier1-gaia-siril", credit: REVIEWED_SIRIL_CREDIT });
    const tree = await solveOnce();
    expect(markupOf(tree)).toMatch(
      /<div[^>]*data-gaia-attribution="true"[^>]*data-solve-credit="true"[^>]*title="Gaia DR3 \(Gaia Collaboration, Vallenari et al\. 2023, A&A 674, A1\), ESA\/Gaia\/DPAC, CC BY-NC 3\.0 IGO, via the Siril HEALPix extract \(Zenodo 14692304\)"[^>]*>Gaia DR3, ESA\/Gaia\/DPAC, CC BY-NC 3\.0 IGO, via the Siril HEALPix extract \(Zenodo 14692304\)<\/div>/,
    );
    (writeButton(show())!.onClick as () => void)();
    await flush();
    expect(writeWcsMock).toHaveBeenCalledWith("C:/data/m16.fits", LOCAL_RESULT.wcs_cards, undefined, null, "tier1-gaia-siril");
  });

  it("credits nothing and passes no tier to Write WCS when the solve result carries no credit", async () => {
    for (const catalog of ["tier1-gaia-siril", "tier0-tycho2"]) {
      hooks.run.cells = [];
      writeWcsMock.mockClear();
      plateSolveMock.mockResolvedValue({ ...LOCAL_RESULT, catalog, credit: null });
      expect(markupOf(await solveOnce()), catalog).not.toContain("data-solve-credit");
      (writeButton(show())!.onClick as () => void)();
      await flush();
      expect(writeWcsMock, catalog).toHaveBeenCalledWith("C:/data/m16.fits", LOCAL_RESULT.wcs_cards, undefined, null, null);
    }
  });

  it("refuses a typed epoch outside 1900 to 2100 before anything goes online", async () => {
    await solveOnce();
    (refineBlock(show())!.onEpochChange as (text: string) => void)("1850");
    const block = refineBlock(show());
    expect(block!.epochError).toMatch(/^Epoch: type a decimal year between 1900 and 2100/);
    (block!.onRefine as () => void)();
    await flush();
    expect(refineMock).not.toHaveBeenCalled();
  });
});

describe("GaiaRefineBlock", () => {
  const base = {
    blocker: null,
    loading: false,
    epochText: "",
    epochError: null,
    epochPlaceholder: "2024.500 (header)",
    onEpochChange: () => {},
    onRefine: () => {},
    result: null,
    error: null,
  };

  it("names the blocker on the disabled button", () => {
    const markup = renderToStaticMarkup(createElement(GaiaRefineBlock, { ...base, blocker: NO_WCS_TO_REFINE }));
    const button = markup.match(/<button[^>]*data-gaia-refine="true"[^>]*>/)?.[0] ?? "";
    expect(button).toContain('disabled=""');
    expect(unescapeHtml(button)).toContain(`title="${NO_WCS_TO_REFINE}"`);
  });

  it("puts the full citation and licence on the attribution line", () => {
    const markup = unescapeHtml(renderToStaticMarkup(createElement(GaiaRefineBlock, { ...base, result: APPLIED_TIE as never })));
    expect(markup).toContain(`title="${APPLIED_TIE.citation}"`);
  });

  it("shows the header epoch as the placeholder of the epoch field", () => {
    const markup = renderToStaticMarkup(createElement(GaiaRefineBlock, base));
    expect(markup).toContain('placeholder="2024.500 (header)"');
  });

  it("shows the refine stage and a Cancel button while it runs, and neither when idle", () => {
    const running = renderToStaticMarkup(
      createElement(GaiaRefineBlock, { ...base, loading: true, progressText: "Measuring stars: 812 of 2000", onCancel: () => {} }),
    );
    const cancel = buttonEndingWith(running, "Cancel");
    expect(cancel).toContain('aria-label="Cancel the Gaia DR3 refinement"');
    expect(cancel).not.toMatch(/^<button[^>]*\sdisabled=""/);
    expect(running).toMatch(/<div[^>]*role="status"[^>]*data-gaia-refine-progress="true"[^>]*>Measuring stars: 812 of 2000<\/div>/);
    const idle = renderToStaticMarkup(createElement(GaiaRefineBlock, { ...base, progressText: "Measuring stars: 812 of 2000", onCancel: () => {} }));
    expect(idle).not.toContain("Cancel</button>");
    expect(idle).not.toContain("data-gaia-refine-progress");
  });

  it("gives the epoch field room for its longest placeholder", () => {
    const markup = renderToStaticMarkup(createElement(GaiaRefineBlock, base));
    const wrapper = markup.match(/<div class="([^"]*)"><label[^>]*>Epoch \(year\)<\/label>/);
    expect(wrapper, "epoch field wrapper").not.toBeNull();
    expect(wrapper![1].split(" ")).toContain("w-40");
    expect(wrapper![1].split(" ")).not.toContain("w-32");
  });
});
