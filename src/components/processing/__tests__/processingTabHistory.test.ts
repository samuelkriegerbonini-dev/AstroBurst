import { beforeEach, describe, expect, it } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { projectChain, nodeRecord } from "../../../utils/historyModel";
import { EMPTY_HISTORY_SNAPSHOT, historyStore } from "../../../utils/historyStore";
import { historyRerunStore } from "../../../utils/historyRerunStore";
import ProcessingTab from "../ProcessingTab";
import type { ProcessedFile } from "../../../shared/types";
import type { HistoryDoc, HistoryNode, HistoryState, OpSpec } from "../../../shared/types/ops";

const RAW = "C:/data/nrca1_i2d.fits";
const DIR = "C:/app/history/3f9c0a1e5b7d2c44";
const BACKGROUND: OpSpec = { op: "background", v: 1, model: "polynomial", config: { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" } };
const DENOISE: OpSpec = { op: "wavelet_denoise", v: 1, numScales: 5, thresholds: [3, 2.5, 2, 1.5, 1], linearDenoise: true, layerBias: null };
const ARCSINH: OpSpec = { op: "arcsinh", v: 1, factor: 50 };

function node(id: string, parent: string | null, op: OpSpec, present = true): HistoryNode {
  return {
    id,
    parent,
    op,
    mask: null,
    output: { path: `${id}.fits`, png: `${id}.png`, extras: [], dims: [8, 8], planes: 1, domain: "linear" },
    at: "2026-10-16T12:00:00Z",
    elapsed_ms: 1,
    app: "0.6.9",
    threads: 8,
    present,
    bytes: 256,
    checkpoint: false,
  };
}

const DOC: HistoryDoc = {
  format: "astroburst-history",
  version: 1,
  image: { id: "3f9c0a1e5b7d2c44", ref: RAW, size: 1, mtime: null },
  image_key: RAW,
  base_path: null,
  head: "n0003",
  policy: { k: 4 },
  app: "0.6.9",
  threads: 8,
  masks: {},
  nodes: [node("n0001", null, BACKGROUND), node("n0002", "n0001", DENOISE, false), node("n0003", "n0002", ARCSINH)],
};

const STATE: HistoryState = { image_id: "3f9c0a1e5b7d2c44", history_dir: DIR, root_path: RAW, doc: DOC, read_only: null, fingerprint_matches: true, busy: false };

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "nrca1_i2d.fits",
    path: RAW,
    sourcePath: RAW,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/a.png", previewUrl: "asset://a.png", dimensions: [8, 8], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function showHistoryHead(file: ProcessedFile): void {
  const key = fileKeyOf(file) as string;
  const record = nodeRecord(DOC, "n0003", { dir: DIR, rootPath: RAW, rootPreviewUrl: null });
  if (!record) throw new Error("the fixture has no n0003");
  function Publisher() {
    useRenderActions().publishProcessed(key, record as NonNullable<typeof record>, () => projectChain(DOC, "n0003", { dir: DIR, base: null }));
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
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

function pillHasDot(html: string, label: string): boolean {
  const match = new RegExp(`title="${label} processing step">${label}(<span class="ab-processing-pill-dot[^"]*"></span>)?</button>`).exec(html);
  if (!match) throw new Error(`no ${label} pill`);
  return match[1] !== undefined;
}

beforeEach(() => {
  historyStore.set(EMPTY_HISTORY_SNAPSHOT);
  historyRerunStore.clear();
});

describe("ProcessingTab on a recorded history", () => {
  it("turns the chain crumbs into buttons that go to their history step", async () => {
    const file = fileOf("crumbs");
    showHistoryHead(file);
    const html = await renderTab(file);
    expect(html).toMatch(/<button[^>]*data-history-crumb="root"[^>]*>nrca1_i2d<\/button>/);
    expect(html).toMatch(/<button[^>]*data-history-crumb="n0001"[^>]*>BG<\/button>/);
    expect(html).toMatch(/<button[^>]*data-history-crumb="n0002"[^>]*>Denoise<\/button>/);
    expect(html).toMatch(/<button[^>]*data-history-crumb="n0003"[^>]*>Stretch<\/button>/);
  });

  it("shows the result dot only for steps whose node is still on disk", async () => {
    const file = fileOf("dots");
    showHistoryHead(file);
    const html = await renderTab(file);
    expect(pillHasDot(html, "Background")).toBe(true);
    expect(pillHasDot(html, "Denoise")).toBe(false);
    expect(pillHasDot(html, "Stretch")).toBe(true);
    expect(pillHasDot(html, "Curves")).toBe(false);
  });

  it("opens the section a re-run asks for and names an armed downstream re-run", async () => {
    const file = fileOf("rerun");
    showHistoryHead(file);
    const pending = historyRerunStore.request({ section: "denoise", op: DENOISE, node: "n0002", downstream: true, scope: { imageKey: RAW, basePath: null } });
    const html = await renderTab(file);
    expect(html).toMatch(/class="ab-processing-pill bg-sky-600\/20[^"]*" title="Denoise processing step"/);
    expect(html).toContain("data-history-rerun");
    expect(html.replace(/<!-- -->/g, "")).toContain("The next Wavelet Denoise run re-runs n0002 and re-applies the steps after it on a new branch.");
    historyRerunStore.consume(pending.seq);
    historyRerunStore.disarm();
    expect(await renderTab(file)).not.toContain("data-history-rerun");
  });

  it("keeps plain crumbs when the file has no history", async () => {
    const file = fileOf("plain");
    const key = fileKeyOf(file) as string;
    function Publisher() {
      useRenderActions().publishProcessed(
        key,
        { fitsPath: "C:/out/x_bg.fits", previewUrl: null, dimensions: [8, 8], label: "Background", kind: "processing", inputPath: RAW },
        (c) => ({ ...c, steps: { background: { fitsPath: "C:/out/x_bg.fits", previewUrl: null, dimensions: [8, 8] } } }),
      );
      return null;
    }
    renderToStaticMarkup(createElement(CompositeProvider, { children: createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) }) }));
    const html = await renderTab(file);
    expect(html).not.toContain("data-history-crumb");
    expect(html).toMatch(/<span class="text-emerald-400\/80">BG<\/span>/);
  });
});
