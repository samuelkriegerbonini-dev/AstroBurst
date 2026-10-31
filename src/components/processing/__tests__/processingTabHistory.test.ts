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

function pillOf(html: string, section: string): string {
  const match = new RegExp(`<button[^>]*data-processing-section="${section}"[^>]*>[\\s\\S]*?</button>`).exec(html);
  if (!match) throw new Error(`no ${section} pill`);
  return match[0];
}

function pillHasDot(html: string, section: string): boolean {
  return /<span class="ab-processing-pill-dot[^"]*" title="Has a result in this chain"><\/span>/.test(pillOf(html, section));
}

function replacedNote(html: string): { id: string; text: string } | null {
  const match = /<div id="([^"]+)" role="note" data-replaced-steps=""[^>]*>([\s\S]*?)<\/div>/.exec(html);
  return match ? { id: match[1], text: match[2].replace(/<!-- -->/g, "") } : null;
}

function runDescribedBy(html: string, label: string): string[] {
  for (const match of html.matchAll(/<button([^>]*class="ab-run-btn[^"]*"[^>]*)>([\s\S]*?)<\/button>/g)) {
    if (match[2].replace(/<[^>]*>/g, "").trim().startsWith(label)) return (/aria-describedby="([^"]*)"/.exec(match[1])?.[1] ?? "").split(" ");
  }
  throw new Error(`no "${label}" run button`);
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
    expect(pillHasDot(html, "background")).toBe(true);
    expect(pillHasDot(html, "denoise")).toBe(false);
    expect(pillHasDot(html, "stretch")).toBe(true);
    expect(pillHasDot(html, "curves")).toBe(false);
  });

  it("opens the section a re-run asks for and names an armed downstream re-run", async () => {
    const file = fileOf("rerun");
    showHistoryHead(file);
    const pending = historyRerunStore.request({ section: "denoise", op: DENOISE, node: "n0002", downstream: true, scope: { imageKey: RAW, basePath: null } });
    const html = await renderTab(file);
    expect(pillOf(html, "denoise")).toMatch(/class="ab-processing-pill bg-sky-600\/20/);
    expect(html).toContain("data-history-rerun");
    expect(replacedNote(html)).toBeNull();
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

describe("ProcessingTab says before Run which steps a run replaces", () => {
  const SCOPE = { imageKey: RAW, basePath: null };

  it("names the later steps a Background re-run replaces, how to get them back, and links Run to the note", async () => {
    const file = fileOf("replaced-background");
    showHistoryHead(file);
    const html = await renderTab(file);
    const note = replacedNote(html);
    expect(note?.text).toBe(
      "Running Background Extraction again starts a new branch from the original. Wavelet Denoise and Arcsinh Stretch stay in History: Go to on the earlier branch brings them back.",
    );
    expect(runDescribedBy(html, "Extract Background")).toContain(note?.id);
    expect(runDescribedBy(html, "Run Noise Reduction")).not.toContain(note?.id);
  });

  it("names the step a Denoise re-run branches from", async () => {
    const file = fileOf("replaced-denoise");
    showHistoryHead(file);
    historyRerunStore.request({ section: "denoise", op: DENOISE, node: "n0002", downstream: false, scope: SCOPE });
    const html = await renderTab(file);
    const note = replacedNote(html);
    expect(note?.text).toBe(
      "Running Wavelet Denoise again starts a new branch from Background Extraction. Arcsinh Stretch stays in History: Go to on the earlier branch brings it back.",
    );
    expect(runDescribedBy(html, "Run Noise Reduction")).toContain(note?.id);
  });

  it("says nothing when the run replaces only its own result", async () => {
    const file = fileOf("replaced-stretch");
    showHistoryHead(file);
    historyRerunStore.request({ section: "stretch", op: ARCSINH, node: "n0003", downstream: false, scope: SCOPE });
    expect(replacedNote(await renderTab(file))).toBeNull();
  });

  it("says nothing while the chain has no recorded history to name its steps", async () => {
    const file = fileOf("replaced-unrecorded");
    const key = fileKeyOf(file) as string;
    function Publisher() {
      useRenderActions().publishProcessed(
        key,
        { fitsPath: "C:/out/x_stretch.fits", previewUrl: null, dimensions: [8, 8], label: "Stretch", kind: "processing", inputPath: RAW },
        (c) => ({ ...c, steps: { stretch: { fitsPath: "C:/out/x_stretch.fits", previewUrl: null, dimensions: [8, 8] } } }),
      );
      return null;
    }
    renderToStaticMarkup(createElement(CompositeProvider, { children: createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) }) }));
    expect(replacedNote(await renderTab(file))).toBeNull();
  });
});
