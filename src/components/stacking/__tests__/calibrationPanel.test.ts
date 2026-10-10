import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { CalibAssignment, ChannelFile } from "../../compose/SmartChannelMapper";
import type { CalibrateResult, ProcessedFile } from "../../../shared/types";

const { captured, calibrateMock } = vi.hoisted(() => ({
  captured: { files: [] as ChannelFile[], onCalibrate: null as null | ((assignments: CalibAssignment) => void) },
  calibrateMock: vi.fn(),
}));

vi.mock("../../compose/SmartChannelMapper", () => ({
  default: (props: { files: ChannelFile[]; onCalibrate?: (assignments: CalibAssignment) => void }) => {
    captured.files = props.files;
    captured.onCalibrate = props.onCalibrate ?? null;
    return null;
  },
}));
vi.mock("../../../hooks/useProgress", () => ({
  useProgress: () => ({ active: false, percent: 0, stage: "", reset: () => {} }),
}));
vi.mock("../../../services/stacking", () => ({ calibrate: calibrateMock }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));

import CalibrationPanel, { CalibrationResultLines } from "../CalibrationPanel";

function processed(name: string, header: Record<string, string>): ProcessedFile {
  return {
    id: name,
    name,
    path: `C:/heavy/${name}`,
    sourcePath: `C:/heavy/${name}`,
    imageRef: null,
    size: 0,
    status: "done",
    result: { header } as ProcessedFile["result"],
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function mapperFilters(files: ProcessedFile[]): (string | undefined)[] {
  renderToStaticMarkup(createElement(CalibrationPanel, { files }));
  return captured.files.map((f) => f.filter);
}

describe("Calibration mapper filter chip", () => {
  it("shows the NIRCam pupil-wheel filter like the file list chip", () => {
    expect(mapperFilters([
      processed("jw02739-o001_t001_nircam_f444w-f470n_i2d.fits", { INSTRUME: "NIRCAM", FILTER: "F444W", PUPIL: "F470N" }),
      processed("jw02739-o001_t001_nircam_clear-f444w_i2d.fits", { INSTRUME: "NIRCAM", FILTER: "F444W", PUPIL: "CLEAR" }),
    ])).toEqual(["F470N", "F444W"]);
  });

  it("shows the WFPC2 filter name and no chip for a frame without a filter", () => {
    expect(mapperFilters([
      processed("656nmos.fits", { INSTRUME: "WFPC2", FILTNAM1: "F656N", FILTER1: "31", FILTER2: "0" }),
      processed("bias_001.fits", { INSTRUME: "ASI2600" }),
      processed("light_red.fits", { INSTRUME: "ASI2600", FILTER: "Red" }),
    ])).toEqual(["F656N", undefined, "Red"]);
  });
});

function channelFile(name: string): ChannelFile {
  return { id: name, path: `C:/r6/a1/calibset/${name}`, name };
}

const LIGHT = channelFile("light_120s.fits");
const DARKS = ["dark_300s_1.fits", "dark_300s_2.fits"].map(channelFile);
const BIAS = ["bias_1.fits", "bias_2.fits"].map(channelFile);
const FLATS = ["flat_2s_1.fits"].map(channelFile);

async function calibrateOptions(assignment: CalibAssignment): Promise<Record<string, unknown>> {
  renderToStaticMarkup(createElement(CalibrationPanel, { files: [] }));
  captured.onCalibrate?.(assignment);
  await vi.waitFor(() => expect(calibrateMock).toHaveBeenCalledTimes(1));
  const [sciencePath, , options] = calibrateMock.mock.calls[0];
  expect(sciencePath).toBe(LIGHT.path);
  return options as Record<string, unknown>;
}

describe("Calibration tab request", () => {
  beforeEach(() => {
    calibrateMock.mockReset();
    calibrateMock.mockResolvedValue({ png_path: "C:/out/light.png", dimensions: [256, 256], elapsed_ms: 1 });
  });

  it("renders no dark exposure ratio slider", () => {
    expect(renderToStaticMarkup(createElement(CalibrationPanel, { files: [] }))).not.toContain("Dark Exposure Ratio");
  });

  it("leaves the dark scale to the backend and sends the flat-dark frames", async () => {
    const flatDarks = ["flatdark_1.fits", "flatdark_2.fits"].map(channelFile);
    const options = await calibrateOptions({ science: LIGHT, bias: BIAS, dark: DARKS, flat: FLATS, flatdark: flatDarks });
    expect(options).not.toHaveProperty("darkExposureRatio");
    expect(options.flatDarkPaths).toEqual(flatDarks.map((f) => f.path));
    expect(options.darkPaths).toEqual(DARKS.map((f) => f.path));
    expect(options.biasPaths).toEqual(BIAS.map((f) => f.path));
    expect(options.flatPaths).toEqual(FLATS.map((f) => f.path));
  });

  it("sends an empty flat-dark list when the slot is empty", async () => {
    const options = await calibrateOptions({ science: LIGHT, bias: [], dark: DARKS, flat: [], flatdark: [] });
    expect(options.flatDarkPaths).toEqual([]);
    expect(options).not.toHaveProperty("darkExposureRatio");
  });
});

function resultLines(extra: Partial<CalibrateResult>): string[] {
  const result: CalibrateResult = { png_path: "C:/out/light.png", dimensions: [256, 256], elapsed_ms: 1, ...extra };
  const html = renderToStaticMarkup(createElement(CalibrationResultLines, { result }));
  return [...html.matchAll(/<div[^>]*>([^<]*)<\/div>/g)].map((m) => m[1]);
}

describe("Calibration result lines", () => {
  it("reports the flat-dark subtraction when flats were divided", () => {
    expect(resultLines({ has_dark: true, has_flat: true, has_flat_dark: true })).toEqual(
      expect.arrayContaining(["Flat-dark subtracted from the flats", "Flat divided"]),
    );
  });

  it("does not claim a flat-dark subtraction when no flat was given", () => {
    const lines = resultLines({ has_dark: true, has_flat: false, has_flat_dark: true });
    expect(lines).toContain("Dark subtracted");
    expect(lines).not.toContain("Flat-dark subtracted from the flats");
    expect(lines).not.toContain("Flat divided");
  });
});

describe("Calibration mapper slots", () => {
  it("offers a Flat dark slot after the Flat slot", async () => {
    const actual = await vi.importActual<typeof import("../../compose/SmartChannelMapper")>("../../compose/SmartChannelMapper");
    const html = renderToStaticMarkup(createElement(actual.default, { mode: "calibration", files: [] }));
    const labels = [...html.matchAll(/class="ab-channel-name">([^<]*)</g)].map((m) => m[1]);
    expect(labels).toEqual(["Science", "Bias", "Dark", "Flat", "Flat dark"]);
  });
});
