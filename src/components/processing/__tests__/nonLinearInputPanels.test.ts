import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";

interface CapturedRun {
  label: string;
  onClick: () => void;
  describedBy?: string;
  disabled?: boolean;
}

const { runButtons, recordHdrmtMock, applyHdrmtCompositeMock } = vi.hoisted(() => ({
  runButtons: new Map<string, CapturedRun>(),
  recordHdrmtMock: vi.fn(),
  applyHdrmtCompositeMock: vi.fn(),
}));

vi.mock("../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui")>();
  return {
    ...actual,
    RunButton: (props: CapturedRun) => {
      runButtons.set(props.label, props);
      return null;
    },
  };
});

vi.mock("../../../services/localContrast", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/localContrast")>()),
  recordHdrmt: recordHdrmtMock,
  applyHdrmtComposite: applyHdrmtCompositeMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import LocalContrastPanel from "../LocalContrastPanel";
import HdrPanel from "../HdrPanel";
import { RGB_FITS_NOTICE } from "../rgbFitsNotice";
import type { CompositeInputView } from "../compositeProps";
import type { ProcessedFile } from "../../../shared/types";

const LHE_NOTE = "No stretch step in this chain: LHE on linear data equalizes the noise floor; Stretch or Masked first gives the usual result.";
const HDR_NOTE = "No stretch step in this chain: HDRMT refuses images whose median is under 1 % of the maximum; Stretch or Masked first.";
const HDR_LINEAR_ERROR = "HDRMT expects a non-linear (stretched) image; median is below 1% of the maximum";
const OLD_HINT = "Arcsinh, GHS or Masked";
const FILE_HINT = "HDRMT works on stretched data. Run Stretch (Arcsinh or GHS) or Masked in this tab, then run HDRMT on its output.";
const COMPOSITE_HINT = "HDRMT works on stretched data. Run Stretch or Masked in this tab, or GHS in Compose › Stretch, then run HDRMT on its output.";
const COMPOSITE_INPUT: CompositeInputView = { input: "background", previewUrl: "asset://out/background.png", label: "Background" };

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "673nmos.fits",
    path: "C:/data/673nmos.fits",
    sourcePath: "C:/data/673nmos.fits",
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/673nmos.png", previewUrl: "asset://C:/out/673nmos.png", dimensions: [64, 64], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

interface PanelOptions {
  nonLinearNote?: string | null;
  disabledReason?: string | null;
  compositeMode?: boolean;
  selected?: boolean;
}

function render(component: ComponentType<never>, file: ProcessedFile, options: PanelOptions = {}): string {
  const compositeMode = options.compositeMode ?? false;
  const props = {
    selectedFile: options.selected === false ? null : file,
    outputDir: "C:/out",
    fileKey: `${file.id}|${file.path}`,
    compositeMode,
    compositeInput: compositeMode ? COMPOSITE_INPUT : null,
    onCompositeDone: () => {},
    fileName: file.name,
    disabledReason: options.disabledReason ?? null,
    disabledReasonId: "rgb-note",
    nonLinearNote: options.nonLinearNote ?? null,
  };
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: createElement(component, props as never) });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function inputNote(html: string): { id: string; text: string } | null {
  const match = /<div id="([^"]+)" role="note" data-non-linear-input=""[^>]*>([^<]*)<\/div>/.exec(html);
  return match ? { id: match[1], text: match[2] } : null;
}

function run(label: string): CapturedRun {
  const button = runButtons.get(label);
  if (!button) throw new Error(`"${label}" not rendered`);
  return button;
}

beforeEach(() => {
  runButtons.clear();
  recordHdrmtMock.mockReset();
  applyHdrmtCompositeMock.mockReset();
});

describe.each([
  { name: "LocalContrastPanel", component: LocalContrastPanel as ComponentType<never>, label: "Run Local Contrast", note: LHE_NOTE },
  { name: "HdrPanel", component: HdrPanel as ComponentType<never>, label: "Run HDRMT", note: HDR_NOTE },
])("$name before Run", ({ component, label, note }) => {
  it("shows the non-linear input note and describes Run with it", () => {
    const html = render(component, fileOf(`${label}-note`), { nonLinearNote: note });
    const shown = inputNote(html);
    expect(shown?.text).toBe(note);
    expect(run(label).describedBy?.split(" ")).toContain(shown?.id);
  });

  it("shows nothing without a note", () => {
    const html = render(component, fileOf(`${label}-none`));
    expect(inputNote(html)).toBeNull();
    expect(run(label).describedBy).toBeUndefined();
  });

  it("hides the note while Run is unavailable", () => {
    const rgb = render(component, fileOf(`${label}-rgb`), { nonLinearNote: note, disabledReason: RGB_FITS_NOTICE });
    expect(inputNote(rgb)).toBeNull();
    expect(run(label).describedBy).toBe("rgb-note");
    const empty = render(component, fileOf(`${label}-empty`), { nonLinearNote: note, selected: false });
    expect(inputNote(empty)).toBeNull();
  });
});

describe("HdrPanel after a run refused on linear data", () => {
  it("on a file, points to Stretch (Arcsinh or GHS) or Masked in this tab and not to Compose", async () => {
    const file = fileOf("hdr-linear-file");
    recordHdrmtMock.mockRejectedValue(new Error(HDR_LINEAR_ERROR));
    render(HdrPanel as ComponentType<never>, file);
    run("Run HDRMT").onClick();
    await vi.waitFor(() => expect(render(HdrPanel as ComponentType<never>, file)).toContain(HDR_LINEAR_ERROR));
    const html = render(HdrPanel as ComponentType<never>, file);
    expect(html).not.toContain(OLD_HINT);
    expect(html).toContain(FILE_HINT);
    expect(html).not.toContain("Compose › Stretch");
  });

  it("on the composite, adds GHS in Compose › Stretch", async () => {
    const file = fileOf("hdr-linear-composite");
    applyHdrmtCompositeMock.mockRejectedValue(new Error(HDR_LINEAR_ERROR));
    render(HdrPanel as ComponentType<never>, file, { compositeMode: true });
    run("Run HDRMT").onClick();
    await vi.waitFor(() => expect(render(HdrPanel as ComponentType<never>, file, { compositeMode: true })).toContain(HDR_LINEAR_ERROR));
    const html = render(HdrPanel as ComponentType<never>, file, { compositeMode: true });
    expect(html).not.toContain(OLD_HINT);
    expect(html).toContain(COMPOSITE_HINT);
  });
});
