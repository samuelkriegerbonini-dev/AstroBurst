import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { ProcessedFile } from "../../../shared/types";
import type { DrizzleRgbResult } from "../../../shared/types/stacking";

const ingest = vi.hoisted(() => ({ showWhenLoaded: vi.fn(), ingestUnlessLoaded: vi.fn() }));
const percentOf = vi.hoisted(() => ({} as Record<string, number>));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../hooks/useProgress", () => ({
  useProgress: (event: string) => ({ active: false, percent: percentOf[event] ?? 0, stage: "", current: 0, total: 0, reset: () => {} }),
}));
vi.mock("../../../services/stacking", () => ({ drizzleRgbStack: vi.fn(), drizzleFrames: vi.fn() }));
vi.mock("../../../services/progress", () => ({ cancelProgress: vi.fn() }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));
vi.mock("../../../hooks/useFileIngest", () => ingest);

import { hooks } from "../../../test/hookRuntime";
import { drizzleFrames, drizzleRgbStack } from "../../../services/stacking";
import { cancelProgress } from "../../../services/progress";
import { getOutputDir } from "../../../infrastructure/tauri";
import { claimStackProgress, releaseStackProgress, stackProgressHolder } from "../../../utils/stackProgressLock";
import { DRIZZLE_FIXED_REJECTION } from "../../../utils/drizzleParams";
import type { StackResult } from "../../../shared/types/stacking";
import DrizzleRgbPanel from "../DrizzleRgbPanel";

type AnyProps = Record<string, unknown> & { children?: ReactNode };
const panel = DrizzleRgbPanel as unknown as (props: Parameters<typeof DrizzleRgbPanel>[0]) => ReactNode;

function frame(name: string, header: Record<string, string> | null = null): ProcessedFile {
  return {
    id: name,
    name,
    path: `C:/m42/${name}`,
    sourcePath: `C:/m42/${name}`,
    imageRef: null,
    size: 0,
    status: "done",
    result: header ? ({ header } as ProcessedFile["result"]) : null,
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

const FRAMES = ["m42_R_1.fits", "m42_R_2.fits", "m42_G_1.fits", "m42_G_2.fits"].map((name) => frame(name));

function nodes(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...nodes(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<AnyProps>(node) ? textOf(node.props.children) : "";
}

const named = (tree: ReactNode, name: string) => nodes(tree).find((el) => typeof el.props.onClick === "function" && textOf(el.props.children).trim() === name);

const REPLY: DrizzleRgbResult = {
  png_path: "C:/out/m42_drizzle.png",
  fits_path: null,
  dimensions: [16, 16],
  output_dims: [16, 16],
  input_dims: [8, 8],
  frame_count_r: 2,
  frame_count_g: 2,
  frame_count_b: 0,
  rejected_pixels: 0,
  scale: 2,
  elapsed_ms: 5,
};

describe("DrizzleRgbPanel Open in Files", () => {
  beforeEach(() => {
    hooks.mount();
    ingest.showWhenLoaded.mockReset();
    vi.mocked(drizzleRgbStack).mockReset();
  });

  afterEach(() => hooks.mount());

  async function drizzled(reply: Partial<DrizzleRgbResult>): Promise<ReactNode> {
    vi.mocked(drizzleRgbStack).mockResolvedValue({ ...REPLY, ...reply });
    const view = () => hooks.render(panel, { files: FRAMES }) as ReactNode;
    (named(view(), "Auto")?.props.onClick as () => void)();
    void (nodes(view()).find((el) => el.props.runningLabel === "Drizzling...")?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(textOf(view())).toContain("Drizzle RGB Complete"));
    expect(vi.mocked(drizzleRgbStack).mock.calls[0].slice(0, 3)).toEqual([
      ["C:/m42/m42_R_1.fits", "C:/m42/m42_R_2.fits"],
      ["C:/m42/m42_G_1.fits", "C:/m42/m42_G_2.fits"],
      [],
    ]);
    return view();
  }

  it("adds the drizzled FITS to Files and shows it", async () => {
    const tree = await drizzled({ fits_path: "C:/out/m42_drizzle.fits" });
    (named(tree, "Open in Files")?.props.onClick as () => void)();
    expect(ingest.showWhenLoaded.mock.calls).toEqual([["C:/out/m42_drizzle.fits"]]);
  });

  it("offers no Open in Files when the drizzle wrote no FITS", async () => {
    expect(named(await drizzled({ fits_path: null }), "Open in Files")).toBeUndefined();
  });
});

const SX13_LIGHTS = ["frame_0001.fits", "frame_0002.fits", "frame_0003.fits", "frame_0004.fits", "frame_0005.fits", "frame_0006.fits"].map((name) =>
  frame(name, { IMAGETYP: "LIGHT", FILTER: "R" }),
);
const SX13_FLATS = ["frame_0007.fits", "frame_0008.fits"].map((name) => frame(name, { IMAGETYP: "FLAT", FILTER: "R" }));
const SX13 = [...SX13_LIGHTS, ...SX13_FLATS];

const MONO_REPLY: StackResult = {
  png_path: "C:/out/frame_0001_drizzle6.png",
  fits_path: "C:/out/frame_0001_drizzle6.fits",
  dimensions: [16, 16],
  frame_count: 6,
  rejected_pixels: 0,
  scale: 2,
  elapsed_ms: 5,
};

const byTestId = (tree: ReactNode, id: string) => nodes(tree).find((el) => el.props["data-testid"] === id);
const runButton = (tree: ReactNode) => nodes(tree).find((el) => el.props.runningLabel === "Drizzling...");
const labelled = (tree: ReactNode, label: string) => nodes(tree).find((el) => el.props.label === label);
const hasText = (tree: ReactNode, text: string) => nodes(tree).some((el) => typeof el.props.children === "string" && el.props.children === text);

describe("DrizzleRgbPanel Auto from the FITS header", () => {
  beforeEach(() => hooks.mount());
  afterEach(() => hooks.mount());

  it("puts the FILTER=R lights in R and names the R flats it leaves out", () => {
    const view = () => hooks.render(panel, { files: SX13 }) as ReactNode;
    (named(view(), "Auto")?.props.onClick as () => void)();
    const tree = view();
    expect(labelled(tree, "Drizzle R (mono)")).toBeDefined();
    expect(nodes(tree).find((el) => el.props.title === "Drizzle RGB")?.props.subtitle).toBe("6 frames assigned");
    const line = byTestId(tree, "drizzle-left-out");
    expect(textOf(line)).toBe("2 frames left out by IMAGETYP: 2 flats");
    expect(String(line?.props.className)).toContain("amber");
    expect(String(named(tree, "Auto")?.props["aria-describedby"])).toContain(String(line?.props.id));
  });

  it("counts the Hα frames it does not assign", () => {
    const ha = ["ha_0001.fits", "ha_0002.fits"].map((name) => frame(name, { IMAGETYP: "LIGHT", FILTER: "Ha" }));
    const view = () => hooks.render(panel, { files: [...SX13_LIGHTS, ...ha] }) as ReactNode;
    (named(view(), "Auto")?.props.onClick as () => void)();
    expect(textOf(byTestId(view(), "drizzle-unassigned"))).toBe("2 Hα frames not assigned: drizzle them in mono or assign by hand");
  });

  it("offers the 1-4x scale range of the drizzle engine", () => {
    const scale = labelled(hooks.render(panel, { files: SX13 }) as ReactNode, "Scale");
    expect([scale?.props.min, scale?.props.max]).toEqual([1, 4]);
  });
});

describe("DrizzleRgbPanel in mono", () => {
  beforeEach(() => {
    hooks.mount();
    for (const key of Object.keys(percentOf)) delete percentOf[key];
    vi.mocked(drizzleFrames).mockReset();
    vi.mocked(drizzleRgbStack).mockReset();
    vi.mocked(cancelProgress).mockReset().mockResolvedValue(true);
    vi.mocked(getOutputDir).mockReset().mockResolvedValue("C:/out");
  });

  afterEach(() => {
    for (const run of ["stack", "drizzle", "compose"] as const) releaseStackProgress(run);
    hooks.mount();
  });

  function autoAssigned(files: ProcessedFile[]): () => ReactNode {
    const view = () => hooks.render(panel, { files }) as ReactNode;
    (named(view(), "Auto")?.props.onClick as () => void)();
    return view;
  }

  it("one ready channel enables Drizzle (mono) and follows stack-progress", async () => {
    percentOf["stack-progress"] = 40;
    percentOf["drizzle-rgb-progress"] = 10;
    let fail: (error: Error) => void = () => {};
    vi.mocked(drizzleFrames).mockReturnValue(new Promise<StackResult>((_, reject) => { fail = reject; }));
    const view = autoAssigned(SX13);
    const run = runButton(view());
    expect(run?.props.label).toBe("Drizzle R (mono)");
    expect(run?.props.disabled).toBe(false);
    void (run?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(vi.mocked(drizzleFrames)).toHaveBeenCalledTimes(1));
    const [paths, outputDir, options] = vi.mocked(drizzleFrames).mock.calls[0];
    expect(paths).toEqual(SX13_LIGHTS.map((f) => f.path));
    expect(outputDir).toBe("C:/out");
    expect(options).toMatchObject({ scale: 2, pixfrac: 0.7, kernel: "square", align: true, rejection: "sigma_clip", sigmaLow: 3, sigmaHigh: 3 });
    expect(options?.name).toMatch(/^frame_0001_drizzle6_\d{8}-\d{6}-\d{3}$/);
    expect(vi.mocked(drizzleRgbStack)).not.toHaveBeenCalled();
    expect(stackProgressHolder()).toBe("drizzle");
    const running = view();
    expect(textOf(running)).toContain("40%");
    expect(textOf(running)).not.toContain("10%");
    (byTestId(running, "drizzle-rgb-cancel")?.props.onClick as () => void)();
    expect(vi.mocked(cancelProgress).mock.calls).toEqual([["stack-progress"]]);
    fail(new Error("Operation cancelled"));
    await vi.waitFor(() => expect(textOf(view())).toContain("Drizzle cancelled"));
    expect(stackProgressHolder()).toBeNull();
  });

  it("shows a mono result card with the one channel and no white balance", async () => {
    vi.mocked(drizzleFrames).mockResolvedValue(MONO_REPLY);
    const view = autoAssigned(SX13);
    void (runButton(view())?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(textOf(view())).toContain("Drizzle R (mono) Complete"));
    const items = nodes(view()).find((el) => Array.isArray(el.props.items))?.props.items as { label: string }[];
    expect(items.map((i) => i.label)).toEqual(["Output", "Scale", "Kernel", "R frames", "Rejected"]);
    (named(view(), "Open in Files")?.props.onClick as () => void)();
    expect(ingest.showWhenLoaded).toHaveBeenLastCalledWith("C:/out/frame_0001_drizzle6.fits");
  });

  it("names a mono drizzle of Ha frames put in R after their filter and hands the result to the viewer", async () => {
    const ha = ["ha_0001.fits", "ha_0002.fits", "ha_0003.fits"].map((name) => frame(name, { IMAGETYP: "LIGHT", FILTER: "Ha" }));
    const onMonoResult = vi.fn();
    vi.mocked(drizzleFrames).mockResolvedValue(MONO_REPLY);
    const view = () => hooks.render(panel, { files: ha, onMonoResult }) as ReactNode;
    for (const r of nodes(view()).filter((el) => typeof el.props.onClick === "function" && textOf(el.props.children) === "r")) (r.props.onClick as () => void)();
    const run = runButton(view());
    expect(run?.props.label).toBe("Drizzle Ha (mono)");
    void (run?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(textOf(view())).toContain("Drizzle Ha (mono) Complete"));
    const items = nodes(view()).find((el) => Array.isArray(el.props.items))?.props.items as { label: string }[];
    expect(items.map((i) => i.label)).toEqual(["Output", "Scale", "Kernel", "Ha frames", "Rejected"]);
    expect(onMonoResult.mock.calls).toEqual([[MONO_REPLY, ha.map((f) => f.path), null, "Ha"]]);
  });

  it("hides white balance, SCNR and Save FITS in mono and names the fixed alignment", () => {
    const mono = autoAssigned(SX13)();
    expect(labelled(mono, "SCNR green removal")).toBeUndefined();
    expect(labelled(mono, "Save FITS alongside PNG")).toBeUndefined();
    expect(hasText(mono, "White balance")).toBe(false);
    expect(textOf(byTestId(mono, "drizzle-mono-alignment"))).toBe("Phase Correlation, fixed in mono");
    hooks.mount();
    const rgb = autoAssigned(FRAMES)();
    expect(labelled(rgb, "SCNR green removal")).toBeDefined();
    expect(labelled(rgb, "Save FITS alongside PNG")).toBeDefined();
    expect(hasText(rgb, "White balance")).toBe(true);
    expect(byTestId(rgb, "drizzle-mono-alignment")).toBeUndefined();
  });

  it("waits while Stacking › Stack runs, and leaves an RGB drizzle free to start", () => {
    claimStackProgress("stack");
    const mono = autoAssigned(SX13)();
    const run = runButton(mono);
    const line = byTestId(mono, "drizzle-progress-busy");
    expect(run?.props.disabled).toBe(true);
    expect(textOf(line)).toBe("Stacking › Stack is running; wait until it ends, as both use the same progress bar and Cancel.");
    expect(run?.props.describedBy).toBe(line?.props.id);
    hooks.mount();
    const rgb = autoAssigned(FRAMES)();
    expect(runButton(rgb)?.props.disabled).toBe(false);
    expect(byTestId(rgb, "drizzle-progress-busy")).toBeUndefined();
    releaseStackProgress("stack");
    hooks.mount();
    expect(runButton(autoAssigned(SX13)())?.props.disabled).toBe(false);
  });

  it("waits for a mono drizzle it did not start, such as one still going when the panel opens again", () => {
    claimStackProgress("drizzle");
    const tree = autoAssigned(SX13)();
    expect(runButton(tree)?.props.disabled).toBe(true);
    expect(textOf(byTestId(tree, "drizzle-progress-busy"))).toBe("Stacking › Drizzle RGB (mono) is running; wait until it ends, as both use the same progress bar and Cancel.");
  });
});

describe("DrizzleRgbPanel rejection", () => {
  beforeEach(() => {
    hooks.mount();
    vi.mocked(drizzleFrames).mockReset();
    vi.mocked(drizzleRgbStack).mockReset();
    vi.mocked(getOutputDir).mockReset().mockResolvedValue("C:/out");
  });

  afterEach(() => {
    for (const run of ["stack", "drizzle", "compose"] as const) releaseStackProgress(run);
    hooks.mount();
  });

  const rejectionControls = (tree: ReactNode) => nodes(tree).find((el) => el.props.settings !== undefined && typeof el.props.onChange === "function");
  const change = (tree: ReactNode, update: Record<string, unknown>) => (rejectionControls(tree)?.props.onChange as (c: Record<string, unknown>) => void)(update);
  const FIXED_KEYS = ["maxIterations", "winsorCutoff", "percentileLow", "percentileHigh", "minmaxLow", "minmaxHigh"];

  it("shows the parameters the drizzle engine fixes read-only and sends only the method and sigmas it takes", async () => {
    vi.mocked(drizzleRgbStack).mockReturnValue(new Promise<DrizzleRgbResult>(() => {}));
    const view = () => hooks.render(panel, { files: FRAMES }) as ReactNode;
    expect(rejectionControls(view())?.props.frameCount).toBeNull();
    (named(view(), "Auto")?.props.onClick as () => void)();
    expect(rejectionControls(view())?.props).toMatchObject({
      label: "Pixel rejection",
      fixed: DRIZZLE_FIXED_REJECTION,
      frameCount: 2,
      disabled: false,
      settings: { rejection: "sigma_clip", sigmaLow: 3, sigmaHigh: 3 },
    });
    change(view(), { rejection: "winsorized_sigma_clip" });
    change(view(), { sigmaHigh: 2.5 });
    void (runButton(view())?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(vi.mocked(drizzleRgbStack)).toHaveBeenCalledTimes(1));
    expect(rejectionControls(view())?.props.disabled).toBe(true);
    const options = vi.mocked(drizzleRgbStack).mock.calls[0][4];
    expect(options).toMatchObject({ rejection: "winsorized_sigma_clip", sigmaLow: 3, sigmaHigh: 2.5 });
    for (const key of FIXED_KEYS) expect(Object.keys(options ?? {})).not.toContain(key);
  });

  it("sends a mono drizzle the method it shows and no sigma for a method without one", async () => {
    vi.mocked(drizzleFrames).mockResolvedValue(MONO_REPLY);
    const view = () => hooks.render(panel, { files: SX13 }) as ReactNode;
    (named(view(), "Auto")?.props.onClick as () => void)();
    expect(rejectionControls(view())?.props.frameCount).toBe(6);
    change(view(), { rejection: "min_max" });
    void (runButton(view())?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(vi.mocked(drizzleFrames)).toHaveBeenCalledTimes(1));
    const options = vi.mocked(drizzleFrames).mock.calls[0][2];
    expect(options).toMatchObject({ rejection: "min_max", sigmaLow: undefined, sigmaHigh: undefined });
    for (const key of FIXED_KEYS) expect(Object.keys(options ?? {})).not.toContain(key);
  });
});
