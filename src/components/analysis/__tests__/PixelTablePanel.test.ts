import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { PixelTableResult } from "../../../shared/types/analysis";
import type { PixelTableLayers } from "../../../shared/types/layers";

const mocks = vi.hoisted(() => ({ table: vi.fn<(...args: unknown[]) => Promise<unknown>>(), clipboard: vi.fn<(text: string) => Promise<void>>() }));
const click = vi.hoisted(() => ({ value: null as { x: number; y: number; seq: number } | null }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/analysis", () => ({ pixelTable: mocks.table }));
vi.mock("../../../hooks/useMousePixelStore", () => ({ usePixelClick: () => click.value, useMousePixel: () => null }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "a.fits", image: "original" }) }));
vi.mock("../../../context/ToolHostContext", () => ({ useToolHost: () => ({ active: true }) }));
vi.mock("../MeasurementBadge", () => ({ default: () => null }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, findElement, textOf } from "../../../test/elementTree";
import { ErrorAlert } from "../../ui";
import PixelTablePanel from "../PixelTablePanel";
import { measurementLog } from "../../../utils/measurementLog";

type Props = Record<string, unknown> & { children?: ReactNode };

const FLOAT_MASK = "mask layer HDU 2 is a floating-point plane (BITPIX -32): flag bits cannot be read exactly";
const STATS = { min: 1, max: 9, mean: 5, median: 5, n_finite: 9, n_nan: 0 };

function fixture(overrides: Partial<PixelTableResult & PixelTableLayers> = {}): PixelTableResult & PixelTableLayers {
  return {
    x: 1,
    y: 1,
    size: 3,
    x0: 0,
    y0: 0,
    values: [
      [1, 2, 3],
      [4, 9, 5],
      [6, 7, 8],
    ],
    err: null,
    dq: null,
    dq_names: [
      [null, null, null],
      [null, "SATURATED | JUMP_DET", null],
      [null, null, null],
    ],
    dq_table: "jwst",
    unit: null,
    stats: STATS,
    err_stats: null,
    wavelength: null,
    wavelength_stats: null,
    wavelength_unit: null,
    elapsed_ms: 1,
    ...overrides,
  };
}

const PROPS = { filePath: "a.fits", measureKey: null };
type Panel = (props: typeof PROPS) => unknown;
const show = () => hooks.render((PixelTablePanel as unknown as { type: Panel }).type, PROPS) as ReactNode;
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

async function loaded(): Promise<ReactNode> {
  const first = show();
  const toggle = findElement<Props>(first, (e) => e.props.label === "Read on image click")!;
  (toggle.props.onChange as (v: boolean) => void)(true);
  click.value = { x: 1, y: 1, seq: 1 };
  show();
  await flush();
  return show();
}

function cells(tree: ReactNode): ReactElement<Props>[] {
  return elementsOf<Props>(tree).filter((e) => e.type === "td" && typeof e.props.title === "string");
}

function summaryRow(tree: ReactNode): string {
  const row = findElement<Props>(tree, (e) => typeof e.props.className === "string" && e.props.className.startsWith("text-[9px] text-zinc-500 font-mono"));
  return textOf(row);
}

function statusTexts(tree: ReactNode): string[] {
  return elementsOf<Props>(tree)
    .filter((e) => e.props.role === "status")
    .map((e) => textOf(e));
}

describe("PixelTablePanel marks, key and layer notes", () => {
  beforeEach(() => {
    hooks.mount();
    click.value = null;
    mocks.table.mockReset().mockResolvedValue(fixture());
  });

  afterEach(() => {
    hooks.mount();
  });

  it("marks a flagged cell and the clicked cell without relying on colour", async () => {
    const tds = cells(await loaded());
    expect(tds).toHaveLength(9);
    const centre = tds[4];
    expect(String(centre.props.className)).toContain("decoration-dotted");
    const spans = elementsOf<Props>(centre.props.children).filter((e) => e.type === "span");
    const glyph = spans.find((e) => e.props["aria-hidden"] === "true");
    expect(glyph && textOf(glyph)).toBe("*");
    const sr = spans.find((e) => e.props.className === "sr-only");
    expect(sr && textOf(sr)).toBe("DQ: SATURATED | JUMP_DET, clicked pixel");
    const plain = tds[0];
    expect(String(plain.props.className)).not.toContain("decoration-dotted");
    expect(elementsOf<Props>(plain.props.children).filter((e) => e.type === "span")).toEqual([]);
  });

  it("shows a key of the tints and names the clicked pixel", async () => {
    const tree = await loaded();
    const key = findElement<Props>(tree, (e) => "data-pixel-key" in e.props);
    expect(key).toBeDefined();
    const keyText = textOf(key);
    for (const label of ["DQ flagged *", "max", "min", "no value"]) expect(keyText).toContain(label);
    expect(summaryRow(tree)).toContain("clicked (1, 1): DQ SATURATED | JUMP_DET");
  });

  it("says where the error comes from and why a layer is not used", async () => {
    const err = [
      [1, 1, 1],
      [1, 5, 1],
      [1, 1, 1],
    ];
    mocks.table.mockResolvedValue(fixture({ err, err_stats: STATS, err_source: "EXTNAME VARIANCE, HDU 3: square root" }));
    const withErr = await loaded();
    expect(summaryRow(withErr)).toContain("ERR EXTNAME VARIANCE, HDU 3: square root");

    hooks.mount();
    click.value = null;
    mocks.table.mockResolvedValue(fixture({ dq_unavailable: FLOAT_MASK }));
    const withGap = await loaded();
    expect(statusTexts(withGap)).toContain(`DQ not used: ${FLOAT_MASK}`);

    hooks.mount();
    click.value = null;
    mocks.table.mockResolvedValue(fixture());
    const bare = await loaded();
    expect(summaryRow(bare)).not.toContain("ERR ");
    expect(statusTexts(bare).some((t) => t.includes("not used"))).toBe(false);
  });

  it("announces a failed read", async () => {
    mocks.table.mockReset().mockRejectedValue(new Error("out of range"));
    const tree = await loaded();
    const box = findElement<Props>(tree, (e) => e.type === ErrorAlert && e.props.message === "out of range");
    expect(box).toBeDefined();
    expect(renderToStaticMarkup(box!)).toContain('role="alert"');
  });

  it("shows the pixel table failure through the compact alert", async () => {
    mocks.table.mockReset().mockRejectedValue(new Error("out of range"));
    const tree = await loaded();
    const alert = findElement<Props>(tree, (e) => e.type === ErrorAlert);
    expect(alert).toBeDefined();
    expect(alert!.props.message).toBe("out of range");
    expect(alert!.props.compact).toBe(true);
    expect(renderToStaticMarkup(alert!)).toBe('<div class="ab-error-alert ab-error-alert-compact" role="alert">out of range</div>');
  });
});

function button(tree: ReactNode, label: string): ReactElement<Props> {
  return elementsOf<Props>(tree).find((e) => e.type === "button" && textOf(e).trim() === label)!;
}

describe("PixelTablePanel copy and log", () => {
  beforeEach(() => {
    hooks.mount();
    click.value = null;
    mocks.table.mockReset().mockResolvedValue(fixture());
    mocks.clipboard.mockReset().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText: mocks.clipboard } });
    vi.spyOn(measurementLog, "append").mockImplementation((draft) => ({ ...draft, id: "id1", timestamp_utc: "2026-11-12T10:00:00.000Z" }));
  });

  afterEach(() => {
    hooks.mount();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("copies the shown plane with its provenance", async () => {
    const tree = await loaded();
    await (button(tree, "Copy CSV").props.onClick as () => Promise<void>)();
    expect(mocks.clipboard).toHaveBeenCalledTimes(1);
    const text = mocks.clipboard.mock.calls[0][0];
    for (const part of ["# file: a.fits", "# plane: SCI", "# centre_x: 1", "# centre_y: 1"]) expect(text).toContain(part);
    expect(text.split("\r\n")).toContain("y\\x,0,1,2");
  });

  it("logs the pixel with the origin of its error", async () => {
    mocks.table.mockResolvedValue(fixture({ err_source: "EXTNAME VARIANCE, HDU 3: square root", dq_unavailable: FLOAT_MASK }));
    const tree = await loaded();
    (button(tree, "Log").props.onClick as () => void)();
    expect(measurementLog.append).toHaveBeenCalledTimes(1);
    const draft = vi.mocked(measurementLog.append).mock.calls[0][0];
    expect(draft.kind).toBe("pixel");
    expect(draft.params.err_source).toBe("EXTNAME VARIANCE, HDU 3: square root");
    expect(draft.notes).toContain(`DQ not used: ${FLOAT_MASK}`);
  });
});
