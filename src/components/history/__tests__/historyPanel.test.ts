import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { HistoryDoc, HistoryNode, HistoryState, NodeEstimate, OpSpec } from "../../../shared/types/ops";
import { EMPTY_HISTORY_SNAPSHOT, type HistorySnapshot } from "../../../utils/historyStore";
import { HistoryPanelView, type HistoryPanelViewProps } from "../HistoryPanel";
import { formatDecimalBytes } from "../../../utils/format";

const DIR = "C:/app/history/3f9c0a1e5b7d2c44";
const SOURCE = "C:/data/jw01475001001_02101_00001_nrca1_i2d.fits#hdu=1";

const BACKGROUND: OpSpec = {
  op: "background",
  v: 1,
  model: "polynomial",
  config: { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" },
};
const DENOISE: OpSpec = { op: "wavelet_denoise", v: 1, numScales: 5, thresholds: [3, 2.5, 2, 1.5, 1], linearDenoise: true, layerBias: null };
const DENOISE_STRONG: OpSpec = { op: "wavelet_denoise", v: 1, numScales: 5, thresholds: [4, 3, 2, 1.5, 1], linearDenoise: true, layerBias: null };
const ARCSINH: OpSpec = { op: "arcsinh", v: 1, factor: 50 };
const TONE: OpSpec = { op: "tone", v: 1, levels: { black: 0.1, gamma: 1, white: 0.9 } };

function node(id: string, parent: string | null, op: OpSpec, present = true, replayable?: boolean): HistoryNode {
  return {
    id,
    parent,
    op,
    mask: null,
    output: { path: `${id}.fits`, png: `${id}.png`, extras: [], dims: [2058, 2058], planes: 1, domain: "linear" },
    at: "2026-10-16T12:00:00Z",
    elapsed_ms: 812,
    app: "0.6.9",
    threads: 16,
    present,
    bytes: 16_941_456,
    checkpoint: false,
    ...(replayable === undefined ? {} : { replayable }),
  };
}

function doc(nodes: HistoryNode[], head: string | null): HistoryDoc {
  return {
    format: "astroburst-history",
    version: 1,
    image: { id: "3f9c0a1e5b7d2c44", ref: SOURCE, size: 1, mtime: null },
    image_key: SOURCE,
    base_path: null,
    head,
    policy: { k: 4 },
    app: "0.6.9",
    threads: 16,
    masks: {},
    nodes,
  };
}

function snapshot(d: HistoryDoc | null, patch: Partial<HistoryState> = {}, error: string | null = null): HistorySnapshot {
  const state: HistoryState = { image_id: "3f9c0a1e5b7d2c44", history_dir: DIR, root_path: SOURCE, doc: d, read_only: null, fingerprint_matches: true, busy: false, ...patch };
  return { ...EMPTY_HISTORY_SNAPSHOT, fileKey: `1|${SOURCE}`, scope: { imageKey: SOURCE, basePath: null }, state, error };
}

const ESTIMATE: NodeEstimate = { next_node_bytes: 16_941_456, dimensions: [2058, 2058], planes: 1, history_bytes: 254_121_840, history_files: 32, history_root: "C:/app/history" };

function render(props: Partial<HistoryPanelViewProps>): string {
  const noop = vi.fn();
  return renderToStaticMarkup(
    createElement(HistoryPanelView, {
      snapshot: snapshot(null),
      estimate: null,
      fileName: "nrca1_i2d.fits",
      onGoTo: noop,
      onRerun: noop,
      onDelete: noop,
      onUndo: noop,
      onRedo: noop,
      onExport: noop,
      onRefresh: noop,
      ...props,
    }),
  );
}

function rowOrder(html: string): string[] {
  return [...html.matchAll(/data-history-row="([^"]+)"/g)].map((m) => m[1]);
}

const BRANCHED = doc(
  [
    node("n0001", null, BACKGROUND),
    node("n0002", "n0001", DENOISE, false),
    node("n0003", "n0002", ARCSINH, false, false),
    node("n0004", "n0003", TONE),
    node("n0005", "n0001", DENOISE_STRONG),
    node("n0006", "n0005", ARCSINH),
    node("n0007", "n0006", TONE),
  ],
  "n0006",
);

describe("HistoryPanelView", () => {
  it("history_panel_lists_the_active_path_with_a_branch_marker_and_size_estimate", () => {
    const html = render({ snapshot: snapshot(BRANCHED), estimate: ESTIMATE });
    expect(rowOrder(html)).toEqual(["root", "n0001", "n0002", "n0003", "n0004", "n0005", "n0006", "n0007"]);
    expect(html).toMatch(/data-history-row="n0006"[^>]*aria-current="step"/);
    expect(html).toMatch(/data-history-row="n0007"[^>]*data-undone="true"/);
    expect(html).toContain("Wavelet Denoise");
    expect(html).toContain("thresholds 4 / 3 / 2 / 1.5 / 1");
    expect(html).toMatch(/data-branch-marker="n0001"[^>]*>[\s\S]*?1 other branch[\s\S]*?data-history-row="n0002"/);
    expect(html).toMatch(/<details>[\s\S]*data-history-row="n0004"[\s\S]*<\/details>[\s\S]*data-history-row="n0005"/);
    expect(html).toContain("Next step ≈ 16.9 MB");
    expect(html).toContain("history/ 254.1 MB · 32 files");
  });

  it("shows evicted steps as replayable and lost ones as not rebuildable", () => {
    const html = render({ snapshot: snapshot(BRANCHED) });
    expect(html).toMatch(/data-history-row="n0002"[\s\S]*?replayable/);
    expect(html).toMatch(/data-history-row="n0003"[\s\S]*?cannot be rebuilt/);
  });

  it("offers re-run, downstream re-run and branch deletion per step", () => {
    const html = render({ snapshot: snapshot(BRANCHED) });
    expect(html).toContain('aria-label="Go to n0005"');
    expect(html).toContain('aria-label="Re-run n0005 with other settings"');
    expect(html).toContain('aria-label="Re-run n0005 and re-apply the steps after it"');
    expect(html).not.toContain('aria-label="Re-run n0007 and re-apply the steps after it"');
    expect(html).toContain('aria-label="Delete n0005 and the steps after it"');
    expect(html).toContain('aria-label="Go to the original image"');
    expect(html).toContain("Export history JSON");
  });

  it("enables undo and redo from the head position", () => {
    const atHead = render({ snapshot: snapshot(doc(BRANCHED.nodes, "n0007")) });
    expect(atHead).toMatch(/<button[^>]*data-history-undo=""(?![^>]*disabled="")/);
    expect(atHead).toMatch(/<button[^>]*data-history-redo=""[^>]*disabled=""/);
    const atRoot = render({ snapshot: snapshot(doc(BRANCHED.nodes, null)) });
    expect(atRoot).toMatch(/<button[^>]*data-history-undo=""[^>]*disabled=""/);
    expect(rowOrder(atRoot)).toEqual(["root", "n0001", "n0002", "n0003", "n0004", "n0005", "n0006", "n0007"]);
    expect(atRoot).toMatch(/data-history-row="n0005"[^>]*data-undone="true"/);
    expect(atRoot).toMatch(/data-history-row="root"[^>]*aria-current="step"/);
  });

  it("says why a history is read-only, why a changed source cannot replay, and what is empty", () => {
    expect(render({ snapshot: snapshot(BRANCHED, { read_only: "this history was written with history format version 2" }) })).toContain(
      "format version 2",
    );
    expect(render({ snapshot: snapshot(BRANCHED, { fingerprint_matches: false }) })).toContain("source file changed");
    expect(render({ snapshot: snapshot(null) })).toContain("No processing steps yet");
    expect(render({ snapshot: snapshot(null), estimate: ESTIMATE })).toContain("Next step ≈ 16.9 MB");
    expect(render({ snapshot: { ...EMPTY_HISTORY_SNAPSHOT } })).toContain("Select an image");
    expect(render({ snapshot: snapshot(BRANCHED, {}, "a step is still running on this image") })).toContain("a step is still running on this image");
  });

  it("formats sizes in decimal units like the plan's estimates", () => {
    expect(formatDecimalBytes(16_941_456)).toBe("16.9 MB");
    expect(formatDecimalBytes(492_815_664)).toBe("492.8 MB");
    expect(formatDecimalBytes(5_421_778_560)).toBe("5.42 GB");
    expect(formatDecimalBytes(900)).toBe("900 B");
    expect(formatDecimalBytes(12_000)).toBe("12.0 kB");
  });
});
