import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ProcessedFile } from "../../../shared/types";
import type { StackResult } from "../../../shared/types/stacking";

const ingest = vi.hoisted(() => ({ showWhenLoaded: vi.fn(), ingestUnlessLoaded: vi.fn() }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../hooks/useProgress", () => ({
  useProgress: () => ({ active: false, percent: 0, stage: "", current: 0, total: 0, reset: () => {} }),
}));
vi.mock("../../../services/stacking", () => ({ noiseWeightsFor: vi.fn(), stackFrames: vi.fn() }));
vi.mock("../../../services/progress", () => ({ cancelProgress: vi.fn() }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));
vi.mock("../../../hooks/useFileIngest", () => ingest);

import { hooks } from "../../../test/hookRuntime";
import { stackFrames } from "../../../services/stacking";
import StackingPanel, { NoiseWeightHintLine } from "../StackingPanel";
import { correctedCopySwap, type CosmeticCopy } from "../../../utils/cosmeticCopies";
import { claimStackProgress, releaseStackProgress, stackProgressHolder } from "../../../utils/stackProgressLock";
import { DEFAULT_STACK_SETTINGS, type StackSettings } from "../../../utils/stackingRejection";

function processed(name: string): ProcessedFile {
  return {
    id: name,
    name,
    path: `C:/wfpc2/${name}`,
    sourcePath: `C:/wfpc2/${name}`,
    imageRef: null,
    size: 0,
    status: "done",
    result: null,
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

const FILES = ["502nmos.fits", "656nmos.fits", "673nmos.fits"].map(processed);

function frameRowTags(html: string): string[] {
  return html.match(/<button[^>]*data-testid="stack-frame-row"[^>]*>/g) ?? [];
}

describe("StackingPanel frame list", () => {
  it("tags every frame row and the All/None buttons", () => {
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: FILES }));
    expect(frameRowTags(html)).toHaveLength(3);
    expect(html).toMatch(/<button[^>]*data-testid="stack-select-all"[^>]*>All<\/button>/);
    expect(html).toMatch(/<button[^>]*data-testid="stack-select-none"[^>]*>None<\/button>/);
  });

  it("tags the calibrated rows injected from the Calibration tab as frame rows too", () => {
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: FILES, injectedPaths: ["C:/out/light_calibrated.fits"] }));
    expect(frameRowTags(html)).toHaveLength(4);
  });

  it("leaves the rows and All/None enabled while no stack runs", () => {
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: FILES }));
    for (const tag of frameRowTags(html)) expect(tag).not.toContain('disabled=""');
    expect(html).not.toMatch(/<button[^>]*data-testid="stack-select-(all|none)"[^>]*disabled=""/);
  });
});

describe("StackingPanel rejection", () => {
  type AnyProps = Record<string, unknown> & { children?: ReactNode };

  function nodes(node: ReactNode): ReactElement<AnyProps>[] {
    if (Array.isArray(node)) return node.flatMap(nodes);
    if (!isValidElement<AnyProps>(node)) return [];
    return [node, ...nodes(node.props.children)];
  }

  beforeEach(() => hooks.mount());
  afterEach(() => hooks.mount());

  const MIN_MAX: StackSettings = { ...DEFAULT_STACK_SETTINGS, rejection: "min_max", minmaxLow: 2, minmaxHigh: 2 };

  it("shows the rejection controls of its method with the counts in the hint", () => {
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: FILES, stackConfig: MIN_MAX }));
    expect(html).toContain("Reject Low");
    expect(html).toContain("Reject High");
    expect(html).not.toContain("Sigma Low");
    expect(html).toMatch(/data-testid="rejection-hint"[^>]*>Min\/max needs more than 4 frames to drop 2 low and 2 high \(have 0\)\.<\/p>/);
  });

  it("hands the Stack tab settings and selection to the controls and their changes back to the tab", () => {
    const onStackConfigChange = vi.fn();
    const tree = hooks.render(StackingPanel as unknown as (props: Record<string, unknown>) => ReactNode, { files: FILES, stackConfig: MIN_MAX, onStackConfigChange }) as ReactNode;
    const controls = nodes(tree).find((el) => el.props.settings !== undefined && typeof el.props.onChange === "function");
    expect(controls?.props).toMatchObject({ settings: MIN_MAX, frameCount: 0, accent: "amber" });
    (controls?.props.onChange as (change: Record<string, unknown>) => void)({ minmaxLow: 3 });
    expect(onStackConfigChange.mock.calls).toEqual([[{ minmaxLow: 3 }]]);
  });
});

describe("StackingPanel frame types", () => {
  function typed(name: string, imagetyp: string | null): ProcessedFile {
    const header: Record<string, string> = imagetyp ? { IMAGETYP: imagetyp, FILTER: "R" } : { FILTER: "R" };
    return { ...processed(name), result: { header } as ProcessedFile["result"] };
  }

  const SESSION = [
    typed("light_1.fits", "LIGHT"),
    typed("flat_1.fits", "FLAT"),
    typed("dark_1.fits", "DARK"),
    typed("bias_1.fits", "BIAS"),
    typed("flatdark_1.fits", "FLAT DARK"),
    typed("frame_ext.fits", "EXT"),
    typed("no_card.fits", null),
  ];

  function chips(html: string): string[] {
    return [...html.matchAll(/<span[^>]*data-testid="stack-frame-kind"[^>]*><span aria-hidden="true">([^<]*)<\/span><span class="sr-only">([^<]*)<\/span><\/span>/g)].map((m) => `${m[1]}=${m[2]}`);
  }

  it("puts a type chip on every row whose IMAGETYP is known", () => {
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: SESSION }));
    expect(frameRowTags(html)).toHaveLength(7);
    expect(chips(html)).toEqual(["L=light frame", "F=flat frame", "D=dark frame", "B=bias frame", "FD=flat dark frame"]);
  });

  it("names the frames that All leaves out by IMAGETYP and describes the All button with it", () => {
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: SESSION }));
    const line = /<p id="([^"]+)"[^>]*data-testid="stack-left-out"[^>]*>([^<]*)<\/p>/.exec(html);
    expect(line?.[2]).toBe("4 frames left out by IMAGETYP: 1 dark, 1 flat, 1 flat dark, 1 bias");
    expect(html).toMatch(new RegExp(`<button[^>]*data-testid="stack-select-all"[^>]*aria-describedby="${line?.[1]}"`));
  });

  it("says nothing about IMAGETYP when every loaded frame is a light", () => {
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: FILES }));
    expect(html).not.toContain("stack-left-out");
    expect(html).not.toContain("stack-frame-kind");
  });
});

describe("StackingPanel cosmetic copies", () => {
  function light(name: string): ProcessedFile {
    return { ...processed(name), path: `C:/sx11/${name}`, sourcePath: `C:/sx11/${name}` };
  }

  const LIGHTS = [1, 2, 3, 4, 5].map((n) => light(`frame_000${n}_3.fits`));
  const COPIES: CosmeticCopy[] = LIGHTS.map((f) => ({
    original: f.path,
    corrected: `C:\\run\\output/${f.name.replace(".fits", "_cosmetic.fits")}`,
  }));

  function rowLabels(html: string): string[] {
    return [...html.matchAll(/<button[^>]*data-testid="stack-frame-row"[^>]*>[\s\S]*?<span class="truncate">([^<]*)<\/span>/g)].map((m) => m[1]);
  }

  function groupHeaders(html: string): string[] {
    return [...html.matchAll(/<div[^>]*data-testid="stack-frame-group"[^>]*>(?:<svg[\s\S]*?<\/svg>)?([^<]*)<\/div>/g)].map((m) => m[1]);
  }

  function useCorrectedButton(html: string): string | null {
    return /<button[^>]*data-testid="stack-use-corrected"[^>]*>[^<]*<\/button>/.exec(html)?.[0] ?? null;
  }

  it("cosmetic batch outputs appear under From Cosmetic and the swap replaces the originals", () => {
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: LIGHTS, cosmeticCopies: COPIES }));
    expect(groupHeaders(html)).toEqual(["From Cosmetic"]);
    expect(rowLabels(html).filter((label) => label.endsWith("_cosmetic.fits"))).toEqual(COPIES.map((c) => c.corrected.split("/").pop()));
    expect(useCorrectedButton(html)).toMatch(/>Use corrected copies \(5\)<\/button>$/);
    expect(useCorrectedButton(html)).not.toContain('disabled=""');
    const swap = correctedCopySwap(LIGHTS.map((f) => f.path), COPIES, []);
    expect(swap).toEqual({ count: 5, paths: COPIES.map((c) => c.corrected) });
  });

  it("lists a copy that is also open in Files once, as its file row", () => {
    const opened = { ...light("frame_0001_3_cosmetic.fits"), path: COPIES[0].corrected.replace(/\//g, "\\") };
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: [...LIGHTS, opened], cosmeticCopies: COPIES }));
    expect(rowLabels(html).filter((label) => label === "frame_0001_3_cosmetic.fits")).toHaveLength(1);
    expect(rowLabels(html).filter((label) => label.endsWith("_cosmetic.fits"))).toHaveLength(5);
  });

  it("shows the calibrated and the cosmetic groups apart", () => {
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: LIGHTS, cosmeticCopies: COPIES, injectedPaths: ["C:/out/light_calibrated.fits"] }));
    expect(groupHeaders(html)).toEqual(["From Calibration", "From Cosmetic"]);
  });

  it("offers no swap without cosmetic copies", () => {
    const html = renderToStaticMarkup(createElement(StackingPanel, { files: LIGHTS }));
    expect(groupHeaders(html)).toEqual([]);
    expect(useCorrectedButton(html)).toBeNull();
  });
});

describe("NoiseWeightHintLine", () => {
  it("shows the idle sentence under the hint test id", () => {
    const html = renderToStaticMarkup(createElement(NoiseWeightHintLine, { effective: null }));
    expect(html).toMatch(/<p[^>]*data-testid="stack-noise-hint"[^>]*>Noise weights are measured when you stack\.<\/p>/);
  });

  it("shows the effective weights the backend reported", () => {
    const effective = { range: "0.60 - 1.80", frames: 3, subframeCount: 0, noise: { missing: 0, total: 3 } };
    const html = renderToStaticMarkup(createElement(NoiseWeightHintLine, { effective }));
    expect(html).toContain(
      ">Effective frame weights 0.60 - 1.80 for the 3 frames stacked (noise weights divided by the normalization scale², normalised to mean 1).</p>",
    );
  });

  it("describes the subframe weights and noise coverage the stack used", () => {
    const effective = { range: "0.60 - 1.80", frames: 3, subframeCount: 2, noise: { missing: 1, total: 3 } };
    const html = renderToStaticMarkup(createElement(NoiseWeightHintLine, { effective }));
    expect(html).toContain(
      "normalised to mean 1), multiplied by the subframe weights. 1 of the 3 frames had no noise estimate and kept the mean noise weight.</p>",
    );
  });
});

describe("StackingPanel Open in Files", () => {
  type AnyProps = Record<string, unknown> & { children?: ReactNode };
  const panel = StackingPanel as unknown as (props: { files: ProcessedFile[] }) => ReactNode;

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

  beforeEach(() => {
    hooks.mount();
    ingest.showWhenLoaded.mockReset();
    vi.mocked(stackFrames).mockReset();
  });

  afterEach(() => hooks.mount());

  async function stacked(reply: Partial<StackResult>): Promise<ReactNode> {
    vi.mocked(stackFrames).mockResolvedValue({ png_path: "C:/out/stack.png", dimensions: [8, 8], elapsed_ms: 5, frame_count: 3, ...reply });
    const view = () => hooks.render(panel, { files: FILES }) as ReactNode;
    (nodes(view()).find((el) => el.props["data-testid"] === "stack-select-all")?.props.onClick as () => void)();
    void (nodes(view()).find((el) => el.props.runningLabel === "Stacking...")?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(textOf(view())).toContain("Stacking Complete"));
    return view();
  }

  it("adds the stacked FITS to Files and shows it", async () => {
    const tree = await stacked({ fits_path: "C:/out/502nmos_stack3_20261030-101500-000.fits" });
    (named(tree, "Open in Files")?.props.onClick as () => void)();
    expect(ingest.showWhenLoaded.mock.calls).toEqual([["C:/out/502nmos_stack3_20261030-101500-000.fits"]]);
  });

  it("offers no Open in Files when the stack wrote no FITS", async () => {
    expect(named(await stacked({}), "Open in Files")).toBeUndefined();
  });
});

describe("StackingPanel stacked results", () => {
  type AnyProps = Record<string, unknown> & { children?: ReactNode };
  const panel = StackingPanel as unknown as (props: { files: ProcessedFile[] }) => ReactNode;

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

  function sx(name: string, header: Record<string, string>): ProcessedFile {
    return { ...processed(name), path: `C:/sx04/${name}`, sourcePath: `C:/sx04/${name}`, result: { header } as ProcessedFile["result"] };
  }

  const LIGHTS = [1, 2, 3].map((n) => sx(`light_000${n}.fits`, { IMAGETYP: "LIGHT", FILTER: "R" }));
  const STACK = sx("light_0001_stack3_20261030-101500-000.fits", { IMAGETYP: "LIGHT", FILTER: "R", ABCOMB: "mean", NCOMBINE: "3", ABPROC: "stacked" });
  const REJECTION_MAP = sx("light_0001_stack3_20261030-101500-000_rejlow.fits", { IMAGETYP: "LIGHT", ABPROC: "rejection" });
  const FLAT = sx("flat_0001.fits", { IMAGETYP: "FLAT", FILTER: "R" });

  const byTestId = (tree: ReactNode, id: string) => nodes(tree).find((el) => el.props["data-testid"] === id);
  const runLabel = (tree: ReactNode) => nodes(tree).find((el) => el.props.runningLabel === "Stacking...")?.props.label;
  const row = (tree: ReactNode, file: ProcessedFile) => nodes(tree).find((el) => el.props["data-testid"] === "stack-frame-row" && el.key === file.id);

  beforeEach(() => hooks.mount());
  afterEach(() => hooks.mount());

  it("All leaves the stacked results out, says why next to the IMAGETYP line and tags their rows", () => {
    const view = () => hooks.render(panel, { files: [...LIGHTS, STACK, REJECTION_MAP, FLAT] }) as ReactNode;
    (byTestId(view(), "stack-select-all")?.props.onClick as () => void)();
    const tree = view();
    expect(runLabel(tree)).toBe("Stack 3 Frames");
    const line = byTestId(tree, "stack-stacked-left-out");
    expect(textOf(line)).toBe("2 stacked results left out of All");
    const describedBy = String(byTestId(tree, "stack-select-all")?.props["aria-describedby"]).split(" ");
    expect(describedBy).toEqual([String(byTestId(tree, "stack-left-out")?.props.id), String(line?.props.id)]);
    expect(textOf(row(tree, STACK))).toContain("stacked");
    expect(textOf(row(tree, REJECTION_MAP))).toContain("stacked");
    expect(textOf(row(tree, LIGHTS[0]))).not.toContain("stacked");
  });

  it("still stacks a stacked result picked by hand", () => {
    const view = () => hooks.render(panel, { files: [...LIGHTS, STACK] }) as ReactNode;
    (byTestId(view(), "stack-select-all")?.props.onClick as () => void)();
    (row(view(), STACK)?.props.onClick as () => void)();
    expect(runLabel(view())).toBe("Stack 4 Frames");
  });

  it("says nothing about stacked results when none is loaded", () => {
    expect(byTestId(hooks.render(panel, { files: LIGHTS }) as ReactNode, "stack-stacked-left-out")).toBeUndefined();
  });
});

describe("StackingPanel and the shared stack-progress", () => {
  type AnyProps = Record<string, unknown> & { children?: ReactNode };
  const panel = StackingPanel as unknown as (props: { files: ProcessedFile[] }) => ReactNode;

  function nodes(node: ReactNode): ReactElement<AnyProps>[] {
    if (Array.isArray(node)) return node.flatMap(nodes);
    if (!isValidElement<AnyProps>(node)) return [];
    return [node, ...nodes(node.props.children)];
  }

  const byTestId = (tree: ReactNode, id: string) => nodes(tree).find((el) => el.props["data-testid"] === id);
  const runButton = (tree: ReactNode) => nodes(tree).find((el) => el.props.runningLabel === "Stacking...");

  function allSelected(): () => ReactNode {
    const view = () => hooks.render(panel, { files: FILES }) as ReactNode;
    (byTestId(view(), "stack-select-all")?.props.onClick as () => void)();
    return view;
  }

  beforeEach(() => {
    hooks.mount();
    vi.mocked(stackFrames).mockReset();
  });

  afterEach(() => {
    for (const run of ["stack", "drizzle", "compose"] as const) releaseStackProgress(run);
    hooks.mount();
  });

  it("waits while a mono drizzle runs, since both report on stack-progress", () => {
    claimStackProgress("drizzle");
    const view = allSelected();
    const tree = view();
    const line = byTestId(tree, "stack-progress-busy");
    expect(runButton(tree)?.props.disabled).toBe(true);
    expect(line?.props.children).toBe("Stacking › Drizzle RGB (mono) is running; wait until it ends, as both use the same progress bar and Cancel.");
    expect(runButton(tree)?.props.describedBy).toBe(line?.props.id);
    releaseStackProgress("drizzle");
    expect(runButton(view())?.props.disabled).toBe(false);
    expect(byTestId(view(), "stack-progress-busy")).toBeUndefined();
  });

  it("waits for a Stack run it did not start, such as one still going when the panel opens again", () => {
    claimStackProgress("stack");
    const tree = allSelected()();
    expect(runButton(tree)?.props.disabled).toBe(true);
    expect(byTestId(tree, "stack-progress-busy")?.props.children).toBe("Stacking › Stack is running; wait until it ends, as both use the same progress bar and Cancel.");
  });

  it("holds stack-progress while it stacks and lets it go when the run ends", async () => {
    let finish: (result: StackResult) => void = () => {};
    vi.mocked(stackFrames).mockReturnValue(new Promise<StackResult>((resolve) => { finish = resolve; }));
    const view = allSelected();
    void (runButton(view())?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(vi.mocked(stackFrames)).toHaveBeenCalledTimes(1));
    expect(stackProgressHolder()).toBe("stack");
    finish({ png_path: "C:/out/stack.png", dimensions: [8, 8], elapsed_ms: 5, frame_count: 3 });
    await vi.waitFor(() => expect(stackProgressHolder()).toBeNull());
  });
});
