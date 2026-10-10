import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ProcessedFile } from "../../../shared/types";

vi.mock("../../../hooks/useProgress", () => ({
  useProgress: () => ({ active: false, percent: 0, stage: "", current: 0, total: 0, reset: () => {} }),
}));
vi.mock("../../../services/stacking", () => ({ noiseWeightsFor: vi.fn(), stackFrames: vi.fn() }));
vi.mock("../../../services/progress", () => ({ cancelProgress: vi.fn() }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));

import StackingPanel, { NoiseWeightHintLine } from "../StackingPanel";

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
