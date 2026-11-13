import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { DqFlagTableResult } from "../../../services/display";
import type { PlaneInfo } from "../../../shared/types";

type FileRow = { id: string; path: string; result: { plane: Partial<PlaneInfo> } | null };

const dq = vi.hoisted(() => ({ state: null as Record<string, unknown> | null, file: null as FileRow | null }));
const mocks = vi.hoisted(() => ({ exclude: vi.fn(), apply: vi.fn() }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../context/PreviewContext", () => ({
  useDqContext: () => {
    if (!dq.state) throw new Error("DqFlagMenu takes its state as props");
    return dq.state;
  },
  useFileContext: () => ({ file: dq.file }),
}));
vi.mock("../../../services/companions", () => ({ setFlagExclusion: mocks.exclude }));
vi.mock("../../../hooks/applyLayerChange", () => ({ applyLayerChange: mocks.apply }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "a.fits", image: "original" }) }));

import DqControls, { DqFlagMenu } from "../DqControls";
import { hooks } from "../../../test/hookRuntime";
import { measurementLog } from "../../../utils/measurementLog";

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
    expect(markup).toContain('title="Flag layer without bit names: raw bits are listed and nothing is excluded until bits are chosen"');
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

  it("lists the names an afw mask gives and labels the exclusion with them", () => {
    const markup = menu(
      {
        ...UNKNOWN,
        label: "MP_ cards (afw mask planes)",
        flags: [
          { bit: 1, name: "BAD" },
          { bit: 2, name: "SAT" },
          { bit: 32, name: "DETECTED" },
        ],
        bits_present: null,
        exclusion_mask: 3,
        default_mask: 3,
        dq_origin: "EXTNAME MASK, HDU 2",
      },
      3,
    );
    expect(flagRows(markup)).toEqual([
      { name: "BAD", checked: true },
      { name: "SAT", checked: true },
      { name: "DETECTED", checked: false },
    ]);
    expect(markup).toContain("exclude BAD, SAT from stats");
    expect(markup).toContain("Mask: EXTNAME MASK, HDU 2");
    expect(markup).not.toContain("no bits set");
  });

  it("marks the flags the statistics leave out, so the drawn ticks of a Rubin mask agree with the count and the sentence", () => {
    const rubin: DqFlagTableResult = {
      ...UNKNOWN,
      label: "MP_ cards (afw mask planes)",
      flags: [
        { bit: 1, name: "BAD" },
        { bit: 2, name: "SAT" },
        { bit: 4, name: "INTRP" },
        { bit: 8, name: "CR" },
        { bit: 16, name: "EDGE" },
        { bit: 32, name: "DETECTED" },
        { bit: 128, name: "SUSPECT" },
        { bit: 256, name: "NO_DATA" },
      ],
      bits_present: null,
      default_mask: 415,
      exclusion_mask: 267,
      exclusion_edited: false,
      dq_origin: "EXTNAME MASK, HDU 2",
    };
    const markup = renderToStaticMarkup(
      createElement(DqFlagMenu, {
        flagTable: rubin,
        overlay: { enabled: false, mask: 415 },
        setOverlay: () => {},
        excludeDq: false,
        setExcludeDq: () => {},
        dqMaskError: null,
        exclusion: { busy: false, error: null, onApply: () => {}, onDefault: () => {} },
      }),
    );
    const rows = [...markup.matchAll(/<label[^>]*>(.*?)<\/label>/g)]
      .map((m) => m[1])
      .filter((row) => /<span[^>]*>\d+<\/span>/.test(row))
      .map((row) => ({ name: /<span[^>]*>([^<]*)<\/span>/.exec(row)?.[1], ticked: /\schecked=""/.test(row), excluded: />excl<\/span>/.test(row) }));
    expect(rows.filter((row) => row.ticked).map((row) => row.name)).toEqual(["BAD", "SAT", "INTRP", "CR", "EDGE", "SUSPECT", "NO_DATA"]);
    expect(rows.filter((row) => row.excluded).map((row) => row.name)).toEqual(["BAD", "SAT", "CR", "NO_DATA"]);
    expect(markup).toContain("exclude 4 flags from stats");
    expect(markup).toContain("Excluded now: BAD, SAT, CR, NO_DATA (default)");
    expect(markup).toContain("Ticked: drawn on the image. excl: left out of histogram, statistics and photometry while the box below is ticked.");
  });

  it("says that the bits could not be read instead of claiming that none is set", () => {
    const markup = menu({ ...UNKNOWN, bits_present: null }, 0);
    expect(markup).toContain("flag bits could not be read from this plane");
    expect(markup).not.toContain("no bits set");
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
  dq.file = { id: "f1", path: "C:/d/hsc.fits", result: { plane: {} } };
  vi.stubGlobal("window", { addEventListener: vi.fn(), removeEventListener: vi.fn() });
  vi.spyOn(measurementLog, "append").mockImplementation((draft) => ({ ...draft, id: "id1", timestamp_utc: "2026-11-12T10:00:00.000Z" }));
});

afterEach(() => {
  hooks.mount();
  dq.state = null;
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
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

type Exclusion = { busy: boolean; error: string | null; onApply: (mask: number) => void; onDefault: () => void };

const REASON = "mask layer HDU 2 is a floating-point plane (BITPIX -32): flag bits cannot be read exactly";
const HSC_DQ = "C:/d/hsc.fits#hdu=2";
const PLANE: PlaneInfo = {
  kind: "hdu",
  index: 1,
  key: null,
  extname: "IMAGE",
  extver: null,
  is_dq: false,
  is_err: false,
  dq_ref: HSC_DQ,
  err_ref: null,
  source_path: "C:/d/hsc.fits",
  dq_table: "unknown",
};

const exclusionOf = (tree: ReactNode) => flagMenu(tree)?.props.exclusion as Exclusion | null | undefined;
const applyButton = (markup: string) => markup.match(/<button[^>]*>Exclude the checked flags<\/button>/)?.[0];

function opened(patch: Record<string, unknown>): ReactNode {
  (chevron(render(patch))!.props.onClick as () => void)();
  return render(patch);
}

describe("DqControls with a mask layer that cannot be used", () => {
  it("opens the reason instead of a dead button when the mask layer cannot be used", () => {
    dq.file!.result!.plane.dq_unavailable = REASON;
    const tree = render();
    expect(toggle(tree).props.disabled).toBe(true);
    expect(toggle(tree).props.title).toBe(`Mask layer not used: ${REASON}`);
    expect(chevron(tree)?.props.disabled).toBe(false);
    expect(chevron(tree)?.props.title).toBe(`Mask layer not used: ${REASON}`);
    (chevron(tree)!.props.onClick as () => void)();
    const open = render();
    expect(flagMenu(open)?.props.unavailable).toBe(REASON);
    const markup = html(flagMenu(open));
    expect(markup).toContain("Mask layer not used:");
    expect(markup).toContain("Nothing is excluded from histogram, statistics and photometry.");
    expect(markup).not.toContain("<input");

    hooks.mount();
    const product = render({ dqTarget: PRODUCT, flagTable: { ...JWST, dq_ref: null } });
    expect(toggle(product).props.title).toBe("No DQ plane in this product (Moment 0)");
    expect(chevron(product)?.props.disabled).toBe(true);
    expect(chevron(product)?.props.title).toBe("No DQ plane in this product (Moment 0)");
  });
});

describe("DqControls exclusion editor", () => {
  it("sends the checked flags as the exclusion and puts the returned plane into the file row", async () => {
    mocks.exclude.mockResolvedValue(PLANE);
    const patch = { dqRef: HSC_DQ, flagTable: { ...UNKNOWN, exclusion_mask: 0, exclusion_edited: false }, overlay: { enabled: true, mask: 5 } };
    const tree = opened(patch);
    const exclusion = exclusionOf(tree)!;
    expect(exclusion.busy).toBe(false);
    expect(applyButton(html(flagMenu(tree)))).toBeDefined();
    expect(applyButton(html(flagMenu(tree)))).not.toContain('disabled=""');
    exclusion.onApply(5);
    expect(exclusionOf(render(patch))?.busy).toBe(true);
    await Promise.resolve();
    expect(mocks.exclude).toHaveBeenCalledWith("C:/d/hsc.fits", 5);
    expect(mocks.apply).toHaveBeenCalledWith(dq.file, PLANE);
    const idle = render({ ...patch, overlay: { enabled: true, mask: 0 } });
    expect(exclusionOf(idle)?.busy).toBe(false);
    expect(applyButton(html(flagMenu(idle)))).toContain('disabled=""');
  });

  it("offers the default again after a choice", async () => {
    mocks.exclude.mockResolvedValue(PLANE);
    const table = { ...UNKNOWN, exclusion_mask: 5, exclusion_default: 0, exclusion_edited: true };
    const tree = opened({ dqRef: HSC_DQ, flagTable: table });
    const markup = html(flagMenu(tree));
    expect(markup).toContain("Default exclusion");
    expect(markup).toContain("Excluded now: BIT 0, BIT 2 (chosen)");
    exclusionOf(tree)!.onDefault();
    expect(mocks.exclude).toHaveBeenCalledWith("C:/d/hsc.fits", null);
    await Promise.resolve();
    expect(html(flagMenu(render({ dqRef: HSC_DQ, flagTable: { ...table, exclusion_edited: false } })))).not.toContain("Default exclusion");
  });

  it("keeps the exclusion buttons away from a product and reports a refusal", async () => {
    const product = opened({ dqTarget: PRODUCT, dqRef: "C:/out/moment0.fits#hdu=2", flagTable: { ...JWST, dq_ref: "C:/out/moment0.fits#hdu=2" } });
    expect(flagMenu(product)).toBeDefined();
    expect(exclusionOf(product) ?? null).toBeNull();

    hooks.mount();
    mocks.exclude.mockRejectedValue(new Error("this image has no mask layer"));
    const patch = { dqRef: HSC_DQ, flagTable: UNKNOWN, overlay: { enabled: true, mask: 5 } };
    exclusionOf(opened(patch))!.onApply(5);
    await Promise.resolve();
    const after = render(patch);
    expect(exclusionOf(after)?.error).toBe("this image has no mask layer");
    expect(html(flagMenu(after))).toMatch(/<div[^>]*role="alert"[^>]*>Exclusion not changed: this image has no mask layer<\/div>/);
    expect(mocks.apply).not.toHaveBeenCalled();
  });

  it("drops a result that arrives after another file was selected", async () => {
    mocks.exclude.mockResolvedValue(PLANE);
    const patch = { dqRef: HSC_DQ, flagTable: UNKNOWN, overlay: { enabled: true, mask: 5 } };
    exclusionOf(opened(patch))!.onApply(5);
    dq.file = { id: "f2", path: "C:/d/other.fits", result: { plane: {} } };
    render(patch);
    await Promise.resolve();
    expect(mocks.apply).not.toHaveBeenCalled();
    expect(exclusionOf(render(patch))?.busy).toBe(false);
  });
});

describe("DqControls overlay toggle", () => {
  it("exposes the overlay state to assistive technology", () => {
    expect(toggle(render()).props["aria-pressed"]).toBe(false);
    expect(toggle(render({ dqRef: JWST.dq_ref, flagTable: JWST, overlay: { enabled: true, mask: 7 } })).props["aria-pressed"]).toBe(true);
    expect(toggle(render({ dqTarget: PRODUCT, flagTable: { ...JWST, dq_ref: null }, overlay: { enabled: true, mask: 7 } })).props["aria-pressed"]).toBe(false);
  });
});

describe("DqControls measurement log", () => {
  const logged = () => vi.mocked(measurementLog.append).mock.calls.map(([draft]) => draft);

  it("logs a chosen exclusion", async () => {
    mocks.exclude.mockResolvedValue(PLANE);
    const patch = { dqRef: HSC_DQ, flagTable: { ...UNKNOWN, exclusion_mask: 0, exclusion_edited: false }, overlay: { enabled: true, mask: 5 } };
    exclusionOf(opened(patch))!.onApply(5);
    await Promise.resolve();
    expect(logged()).toHaveLength(1);
    expect(logged()[0].kind).toBe("layer_assignment");
    expect(logged()[0].file).toBe("a.fits");
    expect(logged()[0].params).toEqual({ role: "exclusion", action: "choose", exclusion_mask: 5 });

    vi.clearAllMocks();
    hooks.mount();
    mocks.exclude.mockResolvedValue(PLANE);
    const table = { ...UNKNOWN, exclusion_mask: 5, exclusion_default: 0, exclusion_edited: true };
    exclusionOf(opened({ dqRef: HSC_DQ, flagTable: table }))!.onDefault();
    await Promise.resolve();
    expect(logged().map((draft) => draft.params)).toEqual([{ role: "exclusion", action: "default", exclusion_mask: null }]);

    vi.clearAllMocks();
    hooks.mount();
    mocks.exclude.mockRejectedValue(new Error("this image has no mask layer"));
    exclusionOf(opened(patch))!.onApply(5);
    await Promise.resolve();
    expect(logged()).toEqual([]);
  });
});
