import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { FitsExtension } from "../../../services/header";
import type { PlaneInfo } from "../../../shared/types/fits.types";

const mocks = vi.hoisted(() => ({ extensions: vi.fn(), header: vi.fn() }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/header", () => ({ getFitsExtensions: mocks.extensions, getHeaderByHdu: mocks.header }));
vi.mock("../CompanionAssign", () => ({
  default: function CompanionAssign() {
    return null;
  },
}));

import HduSelectorPanel from "../HduSelectorPanel";
import CompanionAssign from "../CompanionAssign";
import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";

const FILE = "C:/d/a.fits";
const SHOWN = `${FILE}#hdu=1`;
const BASE: PlaneInfo = {
  kind: "hdu",
  index: 1,
  key: null,
  extname: "IMAGE",
  extver: null,
  is_dq: false,
  is_err: false,
  dq_ref: null,
  err_ref: null,
  source_path: FILE,
  dq_table: null,
};
const IN_USE: PlaneInfo = {
  ...BASE,
  err_ref: `${FILE}#hdu=3`,
  dq_ref: `${FILE}#hdu=2`,
  err_origin: { role: "variance", source: "EXTNAME VARIANCE, HDU 3: square root", assumed: false, n_no_data: 12, origin: "same_file" },
  dq_origin: "EXTNAME MASK, HDU 2",
  dq_origin_kind: "same_file",
};

function hdu(index: number, extname: string | null, patch: Partial<FitsExtension> = {}): FitsExtension {
  return {
    index,
    extname,
    naxis: 2,
    naxis1: 4,
    naxis2: 4,
    naxis3: 0,
    bitpix: -32,
    has_data: true,
    extver: null,
    ref: `${FILE}#hdu=${index}`,
    kind: "hdu",
    is_dq: false,
    is_err: false,
    role: "other",
    role_basis: `EXTNAME ${extname}`,
    role_assumed: false,
    bunit: null,
    ...patch,
  };
}

const AFW: FitsExtension[] = [
  hdu(0, null, { naxis: 0, naxis1: 0, naxis2: 0, has_data: false, role_basis: "no image data" }),
  hdu(1, "IMAGE", { role: "science", bunit: "nJy" }),
  hdu(2, "MASK", { role: "mask", is_dq: true, bitpix: 32 }),
  hdu(3, "VARIANCE", { role: "variance", bunit: "nJy^2" }),
  hdu(4, "WHT", { role: "other" }),
];

type Props = Record<string, unknown> & { children?: ReactNode };
type PanelProps = Parameters<typeof HduSelectorPanel>[0];

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const render = (props: Partial<PanelProps> = {}) =>
  hooks.render(HduSelectorPanel, { filePath: FILE, activePath: SHOWN, onActivate: vi.fn(), ...props }) as ReactNode;
async function loaded(props: Partial<PanelProps> = {}): Promise<ReactNode> {
  render(props);
  await settle();
  return render(props);
}
const elements = (tree: ReactNode) => elementsOf<Props>(tree);
const rowOf = (tree: ReactNode, index: number) => elements(tree).find((e) => e.key === `${FILE}#hdu=${index}`)!;
const markup = (node: ReactNode) => renderToStaticMarkup(node as ReactElement);
const block = (tree: ReactNode) => elements(tree).find((e) => e.type === CompanionAssign);
const alertText = (tree: ReactNode) => elements(tree).filter((e) => e.type === "p" && e.props.role === "alert").map(textOf);

beforeEach(() => {
  hooks.mount();
  vi.clearAllMocks();
  vi.spyOn(console, "error").mockImplementation(() => {});
  mocks.extensions.mockResolvedValue({ extensions: AFW });
});

afterEach(() => {
  hooks.mount();
  vi.restoreAllMocks();
});

describe("HduSelectorPanel layer roles", () => {
  it("tags each HDU with its role and unit", async () => {
    const tree = await loaded({ activePlane: IN_USE });
    const image = markup(rowOf(tree, 1));
    const mask = markup(rowOf(tree, 2));
    const variance = markup(rowOf(tree, 3));
    const wht = markup(rowOf(tree, 4));
    expect(variance).toContain('title="variance layer, from EXTNAME VARIANCE">VAR</span>');
    expect(variance).toContain('title="BUNIT nJy^2">nJy^2</span>');
    expect(variance).toContain('title="Error layer of the shown image">error layer</span>');
    expect(mask).toContain('title="mask layer, from EXTNAME MASK">MASK</span>');
    expect(mask).toContain('title="Mask of the shown image">mask</span>');
    expect(image).toContain('title="BUNIT nJy">nJy</span>');
    expect(wht).toContain(">Use as ERR</button>");
    expect(wht).toContain(">Use as DQ</button>");
    expect(wht).toContain('title="Use HDU 4 (WHT) as the error layer of the shown image"');
    expect(wht).toContain('title="Use HDU 4 (WHT) as the mask of the shown image"');
    expect(wht).not.toContain("layer, from");
    for (const other of [markup(rowOf(tree, 0)), image, mask, variance]) expect(other).not.toContain("Use as");
    expect(markup(rowOf(tree, 0))).not.toContain("BUNIT");
  });

  it("hands the HDU to the layers block", async () => {
    const tree = await loaded({ activePlane: BASE });
    const useAsErr = elements(rowOf(tree, 3)).find((e) => e.type === "button" && textOf(e) === "Use as ERR")!;
    (useAsErr.props.onClick as () => void)();
    const chosen = block(render({ activePlane: BASE }))!;
    expect(chosen.props.pending).toEqual({ role: "err", source: `${FILE}#hdu=3`, name: "HDU 3 (VARIANCE)" });
    expect(chosen.props.imagePath).toBe(SHOWN);
    expect(chosen.props.plane).toBe(BASE);
    (chosen.props.onPendingChange as (pending: null) => void)(null);
    expect(block(render({ activePlane: BASE }))!.props.pending).toBeNull();
    const list = elements(tree).find((e) => String(e.props.className).includes("max-h-[240px]"))!;
    const order = elements(tree);
    expect(order.indexOf(block(tree)!)).toBeLessThan(order.indexOf(list));
  });

  it("offers no row action while the shown plane is itself a layer", async () => {
    const variance: PlaneInfo = { ...BASE, index: 3, extname: "VARIANCE", layer: { role: "variance", basis: "EXTNAME VARIANCE", assumed: false } };
    const tree = await loaded({ activePath: `${FILE}#hdu=3`, activePlane: variance });
    expect(markup(tree)).not.toContain("Use as");
    expect(block(tree)!.props.plane).toBe(variance);
  });

  it("shows the layers block for a file with one HDU", async () => {
    mocks.extensions.mockResolvedValue({ extensions: [hdu(0, "SCI", { role: "science" })] });
    const tree = await loaded({ activePath: `${FILE}#hdu=0`, activePlane: { ...BASE, index: 0 } });
    expect(tree).not.toBeNull();
    expect(block(tree)).toBeDefined();
    expect(textOf(tree)).toContain("Layers");
    expect(textOf(tree)).not.toContain("HDUs");

    for (const activePlane of [null, undefined]) {
      hooks.mount();
      expect(await loaded({ activePath: `${FILE}#hdu=0`, activePlane })).toBeNull();
    }
  });

  it("shows the spinner, not a one-HDU frame, before the list arrives", () => {
    const first = render({ activePlane: IN_USE });
    expect(block(first)).toBeUndefined();
    expect(markup(first)).toContain("animate-spin");
  });

  it("announces a failed read", async () => {
    mocks.extensions.mockRejectedValue(new Error("locked"));
    expect(alertText(await loaded({ activePlane: BASE }))).toEqual(["Failed to read extensions: locked"]);

    hooks.mount();
    mocks.extensions.mockResolvedValue({ extensions: AFW });
    expect(alertText(await loaded({ activePlane: BASE, activateError: "bad plane" }))).toEqual(["Failed to display plane: bad plane"]);

    hooks.mount();
    mocks.header.mockRejectedValue(new Error("gone"));
    const tree = await loaded({ activePlane: BASE });
    const open = elements(rowOf(tree, 2)).find((e) => e.type === "button" && e.props.title === "Show this HDU's header")!;
    (open.props.onClick as () => void)();
    await settle();
    expect(alertText(render({ activePlane: BASE }))).toEqual(["Failed to read HDU header: gone"]);
  });
});

const CUT = "C:/d/cut.fits";
const bandsOf = (index: number) =>
  [..."griz"].map((label, plane) => ({ plane, label, key: `${CUT}#hdu=${index}&plane=${plane}` }));
function cutHdu(index: number, patch: Partial<FitsExtension>): FitsExtension {
  return hdu(index, null, {
    naxis: 3,
    naxis1: 4,
    naxis2: 3,
    naxis3: 4,
    ref: `${CUT}#hdu=${index}`,
    bands: bandsOf(index),
    ...patch,
  });
}
const CUTOUT: FitsExtension[] = [
  cutHdu(0, { role: "science", role_basis: "IMAGETYP IMAGE" }),
  cutHdu(1, { role: "inverse_variance", role_basis: "IMAGETYP INVVAR" }),
  cutHdu(2, { naxis: 2, naxis3: 0, role: "mask", role_basis: "IMAGETYP MASKBITS", bands: [] }),
];
const CUT_PLANE: PlaneInfo = { ...BASE, index: 0, extname: null, source_path: CUT };
const R_SHOWN: PlaneInfo = {
  ...CUT_PLANE,
  band_plane: { hdu: 0, plane: 1, label: "r" },
  err_ref: `${CUT}#hdu=1&plane=1`,
  dq_ref: `${CUT}#hdu=2`,
};
const bandGroup = (tree: ReactNode, index: number) =>
  elements(tree).find((e) => e.props.role === "group" && e.props["aria-label"] === `Bands of HDU ${index}`);
const bandButtons = (tree: ReactNode) =>
  elements(tree).filter((e) => e.type === "button" && String(e.props.title ?? "").startsWith("Display band "));
const cutRow = (tree: ReactNode, index: number) => elements(tree).find((e) => e.key === `${CUT}#hdu=${index}`)!;

describe("HduSelectorPanel band planes", () => {
  beforeEach(() => {
    mocks.extensions.mockResolvedValue({ extensions: CUTOUT });
  });

  it("lists the bands of a cube and displays the one that is clicked", async () => {
    const onActivate = vi.fn();
    const tree = await loaded({ filePath: CUT, activePath: `${CUT}#hdu=0`, activePlane: CUT_PLANE, onActivate });
    const first = bandGroup(tree, 0);
    expect(first).toBeDefined();
    const buttons = elements(first!).filter((e) => e.type === "button");
    expect(buttons.map(textOf)).toEqual(["g", "r", "i", "z"]);
    expect(buttons[1].props.title).toBe("Display band r (plane 1 of HDU 0)");
    expect(bandGroup(tree, 1)).toBeDefined();
    expect(bandGroup(tree, 2)).toBeUndefined();
    (buttons[1].props.onClick as () => void)();
    expect(onActivate).toHaveBeenCalledWith(`${CUT}#hdu=0&plane=1`);
  });

  it("marks the band on screen and keeps the whole cube one click away", async () => {
    const tree = await loaded({ filePath: CUT, activePath: `${CUT}#hdu=0&plane=1`, activePlane: R_SHOWN });
    const buttons = elements(bandGroup(tree, 0)!).filter((e) => e.type === "button");
    expect(buttons.map((b) => b.props["aria-pressed"])).toEqual([false, true, false, false]);
    expect(buttons.map((b) => b.props.disabled)).toEqual([false, true, false, false]);
    const inSecond = elements(bandGroup(tree, 1)!).filter((e) => e.type === "button");
    expect(inSecond.map((b) => b.props["aria-pressed"])).toEqual([false, false, false, false]);
    const cube = markup(cutRow(tree, 0));
    expect(cube).toContain(">Display</button>");
    expect(cube).not.toContain("Currently displayed plane");
    const invvar = markup(cutRow(tree, 1));
    expect(invvar).toContain('title="Error layer of the shown image">error layer</span>');
    expect(invvar).not.toContain("Use as ERR");
    expect(markup(cutRow(tree, 2))).toContain('title="Mask of the shown image">mask</span>');
  });

  it("bands wait while a plane is being displayed", async () => {
    const busy = await loaded({ filePath: CUT, activePath: `${CUT}#hdu=0`, activePlane: CUT_PLANE, activating: true });
    expect(bandButtons(busy)).toHaveLength(8);
    expect(bandButtons(busy).every((b) => b.props.disabled === true)).toBe(true);

    hooks.mount();
    const idle = await loaded({ filePath: CUT, activePath: `${CUT}#hdu=0`, activePlane: CUT_PLANE, onActivate: undefined });
    expect(bandButtons(idle)).toHaveLength(8);
    expect(bandButtons(idle).every((b) => b.props.disabled === true)).toBe(true);
  });
});
