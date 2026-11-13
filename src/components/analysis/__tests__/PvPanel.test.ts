import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { PvDiagramResult } from "../../../shared/types/pv";

const backend = vi.hoisted(() => ({ run: vi.fn(), publish: vi.fn(), log: vi.fn() }));
const dialog = vi.hoisted(() => ({ save: vi.fn() }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/pv", () => ({ computePvDiagram: backend.run }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: async () => "C:/out" }));
vi.mock("../../../context/PreviewContext", () => ({
  useRenderActions: () => ({ publishProcessed: backend.publish, resetProcessed: () => {} }),
  useRenderContext: () => ({ processed: null }),
}));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => "cube.fits" }));
vi.mock("../../../hooks/useRegionStore", () => ({
  useRegionDoc: () => ({
    selectedId: "line",
    regions: [{ id: "line", shape: { shape: "line", x1: 1, y1: 1, x2: 4, y2: 1 }, props: { include: true }, backgroundId: null }],
  }),
}));
vi.mock("../../../utils/measurementLog", () => ({ measurementLog: { append: backend.log }, pvEntry: (value: unknown) => value }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: dialog.save }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, findElement, textOf } from "../../../test/elementTree";
import { CopyCsvButton, ErrorAlert, SMALL_BUTTON_CLASS } from "../../ui";
import PvPanel from "../PvPanel";

type Props = Record<string, unknown>;

const RESULT: PvDiagramResult = {
  fits_path: "C:/out/cube_pv_0-39_2-16_29-16.fits",
  png_path: "C:/out/cube_pv_0-39_2-16_29-16.png",
  dimensions: [3, 40],
  offsets: [-0.36, 0, 0.36],
  offset_unit: "arcsec",
  offset_step: 0.36,
  spectral_values: [-100, 0, 100],
  spectral_unit: "km/s",
  spectral_mode: "velocity",
  spectral_shift_kms: -7.25,
  spectral_rest_um: 1.02,
  spectral_convention: "optical",
  ridge: [-50, null, 50],
  ridge_channel: [19.5, null, 20.5],
  peak_channel: [19, null, 21],
  peak_value: [1.5, null, 1.4],
  peak_spectral: [-50, null, 50],
  xs: [2, 3, 4],
  ys: [16, 16, 16],
  line: { x0: 2, y0: 16, x1: 29, y1: 16 },
  step_px: 1,
  width_px: 1,
  z0: 0,
  z1: 39,
  n_across: 1,
  summary: {
    n_offsets: 3,
    n_channels: 40,
    n_across: 1,
    n_off_image: 0,
    offset_step: 0.36,
    offset_unit: "arcsec",
    slit_length_px: 27,
    slit_length: 9.72,
    slit_pa_deg: 90,
    pixel_scale_arcsec: 0.36,
    ridge_gradient: 138.9,
    ridge_gradient_unit: "km/s/arcsec",
    ridge_span: 100,
    n_ridge_valid: 2,
    peak_value: 1.5,
    peak_offset: -0.36,
    peak_spectral: -50,
    bunit: "Jy/beam",
  },
  notes: ["a", "b"],
  elapsed_ms: 12,
  previewUrl: "pv.png",
};

const PANEL_PROPS = { filePath: "cube.fits", fileKey: "file|cube.fits", inputRevision: "revision-1", cubeDims: { frames: 4 } };
const panel = (PvPanel as unknown as { type: (props: Props) => ReactNode }).type;

function show(): ReactNode {
  hooks.render(panel, PANEL_PROPS);
  return hooks.render(panel, PANEL_PROPS) as ReactNode;
}

function buttonNamed(tree: ReactNode, label: string): ReactElement<Props> {
  const button = findElement<Props>(tree, (el) => el.type === "button" && textOf(el) === label);
  if (!button) throw new Error(`${label} control missing`);
  return button;
}

function runPv(): Promise<void> {
  const button = findElement<Props>(show(), (el) => el.type === "button" && textOf(el).includes("Run PV"));
  if (!button) throw new Error("Run PV control missing");
  return (button.props.onClick as () => Promise<void>)();
}

function copyButton(tree: ReactNode): ReactElement<Props> | undefined {
  return findElement<Props>(tree, (el) => el.type === CopyCsvButton);
}

function alertMessages(tree: ReactNode): unknown[] {
  return elementsOf<Props>(tree)
    .filter((el) => el.type === ErrorAlert)
    .map((el) => el.props.message);
}

function compactAlert(tree: ReactNode, message: string): ReactElement<Props> | undefined {
  return findElement<Props>(tree, (el) => el.type === ErrorAlert && el.props.message === message);
}

beforeEach(() => {
  hooks.mount();
  backend.run.mockReset().mockResolvedValue(RESULT);
  backend.publish.mockReset();
  backend.log.mockReset();
  dialog.save.mockReset();
});

afterEach(() => hooks.mount());

describe("PV panel failures and copy", () => {
  it("announces a failed run through the compact alert", async () => {
    backend.run.mockRejectedValue(new Error("the slit lies outside the cube"));
    await runPv();
    const alert = compactAlert(show(), "the slit lies outside the cube");
    expect(alert).toBeDefined();
    expect(alert?.props.compact).toBe(true);
  });

  it("hands the ridge table to the shared copy button", async () => {
    await runPv();
    const button = copyButton(show());
    expect(button).toBeDefined();
    expect(button?.props.title).toBe("Copy the ridge table as CSV");
    expect(button?.props.disabled).toBeUndefined();
    expect((button?.props.text as () => string | null)()).toContain("offset_arcsec,");
  });

  it("has nothing to copy before a run", () => {
    expect(copyButton(show())).toBeUndefined();
  });

  it("shows a refused clipboard write and clears it at the next copy", async () => {
    await runPv();
    const button = copyButton(show());
    if (!button) throw new Error("copy control missing");
    (button.props.onError as (message: string) => void)("Clipboard copy failed: Document is not focused.");
    const alert = compactAlert(show(), "Clipboard copy failed: Document is not focused.");
    expect(alert).toBeDefined();
    expect(alert?.props.compact).toBe(true);
    (button.props.text as () => string | null)();
    expect(alertMessages(show()).every((message) => message === null)).toBe(true);
  });

  it("shows a failed save through the same alert", async () => {
    await runPv();
    dialog.save.mockRejectedValue(new Error("the dialog was closed by the system"));
    await (buttonNamed(show(), "Save CSV").props.onClick as () => Promise<void>)();
    const alert = compactAlert(show(), "the dialog was closed by the system");
    expect(alert).toBeDefined();
    expect(alert?.props.compact).toBe(true);
  });
});

describe("PV panel readout cards and buttons", () => {
  it("prints the whole peak, with a title that repeats it", async () => {
    await runPv();
    const peak = findElement<Props>(show(), (el) => el.props.label === "peak");
    if (!peak) throw new Error("peak card missing");
    expect(renderToStaticMarkup(peak)).toBe(
      '<div class="bg-zinc-900/80 rounded px-2 py-1.5" title="1.500 Jy/beam at -0.3600 arcsec, -50.00 km/s"><div class="text-[9px] text-zinc-500 uppercase">peak</div><div class="text-[10px] font-mono text-zinc-200 break-words">1.500 Jy/beam at -0.3600 arcsec, -50.00 km/s</div></div>',
    );
  });

  it("keeps the title a card was given", async () => {
    await runPv();
    const gradient = findElement<Props>(show(), (el) => el.props.label === "gradient");
    if (!gradient) throw new Error("gradient card missing");
    expect(renderToStaticMarkup(gradient)).toContain('title="Least-squares slope of the ridge over the offsets"');
  });

  it("draws its small buttons with the shared class", async () => {
    await runPv();
    const tree = show();
    expect(buttonNamed(tree, "Show cube").props.className).toBe(SMALL_BUTTON_CLASS);
    expect(buttonNamed(tree, "Save CSV").props.className).toBe(SMALL_BUTTON_CLASS);
  });
});
