import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ProcessedFile } from "../../../shared/types";
import type { PipelineRequest, PipelineStats } from "../../../shared/types/stacking";
import type { CalibrationState } from "../StackingTab";

const { captured, runPipelineMock } = vi.hoisted(() => ({
  captured: { onRun: null as null | (() => void) },
  runPipelineMock: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui")>();
  return {
    ...actual,
    RunButton: (props: { onClick: () => void }) => {
      captured.onRun = props.onClick;
      return null;
    },
  };
});
vi.mock("../../../services/stacking", () => ({ runCalibrationPipeline: runPipelineMock }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));

import PipelinePanel, { PipelineMasterLines } from "../PipelinePanel";

const RED_FILTER: Record<string, string> = { FILTER: "Red" };

function light(name: string): ProcessedFile {
  return {
    id: name,
    name,
    path: `C:/r6/a1/calibset/${name}`,
    sourcePath: `C:/r6/a1/calibset/${name}`,
    imageRef: null,
    size: 0,
    status: "done",
    result: { header: RED_FILTER } as ProcessedFile["result"],
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

const LIGHTS = ["light_120s.fits", "light_120s_2.fits"].map(light);
const CALIBRATION: CalibrationState = {
  calibratedPath: null,
  calibratedFitsPath: null,
  hasBias: true,
  hasDark: true,
  hasFlat: false,
  darkPaths: ["C:/r6/a1/calibset/dark_300s_1.fits"],
  flatPaths: [],
  biasPaths: ["C:/r6/a1/calibset/bias_1.fits"],
  flatDarkPaths: [],
};

function stats(extra: Partial<PipelineStats>): PipelineStats {
  return { darks_combined: 4, flats_combined: 4, bias_combined: 4, channels: [], ...extra };
}

function linesOf(s: PipelineStats): string[] {
  const html = renderToStaticMarkup(createElement(PipelineMasterLines, { stats: s }));
  return [...html.matchAll(/<div>([^<]*)<\/div>/g)].map((m) => m[1]);
}

describe("PipelinePanel flat-dark row and request", () => {
  beforeEach(() => {
    captured.onRun = null;
    runPipelineMock.mockReset();
    runPipelineMock.mockResolvedValue({ stats: stats({}), channel_previews: [] });
  });

  it("shows an empty Flat darks row", () => {
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files: LIGHTS, calibration: CALIBRATION }));
    expect(html).toMatch(/<div[^>]*data-testid="pipeline-flatdark-row"[^>]*><span[^>]*>Flat darks: 0<\/span>/);
  });

  it("always sends flat_dark_paths and leaves cosmetic correction off by default", async () => {
    renderToStaticMarkup(createElement(PipelinePanel, { files: LIGHTS, calibration: CALIBRATION }));
    captured.onRun?.();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(1));
    const request = runPipelineMock.mock.calls[0][0] as PipelineRequest;
    expect(request.flat_dark_paths).toEqual([]);
    expect(request.dark_paths).toEqual(CALIBRATION.darkPaths);
    expect(request.bias_paths).toEqual(CALIBRATION.biasPaths);
    expect(request.channels).toEqual([{ label: "R", paths: LIGHTS.map((f) => f.path) }]);
    expect(request.cosmetic).toBeNull();
  });

  it("seeds the Flat darks row and the request with the Calibration tab flat-darks", async () => {
    const flatDarkPaths = ["C:/r6/a1/calibset/flatdark_2s_1.fits", "C:/r6/a1/calibset/flatdark_2s_2.fits"];
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files: LIGHTS, calibration: { ...CALIBRATION, flatDarkPaths } }));
    expect(html).toMatch(/<div[^>]*data-testid="pipeline-flatdark-row"[^>]*><span[^>]*>Flat darks: 2<\/span>/);
    captured.onRun?.();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(1));
    expect((runPipelineMock.mock.calls[0][0] as PipelineRequest).flat_dark_paths).toEqual(flatDarkPaths);
  });
});

describe("PipelineMasterLines", () => {
  it("names the master flat-dark and the dark temperature groups", () => {
    expect(linesOf(stats({ flat_darks_combined: 4, dark_groups: [{ temp_c: -10, frames: 4 }] }))).toEqual([
      "Master dark: 4 frames",
      "Master flat: 4 frames",
      "Master flat-dark: 4 frames",
      "dark groups: -10.0 °C (4)",
    ]);
  });

  it("lists every dark group with one decimal and an ASCII minus", () => {
    const lines = linesOf(stats({ dark_groups: [{ temp_c: -20.06, frames: 12 }, { temp_c: -10, frames: 8 }, { temp_c: 0.04, frames: 5 }] }));
    expect(lines).toContain("dark groups: -20.1 °C (12), -10.0 °C (8), 0.0 °C (5)");
    expect(lines.join("\n")).not.toContain("\u2212");
  });

  it("leaves out the flat-dark line without flat-darks and the group line without temperatures", () => {
    expect(linesOf(stats({ flat_darks_combined: 0, dark_groups: [{ temp_c: null, frames: 4 }] }))).toEqual([
      "Master dark: 4 frames",
      "Master flat: 4 frames",
    ]);
    expect(linesOf(stats({}))).toEqual(["Master dark: 4 frames", "Master flat: 4 frames"]);
  });

  it("marks a group without a temperature when other groups have one", () => {
    expect(linesOf(stats({ dark_groups: [{ temp_c: -10, frames: 6 }, { temp_c: null, frames: 5 }] }))).toContain(
      "dark groups: -10.0 °C (6), unknown (5)",
    );
  });
});
