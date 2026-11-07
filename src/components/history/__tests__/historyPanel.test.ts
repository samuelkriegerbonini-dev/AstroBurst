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
      blockedReason: null,
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

function buttonTag(html: string, label: string): string {
  const tag = [...html.matchAll(/<button[^>]*>/g)].map((m) => m[0]).find((t) => t.includes(`aria-label="${label}"`));
  if (!tag) throw new Error(`no button labelled ${label}`);
  return tag;
}

function textOf(markup: string): string {
  return markup.replace(/<[^>]*>/g, " ").replace(/&#x27;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, "&").replace(/\s+/g, " ").trim();
}

function goToButton(html: string, id: string): { tag: string; name: string } {
  const match = new RegExp(`<li data-history-row="${id}"[^>]*>(<button[^>]*>)([\\s\\S]*?)</button>`).exec(html);
  if (!match) throw new Error(`no Go to button in row ${id}`);
  const label = /\saria-label="([^"]*)"/.exec(match[1])?.[1];
  return { tag: match[1], name: label ?? textOf(match[2]) };
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

  it("names each Go to button by its visible step, op summary and status instead of the bare node id", () => {
    const html = render({ snapshot: snapshot(BRANCHED) });
    const evicted = goToButton(html, "n0002");
    expect(evicted.name).toBe("n0002 Wavelet Denoise thresholds 3 / 2.5 / 2 / 1.5 / 1 evicted · replayable");
    expect(evicted.tag).not.toContain("aria-label=");
    expect(evicted.tag).not.toContain("title=");
    expect(goToButton(html, "n0003").name).toContain("Arcsinh Stretch");
    expect(goToButton(html, "n0003").name).toContain("cannot be rebuilt");
    expect(goToButton(html, "n0005").name).not.toContain("Go to");
    expect(html).toContain('aria-label="Go to the original image"');
  });

  it("titles the panel with an h3 heading and the file name as its subtitle", () => {
    const html = render({ snapshot: snapshot(BRANCHED), fileName: "jw02739-o001_t001_nircam_clear-f444w_i2d.fits" });
    expect(html).toMatch(/<h3 class="text-sm font-semibold text-zinc-200 tracking-wide">History<\/h3>/);
    expect(html).toContain('title="jw02739-o001_t001_nircam_clear-f444w_i2d.fits">jw02739-o001_t001_nircam_clear-f444w_i2d.fits</span>');
    expect(html.indexOf(">History</h3>")).toBeLessThan(html.indexOf("data-history-undo"));
  });

  it("offers re-run, downstream re-run and branch deletion per step", () => {
    const html = render({ snapshot: snapshot(BRANCHED) });
    expect(goToButton(html, "n0005").name).toContain("Wavelet Denoise");
    expect(html).toContain('aria-label="Re-run n0005 with other settings"');
    expect(html).toContain('aria-label="Re-run n0005 and re-apply the steps after it"');
    expect(html).not.toContain('aria-label="Re-run n0007 and re-apply the steps after it"');
    expect(html).toContain('aria-label="Delete n0005 and the steps after it"');
    expect(html).toContain('aria-label="Go to the original image"');
    expect(html).toContain("Export history JSON");
  });

  it("names the mask of a masked step in its row, and says when that mask was deleted", () => {
    const luma = { kind: "luminance", auto_stf: false, feather_px: 0, invert: false };
    const range = { kind: "range", lo: 0.1, hi: 0.9, feather_px: 0, invert: false };
    const masked = {
      ...doc([node("n0001", null, BACKGROUND), { ...node("n0002", "n0001", DENOISE), mask: "m01" }, { ...node("n0003", "n0002", ARCSINH), mask: "m02" }], "n0003"),
      masks: { m01: luma, m02: range },
      deleted_masks: ["m02"],
    };
    const html = render({ snapshot: snapshot(masked) });
    expect(html).toMatch(/data-history-row="n0002"[\s\S]*?>Wavelet Denoise · mask m01</);
    expect(goToButton(html, "n0002").name).toContain("Wavelet Denoise · mask m01");
    expect(goToButton(html, "n0003").name).toContain("Arcsinh Stretch · mask m02 (deleted)");
    expect(html).toMatch(/data-history-row="n0003"[\s\S]*?>Arcsinh Stretch · mask m02 \(deleted\)</);
    expect(html).toMatch(/data-history-row="n0001"[\s\S]*?>Background Extraction</);
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

  it("with a colour composite on screen, disables Undo, Redo, every Go to and both Re-run buttons and says why in a note", () => {
    const reason =
      "A colour composite is on screen. Undo, Redo and Go to apply to single files: show 673nmos.fits to use its history. Revert to original in the viewer header undoes the composite's processing.";
    const html = render({ snapshot: snapshot(doc(BRANCHED.nodes, "n0006")), blockedReason: reason });
    expect(html).toMatch(/<button[^>]*data-history-undo=""[^>]*disabled=""/);
    expect(html).toMatch(/<button[^>]*data-history-redo=""[^>]*disabled=""/);
    for (const id of ["n0001", "n0002", "n0007"]) {
      expect(goToButton(html, id).tag, id).toContain('disabled=""');
    }
    expect(buttonTag(html, "Go to the original image")).toContain('disabled=""');
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>(?:(?!<\/button>)[\s\S])*Revert to original/);
    expect(html).toMatch(/<div role="note"[^>]*>A colour composite is on screen\. Undo, Redo and Go to apply to single files: show 673nmos\.fits/);
    expect(html.indexOf('role="note"')).toBeLessThan(html.indexOf('data-history-row="root"'));
    for (const label of ["Re-run n0005 with other settings", "Re-run n0005 and re-apply the steps after it", "Re-run n0001 with other settings"]) {
      expect(buttonTag(html, label), label).toContain('disabled=""');
    }
    expect(buttonTag(html, "Delete n0005 and the steps after it")).not.toContain('disabled=""');
  });

  it("keeps Undo, Redo and Go to enabled and shows no composite note on a single file", () => {
    const html = render({ snapshot: snapshot(doc(BRANCHED.nodes, "n0006")) });
    expect(html).toMatch(/<button[^>]*data-history-undo=""(?![^>]*disabled="")/);
    expect(html).toMatch(/<button[^>]*data-history-redo=""(?![^>]*disabled="")/);
    for (const id of ["n0001", "n0007"]) {
      expect(goToButton(html, id).tag, id).not.toContain('disabled=""');
    }
    for (const label of ["Go to the original image", "Re-run n0005 with other settings", "Re-run n0005 and re-apply the steps after it"]) {
      expect(buttonTag(html, label), label).not.toContain('disabled=""');
    }
    expect(html).not.toMatch(/<button[^>]*disabled=""[^>]*>(?:(?!<\/button>)[\s\S])*Revert to original/);
    expect(html).not.toContain("colour composite");
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
