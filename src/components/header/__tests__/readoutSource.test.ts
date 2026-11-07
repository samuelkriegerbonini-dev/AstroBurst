import { isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hooks } from "../../../test/hookRuntime";
import type { PixelProbeResult } from "../../../shared/types/analysis";
import type { PixelToWorldResult, WcsInfo } from "../../../shared/types/astrometry";

const mocks = vi.hoisted(() => ({ info: vi.fn(), world: vi.fn(), probe: vi.fn() }));
vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/astrometry", () => ({ getWcsInfo: mocks.info, pixelToWorld: mocks.world }));
vi.mock("../../../services/display", () => ({ probePixel: mocks.probe }));

import WcsReadout from "../WcsReadout";
import PixelReadout from "../PixelReadout";

const pixelProps: { filePath: string; mouseX: number | null; mouseY: number | null } = { filePath: "a.fits", mouseX: 4, mouseY: 5 };
const wcsProps = { ...pixelProps, imageWidth: 200, imageHeight: 100 };
const pixelComponent = (PixelReadout as unknown as { type: (props: typeof pixelProps) => ReactNode }).type;
const wcsComponent = (WcsReadout as unknown as { type: (props: typeof wcsProps) => ReactNode }).type;
const pixel = (props = pixelProps) => renderToStaticMarkup(hooks.render(pixelComponent, props) as ReactNode);
const wcsTree = (props = wcsProps) => hooks.render(wcsComponent, props) as ReactNode;
const wcs = (props = wcsProps) => renderToStaticMarkup(wcsTree(props));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function probe(value: number): PixelProbeResult {
  return { x: 4, y: 5, box: 5, value, unit: "adu", neighborhood: { min: value, max: value, mean: value, median: value, n_pixels: 25, n_nan: 0 } } as PixelProbeResult;
}

function info(ra: number, dec: number): WcsInfo {
  return { center_ra: ra, center_dec: dec, pixel_scale_arcsec: 2, fov_arcmin: [1, 2], field_of_view_w_arcmin: 1, field_of_view_h_arcmin: 2, naxis1: 200, naxis2: 100 };
}

type SelectProps = { children?: ReactNode; className?: string; "aria-label"?: string; onChange?: (event: unknown) => void };

function selects(node: ReactNode): SelectProps[] {
  if (Array.isArray(node)) return node.flatMap(selects);
  if (!isValidElement<SelectProps>(node)) return [];
  return node.type === "select" ? [node.props] : selects(node.props.children);
}

function choose(node: ReactNode, label: string, value: string): void {
  for (const select of selects(node)) if (select["aria-label"] === label) select.onChange?.({ target: { value } });
}

function selectFrame(node: ReactNode, frame: string): void {
  choose(node, "Coordinate frame", frame);
}

async function resolveInitialWcs(): Promise<void> {
  wcs();
  await vi.advanceTimersByTimeAsync(0);
  wcs();
  await vi.advanceTimersByTimeAsync(40);
  expect(wcs()).toContain("11.125000");
}

beforeEach(() => {
  hooks.mount();
  vi.useFakeTimers();
  vi.resetAllMocks();
  vi.stubGlobal("window", { localStorage: { getItem: () => '{"frame":"icrs","format":"decimal"}', setItem: vi.fn() } });
  mocks.probe.mockResolvedValue(probe(12345));
  mocks.info.mockResolvedValue(info(30, 10));
  mocks.world.mockResolvedValue({ points: [[11.125, 22.25]], frame: "icrs" });
});

afterEach(() => {
  hooks.mount();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("Pixel readout source", () => {
  it("hides a previous file's values immediately while the new file is pending or fails", async () => {
    pixel();
    await vi.advanceTimersByTimeAsync(40);
    expect(pixel()).toContain("12345 adu");
    const next = deferred<PixelProbeResult>();
    mocks.probe.mockReturnValue(next.promise);
    const props = { ...pixelProps, filePath: "b.fits" };
    expect(pixel(props)).toBe("");
    await vi.advanceTimersByTimeAsync(40);
    expect(mocks.probe).toHaveBeenLastCalledWith("b.fits", 4, 5, 5);
    next.reject(new Error("Unavailable"));
    await vi.advanceTimersByTimeAsync(0);
    expect(pixel(props)).toBe("");
  });

  it("keeps the last value while moving within one file and preserves probe arguments", async () => {
    pixel();
    await vi.advanceTimersByTimeAsync(40);
    const next = deferred<PixelProbeResult>();
    mocks.probe.mockReturnValue(next.promise);
    const props = { ...pixelProps, mouseX: 8, mouseY: 9 };
    expect(pixel(props)).toContain("12345 adu");
    await vi.advanceTimersByTimeAsync(40);
    expect(mocks.probe).toHaveBeenLastCalledWith("a.fits", 8, 9, 5);
    next.resolve(probe(67890));
    await vi.advanceTimersByTimeAsync(0);
    expect(pixel(props)).toContain("67890 adu");
  });

  it("ignores a late value from a file that is no longer selected", async () => {
    const first = deferred<PixelProbeResult>();
    mocks.probe.mockReturnValueOnce(first.promise).mockResolvedValueOnce(probe(67890));
    pixel();
    await vi.advanceTimersByTimeAsync(40);
    const props = { ...pixelProps, filePath: "b.fits" };
    pixel(props);
    await vi.advanceTimersByTimeAsync(40);
    first.resolve(probe(12345));
    await vi.advanceTimersByTimeAsync(0);
    expect(pixel(props)).toContain("67890 adu");
    expect(pixel(props)).not.toContain("12345");
  });

  it("clears the value and sends no probe when the pointer leaves the image", async () => {
    pixel();
    await vi.advanceTimersByTimeAsync(40);
    expect(pixel()).toContain("12345 adu");
    const probes = mocks.probe.mock.calls.length;
    const left = { ...pixelProps, mouseX: null, mouseY: null };
    pixel(left);
    expect(pixel(left)).toBe("");
    await vi.advanceTimersByTimeAsync(40);
    expect(mocks.probe).toHaveBeenCalledTimes(probes);
    expect(pixel(left)).toBe("");
  });
});

describe("WCS readout source", () => {
  it("hides previous coordinates and metadata immediately when the next file is pending or fails", async () => {
    await resolveInitialWcs();
    const next = deferred<WcsInfo>();
    mocks.info.mockReturnValue(next.promise);
    const props = { ...wcsProps, filePath: "b.fits" };
    expect(wcs(props)).toBe("");
    expect(mocks.info).toHaveBeenLastCalledWith("b.fits");
    next.reject(new Error("No WCS"));
    await vi.advanceTimersByTimeAsync(40);
    expect(wcs(props)).toBe("");
    expect(mocks.world.mock.calls.every(([path]) => path === "a.fits")).toBe(true);
  });

  it("uses the new file's center while its hover coordinates are still pending", async () => {
    await resolveInitialWcs();
    mocks.info.mockResolvedValue(info(44, 55));
    mocks.world.mockReturnValue(deferred<PixelToWorldResult>().promise);
    const props = { ...wcsProps, filePath: "b.fits" };
    wcs(props);
    await vi.advanceTimersByTimeAsync(0);
    wcs(props);
    const html = wcs(props);
    expect(html).toContain("44.000000");
    expect(html).not.toContain("11.125000");
    await vi.advanceTimersByTimeAsync(40);
    expect(mocks.world).toHaveBeenLastCalledWith("b.fits", [[4, 5]], "icrs");
  });

  it("does not relabel old coordinates with a newly selected sky frame", async () => {
    await resolveInitialWcs();
    const next = deferred<PixelToWorldResult>();
    mocks.world.mockReturnValue(next.promise);
    selectFrame(wcsTree(), "galactic");
    const html = wcs();
    expect(html).not.toContain("11.125000");
    expect(html).not.toContain("30.000000");
    expect(mocks.world).toHaveBeenLastCalledWith("a.fits", [[100, 50]], "galactic");
    next.resolve({ points: [[66, 77]], frame: "galactic" });
    await vi.advanceTimersByTimeAsync(0);
    expect(wcs()).toContain("66.000000");
  });

  it("keeps coordinates while the pointer moves within the same file and frame", async () => {
    await resolveInitialWcs();
    mocks.world.mockReturnValue(deferred<PixelToWorldResult>().promise);
    const props = { ...wcsProps, mouseX: 8, mouseY: 9 };
    expect(wcs(props)).toContain("11.125000");
    await vi.advanceTimersByTimeAsync(40);
    expect(mocks.world).toHaveBeenLastCalledWith("a.fits", [[8, 9]], "icrs");
    expect(wcs(props)).toContain("11.125000");
  });

  it("falls back to the image centre without a pixel label when the pointer leaves", async () => {
    await resolveInitialWcs();
    expect(wcs()).toContain("px(4,5)");
    const lookups = mocks.world.mock.calls.length;
    const left = { ...wcsProps, mouseX: null, mouseY: null };
    wcs(left);
    const html = wcs(left);
    expect(html).toContain("30.000000");
    expect(html).toContain("10.000000");
    expect(html).not.toContain("11.125000");
    expect(html).not.toContain("px(");
    await vi.advanceTimersByTimeAsync(40);
    expect(mocks.world).toHaveBeenCalledTimes(lookups);
  });

  it("switches to sexagesimal through the format select and remembers the choice", async () => {
    await resolveInitialWcs();
    choose(wcsTree(), "Coordinate format", "sexagesimal");
    const html = wcs();
    expect(html).toContain("00h44m30.00s");
    expect(html).toContain("+22°15");
    expect(html).not.toContain("11.125000");
    const setItem = (window.localStorage.setItem as unknown as ReturnType<typeof vi.fn>).mock.calls;
    expect(setItem.at(-1)).toEqual(["astroburst.readout.v1", JSON.stringify({ frame: "icrs", format: "sexagesimal" })]);
  });

  it("gives the coordinate selects the dark select background so their option lists stay readable", async () => {
    await resolveInitialWcs();
    const found = selects(wcsTree());
    expect(found.map((select) => select["aria-label"])).toEqual(["Coordinate frame", "Coordinate format"]);
    for (const select of found) {
      const classes = String(select.className).split(" ");
      expect(classes).toContain("bg-zinc-900");
      expect(classes).not.toContain("bg-transparent");
    }
  });
});
