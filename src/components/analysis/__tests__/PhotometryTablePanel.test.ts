import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";

const FIRST = "C:/data/502nmos.fits";
const SECOND = "C:/data/f200w_i2d.fits";

const backend = vi.hoisted(() => ({
  batch: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  saved: [] as string[],
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/analysis", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/analysis")>()),
  photometryGainModel: () => new Promise(() => {}),
  measurePhotometryBatch: backend.batch,
}));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => ({ excludeDq: false }) }));
vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementProvenance: () => ({ file: "502nmos.fits", image: "original" }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async (dialog: { defaultPath: string }) => `C:/out/${dialog.defaultPath}` }));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeTextFile: async (path: string) => void backend.saved.push(path) }));

import { hooks } from "../../../test/hookRuntime";

const { default: PhotometryTablePanel } = await import("../PhotometryTablePanel");

type Props = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): Props[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Props>(node)) return [];
  return [node.props, ...elements(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<Props>(node) ? textOf(node.props.children) : "";
}

function measuredStar(): Record<string, unknown> {
  return {
    x: 10.04, y: 12.06, peak: 900, net_flux: 1234.5678, flux_err: 12, mag_inst: -7.73, snr: 102.9, fwhm: 3.214,
    aperture_radius: 5, aperture_pixels: 78, aperture_area: 78.5, bg_mean: 10.25, bg_sigma: 2, bg_pixels: 200,
    saturated: false, saturation_source: "none", n_masked: 0, n_saturated: 0, err_used: true,
    aperture_correction: 1.0421, flux_total: 1286.6, plateau_radius: 9, flux_jy: null, flux_err_jy: null,
    mag_ab: 24.5123, mag_ab_err: 0.0314, mag_ab_total: null, st_mag: null, sky_inner: 10, sky_outer: 15, ee50_radius: 2.345,
  };
}

const STARS = [{ x: 10, y: 12, flux: 100, fwhm: 3, snr: 50 }];
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

type Panel = (props: Record<string, unknown>) => unknown;
const panel = (PhotometryTablePanel as unknown as { type: Panel }).type;
const show = (filePath = FIRST) => hooks.render(panel, { filePath, overlayKey: null, stars: STARS }) as ReactNode;
const notice = (filePath = FIRST) =>
  elements(show(filePath)).find((p) => typeof p.className === "string" && p.className.includes("text-emerald-400/90"));

function click(text: string): void {
  const found = elements(show()).find((p) => p.children !== undefined && textOf(p.children as ReactNode).trim() === text);
  expect(found?.disabled, `${text} is enabled`).not.toBe(true);
  (found!.onClick as () => void)();
}

async function measured(): Promise<void> {
  const run = elements(show()).find((p) => p.label === "Measure 1");
  (run!.onClick as () => void)();
  await flush();
}

async function savedOnce(): Promise<void> {
  await measured();
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true });
  click("Save CSV");
  await vi.waitFor(() => expect(backend.saved).toHaveLength(1));
  expect(textOf(notice()?.children as ReactNode)).toBe(`Saved ${backend.saved[0]}`);
}

describe("PhotometryTablePanel Saved notice", () => {
  beforeEach(() => {
    hooks.mount();
    backend.saved = [];
    backend.batch.mockReset().mockResolvedValue({
      rows: [{ index: 0, photometry: measuredStar(), sky: null, error: null }],
      photcal: null,
      warnings: [],
      masked: false,
      n_measured: 1,
      n_failed: 0,
      elapsed_ms: 8,
    });
  });

  afterEach(() => {
    hooks.mount();
    vi.useRealTimers();
  });

  it("cancels the notice timer when the panel closes", async () => {
    await savedOnce();
    expect(vi.getTimerCount(), "the notice timer runs while the panel is open").toBe(1);
    hooks.mount();
    expect(vi.getTimerCount(), "no timer outlives the panel").toBe(0);
  });

  it("shows each save in one timed notice that a later save restarts", async () => {
    await savedOnce();
    await vi.advanceTimersByTimeAsync(3_000);
    click("Save CSV");
    await vi.waitFor(() => expect(backend.saved).toHaveLength(2));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(notice(), "the second save is still shown 5 s after it").toBeDefined();
    await vi.advanceTimersByTimeAsync(1_500);
    expect(notice()).toBeUndefined();
  });

  it("drops the notice and its timer when the table is cleared", async () => {
    await savedOnce();
    click("Clear");
    expect(notice()).toBeUndefined();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("drops the notice and its timer when another file is measured", async () => {
    await savedOnce();
    show(SECOND);
    expect(notice(SECOND)).toBeUndefined();
    expect(vi.getTimerCount()).toBe(0);
  });
});
