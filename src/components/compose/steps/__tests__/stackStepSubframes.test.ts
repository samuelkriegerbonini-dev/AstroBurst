import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { SubframeAnalysisResult, WizardState } from "../../../../utils/wizard";

const wizard = vi.hoisted(() => ({ state: null as WizardState | null }));

vi.mock("../../../../context/ComposeWizardContext", () => ({
  useComposeWizardContext: () => ({
    stackRun: null,
    setStackRun: vi.fn(),
    finishStackRun: vi.fn(),
    getStackGeneration: () => 0,
    getState: () => wizard.state,
    stackDiscarded: {},
    recordStackOutcome: vi.fn(),
  }),
}));
vi.mock("../../../../services/stacking", () => ({ stackFrames: vi.fn(), drizzleFrames: vi.fn() }));
vi.mock("../../../../services/analysis", () => ({ analyzeSubframes: vi.fn() }));
vi.mock("../../../../services/progress", () => ({ cancelProgress: vi.fn() }));
vi.mock("../../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));
vi.mock("../../../../hooks/useProgress", () => ({
  useProgress: () => ({ active: false, percent: 0, stage: "", current: 0, total: 0, reset: () => {} }),
}));
vi.mock("../../../../hooks/useFileStore", () => ({ resolveEffectivePath: (path: string) => path }));

import StackStep from "../StackStep";
import { INITIAL_STATE } from "../../../../utils/wizard";

const R = ["/sx03/r_0001.fits", "/sx03/r_0002.fits", "/sx03/r_0003.fits"];
const G = ["/sx03/g_0001.fits", "/sx03/g_0002.fits", "/sx03/g_0003.fits"];

function analysedAt(paths: string[], maxFwhm: number): SubframeAnalysisResult {
  return {
    subframes: paths.map((path, i) => ({
      file_path: path,
      file_name: path.split("/").pop() ?? path,
      star_count: 40,
      median_fwhm: 3 + i,
      median_eccentricity: 0.3,
      median_snr: 20,
      background_median: 100,
      background_sigma: 5,
      noise_ratio: 0.05,
      noise_sigma: 0.001234,
      noise_fraction: 0.9,
      weight: 1,
      accepted: true,
    })),
    total: paths.length,
    accepted: paths.length,
    rejected: 0,
    elapsed_ms: 12,
    analysedWith: { maxFwhm, maxEccentricity: 0.7, minSnr: 5, minStars: 5 },
  };
}

function render(subframeResults: Record<string, SubframeAnalysisResult>): string {
  const state: WizardState = {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: b.id === "r" ? R : b.id === "g" ? G : [] })),
    subframeResults,
  };
  wizard.state = state;
  return renderToStaticMarkup(createElement(StackStep, { state, dispatch: vi.fn(), onStacked: vi.fn() }));
}

function applyDisabled(html: string): boolean[] {
  return [...html.matchAll(/<button[^>]*data-testid="stack-subframe-apply"[^>]*>/g)].map((m) => m[0].includes('disabled=""'));
}

function staleLines(html: string): string[] {
  return [...html.matchAll(/<div[^>]*data-testid="stack-subframe-stale"[^>]*>([^<]*)<\/div>/g)].map((m) => m[1]).filter((text) => text !== "");
}

function maxFwhmShown(html: string): string | null {
  return /aria-label="Edit Max FWHM value"[^>]*>([^<]*)<\/button>/.exec(html)?.[1] ?? null;
}

describe("compose Stack step subframe selection", () => {
  it("after Max FWHM changes, Apply is disabled and the stale line shows for the channel analysed with the old value", () => {
    const html = render({ r: analysedAt(R, 6.5), g: analysedAt(G, 5) });
    expect(maxFwhmShown(html)).toBe("6.5 px");
    expect(applyDisabled(html)).toEqual([false, true]);
    expect(staleLines(html)).toEqual(["Settings changed since the analysis: Analyze again"]);
  });

  it("describes the disabled Apply with the stale line", () => {
    const html = render({ r: analysedAt(R, 6.5), g: analysedAt(G, 5) });
    const id = /<div id="([^"]+)"[^>]*data-testid="stack-subframe-stale"[^>]*>Settings changed/.exec(html)?.[1];
    expect(id).toBeTruthy();
    const applies = [...html.matchAll(/<button[^>]*data-testid="stack-subframe-apply"[^>]*>/g)].map((m) => m[0]);
    expect(applies.filter((tag) => tag.includes(`aria-describedby="${id}"`) && tag.includes('disabled=""'))).toHaveLength(1);
  });

  it("keeps Apply enabled and starts the sliders from the analysis when they match", () => {
    const html = render({ r: analysedAt(R, 8) });
    expect(maxFwhmShown(html)).toBe("8.0 px");
    expect(applyDisabled(html)).toEqual([false]);
    expect(staleLines(html)).toEqual([]);
  });
});
