import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ChannelFile } from "../../compose/SmartChannelMapper";
import type { ProcessedFile } from "../../../shared/types";

const { captured } = vi.hoisted(() => ({
  captured: { files: [] as ChannelFile[] },
}));

vi.mock("../../compose/SmartChannelMapper", () => ({
  default: (props: { files: ChannelFile[] }) => {
    captured.files = props.files;
    return null;
  },
}));
vi.mock("../../../hooks/useProgress", () => ({
  useProgress: () => ({ active: false, percent: 0, stage: "", reset: () => {} }),
}));
vi.mock("../../../services/stacking", () => ({ calibrate: vi.fn() }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));

import CalibrationPanel from "../CalibrationPanel";

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
