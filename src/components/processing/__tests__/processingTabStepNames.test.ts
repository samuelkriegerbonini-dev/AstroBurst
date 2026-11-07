import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { renderToStaticMarkup } from "react-dom/server";

interface PanelProps {
  chainedFrom?: string | null;
  inputLabel?: string;
  inputPreviewUrl?: string | null;
}

const { captured } = vi.hoisted(() => ({ captured: {} as Record<string, PanelProps | undefined> }));

function capture(name: string) {
  return {
    default: (props: PanelProps) => {
      captured[name] = props;
      return null;
    },
  };
}

vi.mock("../BackgroundPanel", () => capture("background"));
vi.mock("../WaveletPanel", () => capture("denoise"));
vi.mock("../LocalContrastPanel", () => capture("lhe"));
vi.mock("../HdrPanel", () => capture("hdr"));
vi.mock("../CurvesPanel", () => capture("curves"));
vi.mock("../PixelMathPanel", () => capture("pixelMath"));

import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { nodeRecord, opNameLabel, projectChain } from "../../../utils/historyModel";
import ChainBanner from "../../ui/ChainBanner";
import { EMPTY_HISTORY_SNAPSHOT, historyStore } from "../../../utils/historyStore";
import { historyRerunStore } from "../../../utils/historyRerunStore";
import ProcessingTab from "../ProcessingTab";
import type { ProcessedFile } from "../../../shared/types";
import type { HistoryDoc, HistoryNode, HistoryState, OpName, OpSpec } from "../../../shared/types/ops";

const RAW = "C:/data/673nmos.fits";
const DIR = "C:/app/history/7a1c3e5b9d2f4086";
const DBE: OpSpec = {
  op: "background",
  v: 1,
  model: "spline",
  config: { sampleRadius: 5, tolerance: 1, smoothing: 0.5, autoGrid: 8, manualSamples: [], rejectStars: true, mode: "subtract", normalize: true, maxSamples: 400 },
};
const DENOISE: OpSpec = { op: "wavelet_denoise", v: 1, numScales: 5, thresholds: [3, 2.5, 2, 1.5, 1], linearDenoise: true, layerBias: null };
const GHS: OpSpec = { op: "ghs", v: 1, d: 3, b: 0, sp: 0.05, lp: 0, hp: 1 };
const HDRMT: OpSpec = { op: "hdrmt", v: 1, layers: 6, iterations: 1, overdrive: 0, inverted: false, toLightness: true, deringing: false, deringingAmount: 0.5 };
const PIXELMATH: OpSpec = { op: "pixelmath", v: 1, expression: "$T * 0.5", slots: [], truncate: true, rescale: false };

function node(id: string, parent: string | null, op: OpSpec): HistoryNode {
  return {
    id,
    parent,
    op,
    mask: null,
    output: { path: `${id}.fits`, png: `${id}.png`, extras: [], dims: [8, 8], planes: 1, domain: "linear" },
    at: "2026-10-30T12:00:00Z",
    elapsed_ms: 1,
    app: "0.8.0",
    threads: 8,
    present: true,
    bytes: 256,
    checkpoint: false,
  };
}

const DOC: HistoryDoc = {
  format: "astroburst-history",
  version: 1,
  image: { id: "7a1c3e5b9d2f4086", ref: RAW, size: 1, mtime: null },
  image_key: RAW,
  base_path: null,
  head: "n0005",
  policy: { k: 4 },
  app: "0.8.0",
  threads: 8,
  masks: {},
  nodes: [node("n0001", null, DBE), node("n0002", "n0001", DENOISE), node("n0003", "n0002", GHS), node("n0004", "n0003", HDRMT), node("n0005", "n0004", PIXELMATH)],
};

const STATE: HistoryState = { image_id: "7a1c3e5b9d2f4086", history_dir: DIR, root_path: RAW, doc: DOC, read_only: null, fingerprint_matches: true, busy: false };

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "673nmos.fits",
    path: RAW,
    sourcePath: RAW,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/673nmos.png", previewUrl: "asset://673nmos.png", dimensions: [8, 8], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function showHistoryHead(file: ProcessedFile, head = "n0005", shown = head): void {
  const key = fileKeyOf(file) as string;
  const record = nodeRecord(DOC, shown, { dir: DIR, rootPath: RAW, rootPreviewUrl: null });
  if (!record) throw new Error(`the fixture has no ${shown}`);
  function Publisher() {
    useRenderActions().publishProcessed(key, record as NonNullable<typeof record>, () => projectChain(DOC, head, { dir: DIR, base: null }));
    return null;
  }
  renderToStaticMarkup(createElement(CompositeProvider, { children: createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) }) }));
  historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: key, scope: { imageKey: RAW, basePath: null }, state: STATE });
}

async function renderTab(file: ProcessedFile): Promise<string> {
  const tree = createElement(CompositeProvider, {
    children: createElement(ComposeWizardProvider, {
      children: createElement(PreviewProvider, { file, doneFiles: [file], children: createElement(ProcessingTab) }),
    }),
  });
  const { prelude } = await prerender(tree);
  return new Response(prelude).text();
}

function panel(name: string): PanelProps {
  const props = captured[name];
  if (!props) throw new Error(`ProcessingTab did not render the ${name} panel`);
  return props;
}

function crumbLabels(html: string): string[] {
  const bar = /<div class="flex items-center gap-1 px-4 py-1\.5[^"]*">([\s\S]*?)<\/div>/.exec(html)?.[1] ?? "";
  return [...bar.matchAll(/<(?:span|button)[^>]*class="(?:text-emerald-400\/80|text-zinc-500)[^"]*"[^>]*>([^<]*)<\/(?:span|button)>/g)].map((m) => m[1]);
}

const READ_ONLY: HistoryState = { ...STATE, read_only: "this history was written with history format version 2" };

const EVERY_OP = Object.keys({
  background: true,
  wavelet_denoise: true,
  rl_deconv: true,
  arcsinh: true,
  ghs: true,
  masked_stretch: true,
  lhe: true,
  hdrmt: true,
  tone: true,
  pixelmath: true,
} satisfies Record<OpName, true>) as OpName[];

function bannerText(chainedFrom: string): string {
  return renderToStaticMarkup(createElement(ChainBanner, { chainedFrom })).replace(/<[^>]*>/g, "");
}

beforeEach(() => {
  for (const name of Object.keys(captured)) delete captured[name];
  historyStore.set(EMPTY_HISTORY_SNAPSHOT);
  historyRerunStore.clear();
});

describe("ProcessingTab names each step by the tool that made it", () => {
  it("gives every crumb the tool of its history step", async () => {
    const file = fileOf("names-crumbs");
    showHistoryHead(file);
    const html = await renderTab(file);
    expect(html).toMatch(/<button[^>]*data-history-crumb="n0001"[^>]*>DBE<\/button>/);
    expect(html).toMatch(/<button[^>]*data-history-crumb="n0002"[^>]*>Denoise<\/button>/);
    expect(html).toMatch(/<button[^>]*data-history-crumb="n0003"[^>]*>GHS<\/button>/);
    expect(html).toMatch(/<button[^>]*data-history-crumb="n0004"[^>]*title="Go to HDRMT \(n0004\)"[^>]*>HDRMT<\/button>/);
    expect(html).not.toContain("LHE/HDRMT");
  });

  it("labels each panel's compare base and chain banner with the tool that made its input", async () => {
    const file = fileOf("names-panels");
    showHistoryHead(file);
    await renderTab(file);
    expect(panel("curves")).toMatchObject({ inputLabel: "HDRMT", chainedFrom: "HDRMT" });
    expect(panel("hdr")).toMatchObject({ inputLabel: "GHS", chainedFrom: "GHS Stretch" });
    expect(panel("lhe")).toMatchObject({ inputLabel: "GHS", chainedFrom: "GHS Stretch" });
    expect(panel("denoise")).toMatchObject({ inputLabel: "DBE", chainedFrom: "DBE" });
    expect(panel("background")).toMatchObject({ inputLabel: "Original" });
    expect(panel("pixelMath").inputLabel).toBe("HDRMT");
  });

  it("names the latest step in the PixelMath notice by its tool, as written in History", async () => {
    const afterHdrmt = fileOf("names-pixelmath-hdrmt");
    showHistoryHead(afterHdrmt, "n0004", "n0003");
    await renderTab(afterHdrmt);
    expect(panel("pixelMath").chainedFrom).toBe("HDRMT");
    const afterGhs = fileOf("names-pixelmath-ghs");
    showHistoryHead(afterGhs, "n0003", "n0002");
    await renderTab(afterGhs);
    expect(panel("pixelMath").chainedFrom).toBe("GHS Stretch");
  });

  it("keeps the tool names while History is not loaded or is read-only", async () => {
    for (const state of [null, READ_ONLY]) {
      const file = fileOf(`names-without-history-${state ? "read-only" : "none"}`);
      showHistoryHead(file, "n0004", "n0003");
      historyStore.set(state ? { ...EMPTY_HISTORY_SNAPSHOT, fileKey: fileKeyOf(file), scope: { imageKey: RAW, basePath: null }, state } : EMPTY_HISTORY_SNAPSHOT);
      const html = await renderTab(file);
      expect(crumbLabels(html)).toEqual(["673nmos", "DBE", "Denoise", "GHS", "HDRMT"]);
      expect(html).not.toContain("data-history-crumb");
      expect(panel("curves")).toMatchObject({ inputLabel: "HDRMT", chainedFrom: "HDRMT" });
      expect(panel("hdr")).toMatchObject({ inputLabel: "GHS", chainedFrom: "GHS Stretch" });
      expect(panel("denoise")).toMatchObject({ inputLabel: "DBE", chainedFrom: "DBE" });
      expect(panel("pixelMath").chainedFrom).toBe("HDRMT");
    }
  });
});

describe("ChainBanner names the input as History does", () => {
  it("shows the label History gives each tool, unchanged", () => {
    for (const op of EVERY_OP) expect(bannerText(opNameLabel(op))).toBe(`Using output from ${opNameLabel(op)}`);
  });

  it("shows a base image label as given, even when it reads like a step id", () => {
    for (const label of ["background", "denoise", "deconvolution", "deconv"]) {
      expect(bannerText(label)).toBe(`Using output from ${label}`);
    }
  });
});
