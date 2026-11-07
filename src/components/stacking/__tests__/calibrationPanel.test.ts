import { withAllFramesOfKind, toggleCalibFrame } from "../../compose/calibFrames";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { CalibAssignment, ChannelFile } from "../../compose/SmartChannelMapper";
import type { CalibrateResult, ProcessedFile } from "../../../shared/types";
import type { CalibrationState } from "../StackingTab";

const { captured, calibrateMock, runPipelineMock } = vi.hoisted(() => ({
  captured: {
    files: [] as ChannelFile[],
    onCalibrate: null as null | ((assignments: CalibAssignment) => void),
    note: null as unknown,
  },
  calibrateMock: vi.fn(),
  runPipelineMock: vi.fn(),
}));

vi.mock("../../compose/SmartChannelMapper", () => ({
  default: (props: { files: ChannelFile[]; onCalibrate?: (assignments: CalibAssignment) => void; note?: unknown }) => {
    captured.files = props.files;
    captured.onCalibrate = props.onCalibrate ?? null;
    captured.note = props.note ?? null;
    return null;
  },
}));
vi.mock("../../../hooks/useProgress", () => ({
  useProgress: () => ({ active: false, percent: 0, stage: "", reset: () => {} }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../../services/stacking", () => ({ calibrate: calibrateMock, runCalibrationPipeline: runPipelineMock }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));

import CalibrationPanel, { CalibrationResultLines, CALIBRATE_SCOPE_NOTE, PipelineHandoff, type CalibrationMasters } from "../CalibrationPanel";
import PipelinePanel from "../PipelinePanel";
import { withCalibrationFrames } from "../../../utils/calibrationHandoff";

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
  it("names each empty picker by its calibration slot and exposes its collapsed state", async () => {
    const actual = await vi.importActual<typeof import("../../compose/SmartChannelMapper")>("../../compose/SmartChannelMapper");
    const html = renderToStaticMarkup(createElement(actual.default, { mode: "calibration", files: [] }));
    const pickers = [...html.matchAll(/<button[^>]*class="ab-channel-browse"[^>]*>/g)].map((match) => match[0]);
    expect(pickers).toHaveLength(5);
    for (const [index, name] of ["Select Science file", "Select Bias frames", "Select Dark frames", "Select Flat frames", "Select Flat dark frames"].entries()) {
      expect(pickers[index]).toContain(`aria-label="${name}"`);
      expect(pickers[index]).toContain('aria-expanded="false"');
    }
  });

  it("distinguishes the four RGB pickers without relying on nearby visual labels", async () => {
    const actual = await vi.importActual<typeof import("../../compose/SmartChannelMapper")>("../../compose/SmartChannelMapper");
    const html = renderToStaticMarkup(createElement(actual.default, { mode: "rgb", files: [] }));
    for (const label of ["Luminance", "Red", "Green", "Blue"]) expect(html).toContain(`aria-label="Select ${label} file"`);
    expect(html.match(/aria-expanded="false"/g)).toHaveLength(4);
  });

  it("names the slot affected by clearing a selected science file or calibration frames", async () => {
    const actual = await vi.importActual<typeof import("../../compose/SmartChannelMapper")>("../../compose/SmartChannelMapper");
    const html = renderToStaticMarkup(createElement(actual.default, {
      mode: "calibration",
      files: [LIGHT, ...DARKS],
      calibAssignment: { science: LIGHT, bias: [], dark: DARKS, flat: [], flatdark: [] },
      onCalibAssignmentChange: vi.fn(),
    }));
    expect(html).toContain('aria-label="Clear Science slot"');
    expect(html).toContain('aria-label="Clear all 2 Dark frames"');
    expect(html).toContain('aria-label="Select Dark frames"');
  });

  it("offers a Flat dark slot after the Flat slot", async () => {
    const actual = await vi.importActual<typeof import("../../compose/SmartChannelMapper")>("../../compose/SmartChannelMapper");
    const html = renderToStaticMarkup(createElement(actual.default, { mode: "calibration", files: [] }));
    const labels = [...html.matchAll(/class="ab-channel-name">([^<]*)</g)].map((m) => m[1]);
    expect(labels).toEqual(["Science", "Bias", "Dark", "Flat", "Flat dark"]);
  });
});

function framesOf(kind: string, imagetyp: string, count: number): ProcessedFile[] {
  return Array.from({ length: count }, (_, i) => processed(`${kind}_${String(i + 1).padStart(2, "0")}.fits`, { IMAGETYP: imagetyp }));
}

const SX12_LIGHTS = framesOf("light", "LIGHT", 6);
const SX12_DARKS = framesOf("dark", "DARK", 10);
const SX12_FLATS = framesOf("flat", "FLAT", 5);
const SX12_BIAS = framesOf("bias", "BIAS", 3);
const SX12 = [...SX12_LIGHTS, ...SX12_DARKS, ...SX12_FLATS, ...SX12_BIAS];
const NO_FRAMES: CalibAssignment = { science: null, bias: [], dark: [], flat: [], flatdark: [] };

type Mapper = typeof import("../../compose/SmartChannelMapper");

function actualMapper(): Promise<Mapper> {
  return vi.importActual<Mapper>("../../compose/SmartChannelMapper");
}

function sessionChannelFiles(): ChannelFile[] {
  renderToStaticMarkup(createElement(CalibrationPanel, { files: SX12 }));
  return captured.files;
}

function slotRows(html: string): string[] {
  return html.split('class="ab-channel-slot-wrapper"').slice(1);
}

function listedNames(row: string): string[] {
  return [...row.matchAll(/class="ab-channel-filename[^"]*"[^>]*>([^<]*)</g)].map((m) => m[1]);
}

function addAllButtons(html: string): { label: string; disabled: boolean }[] {
  return [...html.matchAll(/<button([^>]*)>(Add all [^<]*)<\/button>/g)].map((m) => ({ label: m[2], disabled: m[1].includes('disabled=""') }));
}

describe("Calibration mapper frame lists", () => {
  it("hands the mapper the IMAGETYP kind of every frame", () => {
    expect(sessionChannelFiles().map((f) => f.frameKind)).toEqual([
      ...SX12_LIGHTS.map(() => "light"),
      ...SX12_DARKS.map(() => "dark"),
      ...SX12_FLATS.map(() => "flat"),
      ...SX12_BIAS.map(() => "bias"),
    ]);
  });

  it("Add all darks adds every IMAGETYP=DARK file", async () => {
    const mapper = await actualMapper();
    const files = sessionChannelFiles();
    const html = renderToStaticMarkup(createElement(mapper.default, { mode: "calibration", files }));
    expect(addAllButtons(html)).toEqual([
      { label: "Add all bias frames (3)", disabled: false },
      { label: "Add all dark frames (10)", disabled: false },
      { label: "Add all flat frames (5)", disabled: false },
    ]);
    const withDarks = withAllFramesOfKind(NO_FRAMES, "dark", files);
    expect(withDarks.dark.map((f) => f.path)).toEqual(SX12_DARKS.map((f) => f.path));
    expect({ ...withDarks, dark: [] }).toEqual(NO_FRAMES);
    expect(withAllFramesOfKind(withDarks, "dark", files)).toBe(withDarks);
    const after = renderToStaticMarkup(
      createElement(mapper.default, { mode: "calibration", files, calibAssignment: withDarks, onCalibAssignmentChange: vi.fn() }),
    );
    expect(listedNames(slotRows(after)[2])).toEqual(SX12_DARKS.map((f) => f.name));
    expect(addAllButtons(after)).toContainEqual({ label: "Add all dark frames (10)", disabled: true });
  });

  it("lists the frames of a row with a remove button each and collapses those after the fifth", async () => {
    const mapper = await actualMapper();
    const files = sessionChannelFiles();
    const sevenDarks = files.filter((f) => f.frameKind === "dark").slice(0, 7);
    const html = renderToStaticMarkup(
      createElement(mapper.default, { mode: "calibration", files, calibAssignment: { ...NO_FRAMES, dark: sevenDarks }, onCalibAssignmentChange: vi.fn() }),
    );
    const darkRow = slotRows(html)[2];
    expect(darkRow).toContain(">7 frames<");
    expect(listedNames(darkRow)).toEqual(sevenDarks.map((f) => f.name));
    const [shown, collapsed = ""] = darkRow.split("<details");
    expect(listedNames(shown)).toHaveLength(5);
    expect(collapsed).toMatch(/<summary[^>]*>2 more<\/summary>/);
    for (const f of sevenDarks) expect(darkRow).toContain(`aria-label="Remove ${f.name} from Dark"`);
  });

  it("toggles a frame picked from the menu in and out of its row", async () => {
    const dark = sessionChannelFiles().find((f) => f.frameKind === "dark");
    if (!dark) throw new Error("no dark frame");
    const picked = toggleCalibFrame(NO_FRAMES, "dark", dark);
    expect(picked).toEqual({ ...NO_FRAMES, dark: [dark] });
    expect(toggleCalibFrame(picked, "dark", dark)).toEqual(NO_FRAMES);
  });
});

const SX12_MASTERS: CalibrationMasters = {
  darkPaths: SX12_DARKS.map((f) => f.path),
  flatPaths: SX12_FLATS.map((f) => f.path),
  biasPaths: SX12_BIAS.map((f) => f.path),
  flatDarkPaths: [],
};

const NO_CALIBRATION: CalibrationState = {
  calibratedPath: null,
  calibratedFitsPath: null,
  hasBias: false,
  hasDark: false,
  hasFlat: false,
  darkPaths: [],
  flatPaths: [],
  biasPaths: [],
  flatDarkPaths: [],
};

type HandoffButton = ReactElement<{ disabled?: boolean; onClick: () => void; children?: ReactNode }>;

function buttonIn(node: unknown): HandoffButton | null {
  if (!node || typeof node !== "object" || !("type" in node)) return null;
  const el = node as ReactElement<{ children?: unknown }>;
  if (el.type === "button") return el as unknown as HandoffButton;
  for (const child of [el.props.children].flat()) {
    const found = buttonIn(child);
    if (found) return found;
  }
  return null;
}

describe("Calibration hand-off to the Pipeline", () => {
  beforeEach(() => {
    calibrateMock.mockReset();
    runPipelineMock.mockReset();
  });

  it("says the panel calibrates one light and points to the Pipeline", () => {
    renderToStaticMarkup(createElement(CalibrationPanel, { files: SX12, onUseInPipeline: vi.fn() }));
    const html = renderToStaticMarkup(captured.note as ReactElement);
    expect(CALIBRATE_SCOPE_NOTE).toBe("Calibrates one light; for a whole session use Pipeline");
    expect(html).toContain(CALIBRATE_SCOPE_NOTE);
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Use these frames in Pipeline<\/button>/);
  });

  it("Use these frames in Pipeline seeds the Pipeline rows without a run", () => {
    const onUse = vi.fn();
    const button = buttonIn(PipelineHandoff({ masters: SX12_MASTERS, onUse }));
    if (!button) throw new Error("no hand-off button");
    expect(button.props.disabled).toBe(false);
    button.props.onClick();
    expect(onUse).toHaveBeenCalledWith(SX12_MASTERS);
    const seeded = withCalibrationFrames(NO_CALIBRATION, onUse.mock.calls[0][0] as CalibrationMasters);
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files: SX12_LIGHTS, calibration: seeded }));
    expect(html).toContain(">Darks: 10<");
    expect(html).toContain(">Flats: 5<");
    expect(html).toContain(">Bias: 3<");
    expect(html).toContain(">Flat darks: 0<");
    expect(calibrateMock).not.toHaveBeenCalled();
    expect(runPipelineMock).not.toHaveBeenCalled();
  });

  it("keeps the last Calibrate run's output when it hands the frames over", () => {
    const calibrated: CalibrationState = {
      ...NO_CALIBRATION,
      calibratedPath: "asset://light.png",
      calibratedFitsPath: "C:/out/light_calibrated.fits",
      hasDark: true,
      darkPaths: ["C:/heavy/dark_old.fits"],
    };
    expect(withCalibrationFrames(calibrated, SX12_MASTERS)).toEqual({ ...calibrated, ...SX12_MASTERS });
  });
});
