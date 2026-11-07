import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { DqFlagTableResult } from "../../../services/display";

const dq = vi.hoisted(() => ({ state: null as Record<string, unknown> | null }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../context/PreviewContext", () => ({
  useDqContext: () => {
    if (!dq.state) throw new Error("DqFlagMenu takes its state as props");
    return dq.state;
  },
}));

import DqControls, { DqFlagMenu } from "../DqControls";
import { hooks } from "../../../test/hookRuntime";

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

type Props = Record<string, unknown> & { children?: ReactNode };

const PRODUCT = { kind: "product", label: "Moment 0" };
const controls = (DqControls as unknown as { type: () => ReactNode }).type;
const actions = { retryFlagTable: vi.fn(), setOverlay: vi.fn(), setExcludeDq: vi.fn() };

function render(patch: Record<string, unknown> = {}): ReactNode {
  dq.state = {
    dqRef: null,
    dqTarget: { kind: "file" },
    flagTable: null,
    flagTableLoading: false,
    flagTableError: null,
    overlay: { enabled: false, mask: 0 },
    excludeDq: false,
    dqMask: null,
    dqMaskLoading: false,
    dqMaskError: null,
    ...actions,
    ...patch,
  };
  return hooks.render(controls, {}) as ReactNode;
}

function findAll(node: ReactNode, match: (node: ReactElement<Props>) => boolean): ReactElement<Props>[] {
  if (Array.isArray(node)) return node.flatMap((child) => findAll(child, match));
  if (!isValidElement<Props>(node)) return [];
  return [...(match(node) ? [node] : []), ...findAll(node.props.children, match)];
}

const buttons = (tree: ReactNode) => findAll(tree, (node) => node.type === "button");
const html = (node: ReactNode) => renderToStaticMarkup(node as ReactElement);
const toggle = (tree: ReactNode) => buttons(tree).find((button) => html(button).includes("DQ</button>"))!;
const named = (tree: ReactNode, text: string) => buttons(tree).find((button) => html(button).includes(text));
const chevron = (tree: ReactNode) => buttons(tree).find((button) => button.props["aria-label"] === "DQ flag options");
const flagMenu = (tree: ReactNode) => findAll(tree, (node) => node.type === DqFlagMenu)[0];

beforeEach(() => {
  hooks.mount();
  vi.clearAllMocks();
  vi.stubGlobal("window", { addEventListener: vi.fn(), removeEventListener: vi.fn() });
});

afterEach(() => {
  hooks.mount();
  dq.state = null;
  vi.unstubAllGlobals();
});

describe("DqControls title follows the displayed target", () => {
  it("says the DQ of a product is being read instead of claiming the file has none", () => {
    const tree = render({ dqTarget: PRODUCT, flagTableLoading: true });
    expect(toggle(tree).props.title).toBe("Reading DQ of Moment 0…");
    expect(toggle(tree).props.disabled).toBe(true);
    expect(html(tree)).not.toContain("No DQ plane in this file");
  });

  it("names the product when its FITS has no DQ plane", () => {
    const tree = render({ dqTarget: PRODUCT, flagTable: { ...JWST, dq_ref: null } });
    expect(toggle(tree).props.title).toBe("No DQ plane in this product (Moment 0)");
  });

  it("has its own wording for the colour composite", () => {
    expect(toggle(render({ dqTarget: { kind: "composite" } })).props.title).toBe("No DQ overlay for the colour composite");
  });

  it("keeps the file wording for a file without a DQ plane", () => {
    expect(toggle(render()).props.title).toBe("No DQ plane in this file");
  });
});

describe("DqControls with a failed flag-table read", () => {
  it("shows the error and a Retry that reads the table again, not 'No DQ plane in this file'", () => {
    const tree = render({ dqTarget: PRODUCT, flagTableError: "output is locked" });
    const markup = html(tree);
    expect(toggle(tree).props.title).toBe("DQ table unavailable: output is locked");
    expect(markup).not.toContain("No DQ plane");
    const retry = named(tree, "Retry");
    expect(retry?.props.title).toBe("DQ table unavailable: output is locked");
    (retry?.props.onClick as () => void)();
    expect(actions.retryFlagTable).toHaveBeenCalledTimes(1);
  });

  it("closes the flag menu of the source file instead of spinning when its table fails", () => {
    (chevron(render({ dqRef: JWST.dq_ref }))!.props.onClick as () => void)();
    const loading = render({ dqRef: JWST.dq_ref });
    expect(flagMenu(loading)).toBeDefined();
    expect(html(loading)).toContain("animate-spin");
    const failed = render({ dqRef: JWST.dq_ref, flagTableError: "read error" });
    expect(flagMenu(failed)).toBeUndefined();
    expect(html(failed)).not.toContain("animate-spin");
    expect(named(failed, "Retry")).toBeDefined();
  });
});

describe("DqControls states", () => {
  it("offers the overlay for a target with DQ and toggles it", () => {
    const tree = render({ dqRef: JWST.dq_ref, flagTable: JWST });
    expect(toggle(tree).props.disabled).toBe(false);
    expect(toggle(tree).props.title).toBe("Show flagged DQ pixels over the image");
    (toggle(tree).props.onClick as () => void)();
    expect(actions.setOverlay).toHaveBeenCalledWith({ enabled: true });
    expect(toggle(render({ dqRef: JWST.dq_ref, flagTable: JWST, overlay: { enabled: true, mask: 7 } })).props.title).toBe("DQ overlay shown — click to hide");
  });

  it("reports a failed mask and shows a spinner while the mask loads", () => {
    expect(toggle(render({ dqRef: JWST.dq_ref, flagTable: JWST, dqMaskError: "mask failed" })).props.title).toBe("DQ overlay failed: mask failed");
    expect(html(toggle(render({ dqRef: JWST.dq_ref, flagTable: JWST, dqMaskLoading: true })))).toContain("animate-spin");
  });

  it("does not show the overlay as on while the displayed target has no DQ plane", () => {
    const off = toggle(render({ dqTarget: PRODUCT, flagTable: { ...JWST, dq_ref: null }, overlay: { enabled: true, mask: 7 } }));
    const on = toggle(render({ dqRef: JWST.dq_ref, flagTable: JWST, overlay: { enabled: true, mask: 7 } }));
    expect((off.props.style as { color: string }).color).not.toBe((on.props.style as { color: string }).color);
  });

  it("opens the flag menu from a named options button and closes it when the DQ plane goes away", () => {
    const closed = render({ dqRef: JWST.dq_ref, flagTable: JWST });
    const options = chevron(closed);
    expect(options).toBeDefined();
    expect(String(options?.props.className).split(" ")).toContain("p-1");
    expect(options?.props["aria-expanded"]).toBe(false);
    (options!.props.onClick as () => void)();
    const open = render({ dqRef: JWST.dq_ref, flagTable: JWST });
    expect(chevron(open)?.props["aria-expanded"]).toBe(true);
    expect(flagMenu(open)?.props.flagTable).toBe(JWST);
    render({ dqTarget: PRODUCT, flagTable: { ...JWST, dq_ref: null } });
    expect(flagMenu(render({ dqRef: JWST.dq_ref, flagTable: JWST }))).toBeUndefined();
  });
});
