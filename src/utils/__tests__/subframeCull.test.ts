import { describe, it, expect } from "vitest";
import {
  SUBFRAME_NOTICE_NAMES,
  cullChannelGroups,
  cullDrizzleChannels,
  fileSetKey,
  liveSubframeSelection,
  panelRejectedPaths,
  pipelineChannelInputs,
  subframeExclusionNotice,
  withAllFramesIn,
  type SubframeSelection,
} from "../subframeCull";
import type { PipelineChannel } from "../../shared/types/stacking";

const DIR = "C:\\data\\subsel\\";

function frames(prefix: string, n: number): string[] {
  return Array.from({ length: n }, (_, i) => `${DIR}${prefix}_${String(i).padStart(4, "0")}.fits`);
}

const R = frames("R", 10);
const G = frames("G", 4);
const B = frames("B", 4);
const X_9 = `${DIR}X_0009.fits`;
const REJECTED = [R[1], R[4], R[7], X_9];

function groups(): PipelineChannel[] {
  return [
    { label: "R", paths: [...R] },
    { label: "G", paths: [...G] },
    { label: "B", paths: [...B] },
  ];
}

describe("fileSetKey", () => {
  it("fileSetKey ignores order and duplicates", () => {
    expect(fileSetKey(["/b.fits", "/a.fits", "/a.fits"])).toBe(fileSetKey(["/a.fits", "/b.fits"]));
    expect(fileSetKey(["/b.fits", "/a.fits", "/a.fits"])).toBe("/a.fits\n/b.fits");
    expect(fileSetKey(["/a.fits"])).not.toBe(fileSetKey(["/a.fits", "/b.fits"]));
    expect(fileSetKey([])).toBe("");
  });
});

describe("liveSubframeSelection", () => {
  it("a selection lives while the file list is the same", () => {
    const paths = [...R, ...G, ...B];
    const selection: SubframeSelection = {
      accepted: paths.filter((p) => !REJECTED.includes(p)),
      rejected: [R[1], R[4], R[7]],
      fileKey: fileSetKey(paths),
    };
    expect(liveSubframeSelection(selection, [...paths].reverse())).toBe(selection);
    expect(liveSubframeSelection(selection, [...paths, X_9])).toBeNull();
    expect(liveSubframeSelection(selection, paths.slice(1))).toBeNull();
    expect(liveSubframeSelection(null, paths)).toBeNull();
  });

  it("a selection dropped by a file-list change stays dropped when the old list returns", () => {
    const paths = [...R, ...G, ...B];
    const applied: SubframeSelection = { accepted: [R[0]], rejected: [R[1], R[4], R[7]], fileKey: fileSetKey(paths) };
    const stored = liveSubframeSelection(applied, [...paths, X_9]);
    expect(stored).toBeNull();
    expect(liveSubframeSelection(stored, paths)).toBeNull();
    expect(panelRejectedPaths(stored, "pipeline")).toEqual([]);
  });
});

describe("Use all frames per panel", () => {
  const paths = [...R, ...G, ...B];
  const weights = { [R[0]]: 0.8, [R[2]]: 1.2 };
  const applied = (): SubframeSelection => ({
    accepted: paths.filter((p) => !REJECTED.includes(p)),
    rejected: [R[1], R[4], R[7]],
    weights,
    fileKey: fileSetKey(paths),
  });

  it("every panel culls the applied rejections until it opts out", () => {
    const selection = applied();
    expect(panelRejectedPaths(selection, "pipeline")).toBe(selection.rejected);
    expect(panelRejectedPaths(selection, "drizzle")).toBe(selection.rejected);
  });

  it("Use all frames in the Pipeline keeps the Stack weights, the rejected tags and the Drizzle cull", () => {
    const selection = applied();
    const next = withAllFramesIn(selection, "pipeline");
    expect(next).not.toBeNull();
    expect(next?.accepted).toBe(selection.accepted);
    expect(next?.rejected).toBe(selection.rejected);
    expect(next?.weights).toBe(weights);
    expect(next?.fileKey).toBe(selection.fileKey);
    expect(panelRejectedPaths(next, "pipeline")).toEqual([]);
    expect(panelRejectedPaths(next, "drizzle")).toBe(selection.rejected);
    expect(panelRejectedPaths(selection, "pipeline")).toBe(selection.rejected);
  });

  it("Use all frames in the Drizzle panel leaves the Pipeline cull alone", () => {
    const selection = applied();
    const next = withAllFramesIn(selection, "drizzle");
    expect(panelRejectedPaths(next, "drizzle")).toEqual([]);
    expect(panelRejectedPaths(next, "pipeline")).toBe(selection.rejected);
    const both = withAllFramesIn(next, "pipeline");
    expect(panelRejectedPaths(both, "pipeline")).toEqual([]);
    expect(panelRejectedPaths(both, "drizzle")).toEqual([]);
    expect(both?.rejected).toBe(selection.rejected);
    expect(withAllFramesIn(both, "drizzle")).toBe(both);
  });

  it("a new Apply culls in every panel again", () => {
    const optedOut = withAllFramesIn(applied(), "pipeline");
    expect(panelRejectedPaths(optedOut, "pipeline")).toEqual([]);
    const reapplied = applied();
    expect(panelRejectedPaths(reapplied, "pipeline")).toBe(reapplied.rejected);
  });

  it("no selection means nothing to cull, with one stable empty list", () => {
    expect(withAllFramesIn(null, "pipeline")).toBeNull();
    expect(panelRejectedPaths(null, "pipeline")).toEqual([]);
    expect(panelRejectedPaths(null, "pipeline")).toBe(panelRejectedPaths(withAllFramesIn(applied(), "drizzle"), "drizzle"));
  });
});

describe("cullChannelGroups / pipelineChannelInputs", () => {
  it("the pipeline request omits culled frames", () => {
    const { inputs, excluded } = pipelineChannelInputs(groups(), REJECTED);
    expect(inputs.map((g) => g.label)).toEqual(["R", "G", "B"]);
    expect(inputs[0].paths).toEqual([R[0], R[2], R[3], R[5], R[6], R[8], R[9]]);
    expect(inputs[0].paths).toHaveLength(7);
    expect(inputs[1].paths).toEqual(G);
    expect(inputs[2].paths).toEqual(B);
    expect(excluded).toEqual([R[1], R[4], R[7]]);
  });

  it("lists each excluded frame once, in the order of the channel lists", () => {
    const input: PipelineChannel[] = [
      { label: "R", paths: [R[0], R[4], R[1]] },
      { label: "G", paths: [R[4], G[0]] },
    ];
    const { groups: culled, excluded } = cullChannelGroups(input, [R[1], R[4]]);
    expect(culled).toEqual([
      { label: "R", paths: [R[0]] },
      { label: "G", paths: [G[0]] },
    ]);
    expect(excluded).toEqual([R[4], R[1]]);
    expect(input[0].paths).toEqual([R[0], R[4], R[1]]);
  });

  it("a channel emptied by the cull is dropped from the request but kept for display", () => {
    const input: PipelineChannel[] = [
      { label: "R", paths: [R[1], R[4]] },
      { label: "G", paths: [...G] },
      { label: "B", paths: [] },
    ];
    const display = cullChannelGroups(input, REJECTED);
    expect(display.groups.map((g) => [g.label, g.paths.length])).toEqual([["R", 0], ["G", 4], ["B", 0]]);
    expect(display.excluded).toEqual([R[1], R[4]]);
    const request = pipelineChannelInputs(input, REJECTED);
    expect(request.inputs).toEqual([{ label: "G", paths: G }]);
    expect(request.excluded).toEqual([R[1], R[4]]);
  });

  it("keeps every frame when nothing is rejected", () => {
    const { inputs, excluded } = pipelineChannelInputs(groups(), []);
    expect(inputs).toEqual(groups());
    expect(excluded).toEqual([]);
  });
});

describe("cullDrizzleChannels", () => {
  it("drizzle channels omit culled frames", () => {
    const { channels, excluded } = cullDrizzleChannels({ r: [...R], g: [...G], b: [] as string[] }, REJECTED);
    expect(channels.r.length).toBe(7);
    expect(channels.r).toEqual([R[0], R[2], R[3], R[5], R[6], R[8], R[9]]);
    expect(channels.g).toEqual(G);
    expect(channels.b).toEqual([]);
    expect(Object.keys(channels).sort()).toEqual(["b", "g", "r"]);
    expect(excluded).toEqual([R[1], R[4], R[7]]);
  });
});

describe("subframeExclusionNotice", () => {
  it("notice", () => {
    expect(subframeExclusionNotice([])).toBeNull();
    expect(subframeExclusionNotice([R[1], R[4], R[7]])).toBe(
      "3 excluded by Subframe Selector: R_0001.fits, R_0004.fits, R_0007.fits",
    );
    const eight = R.slice(0, 8);
    expect(SUBFRAME_NOTICE_NAMES).toBe(5);
    expect(subframeExclusionNotice(eight)).toBe(
      "8 excluded by Subframe Selector: R_0000.fits, R_0001.fits, R_0002.fits, R_0003.fits, R_0004.fits and 3 more",
    );
    expect(subframeExclusionNotice(["/night/L_01.fits"])).toBe("1 excluded by Subframe Selector: L_01.fits");
  });
});
