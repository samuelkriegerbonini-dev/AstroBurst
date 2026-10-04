import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { captured, runPixelMathMock } = vi.hoisted(() => ({
  captured: { onClick: null as null | (() => void) },
  runPixelMathMock: vi.fn(),
}));

vi.mock("../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui")>();
  return {
    ...actual,
    RunButton: (props: { onClick: () => void }) => {
      captured.onClick = props.onClick;
      return null;
    },
  };
});

vi.mock("../../../services/pixelmath", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/pixelmath")>()),
  runPixelMath: runPixelMathMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { fileStore } from "../../../hooks/useFileStore";
import { registerFileIngest } from "../../../hooks/useFileIngest";
import { INPUT_CHANGED_MESSAGE } from "../../../hooks/useProcessingRun";
import { withStep } from "../../../utils/processingChain";
import { rebindUntouchedSlots } from "../../../utils/pixelmathSlots";
import PixelMathPanel from "../PixelMathPanel";
import { retain, retainedFor } from "../pixelMathPanelState";
import type { AstroFile, ProcessedFile } from "../../../shared/types";
import type { PixelMathResult, PixelMathSlot } from "../../../shared/types/pixelmath";

const SOURCE = "C:/data/673nmos.fits";
const OUTPUT = "C:/out/673nmos_pixelmath.fits";
const BG = "C:/out/673nmos_bg.fits";
const LOADED_502 = "C:/data/502nmos.fits";
const LOADED_656 = "C:/data/656nmos.fits";

function sourceFile(id: string): ProcessedFile {
  return {
    id,
    name: "673nmos.fits",
    path: SOURCE,
    sourcePath: SOURCE,
    imageRef: null,
    size: 1,
    status: "done",
    result: null,
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function pixelMathResult(fitsPath: string): PixelMathResult {
  return {
    png_path: fitsPath.replace(/\.fits$/, ".png"),
    fits_path: fitsPath,
    previewUrl: "asset://out/673nmos_pixelmath.png",
    dimensions: [8, 8],
    elapsed_ms: 1,
    stats: null,
    non_finite_count: 0,
  };
}

function renderPanel(file: ProcessedFile, onProcessingDone: (res: PixelMathResult) => void): string {
  const panel = createElement(PixelMathPanel, {
    selectedFile: file,
    outputDir: "C:/out",
    fileKey: fileKeyOf(file),
    compositeMode: false,
    compositeInput: null,
    onCompositeDone: () => {},
    onProcessingDone,
    fileName: file.name,
  });
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: panel });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function showBackgroundOutput(file: ProcessedFile): void {
  const key = fileKeyOf(file) as string;
  function Publisher() {
    const { publishProcessed } = useRenderActions();
    publishProcessed(
      key,
      { fitsPath: BG, previewUrl: "asset://out/673nmos_bg.png", dimensions: [8, 8], label: "Background", kind: "processing", inputPath: SOURCE },
      (c) => withStep(c, "background", { fitsPath: BG, previewUrl: "asset://out/673nmos_bg.png", dimensions: [8, 8] }),
    );
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function seedPanel(file: ProcessedFile, expression: string, slots: PixelMathSlot[]): string {
  const key = fileKeyOf(file) as string;
  retain(key, { expression, slots, truncate: false, rescale: false, outputName: "", slotsTouched: false });
  return key;
}

function loadAndSelectSource(): string {
  fileStore.addFiles([{ name: "673nmos.fits", path: SOURCE, size: 0 }]);
  const id = fileStore.getFileIds()[fileStore.getFileIds().length - 1];
  fileStore.selectFile(id);
  return id;
}

describe("PixelMathPanel file-mode run and the Files list", () => {
  const ingest = vi.fn((files: AstroFile[]) => fileStore.addFiles(files));
  let unregister: () => void;

  beforeEach(() => {
    fileStore.reset();
    captured.onClick = null;
    ingest.mockClear();
    runPixelMathMock.mockReset();
    unregister = registerFileIngest(ingest);
  });

  afterEach(() => unregister());

  it("adds the result to the Files list after publishing it, once across re-runs, and keeps the selection", async () => {
    const selectedId = loadAndSelectSource();
    const file = sourceFile("pm-run-ingest");
    const onProcessingDone = vi.fn();
    runPixelMathMock.mockResolvedValue(pixelMathResult(OUTPUT));

    renderPanel(file, onProcessingDone);
    captured.onClick?.();
    await vi.waitFor(() => expect(ingest).toHaveBeenCalledTimes(1));
    expect(ingest).toHaveBeenCalledWith([{ name: "673nmos_pixelmath.fits", path: OUTPUT, size: 0 }], { quiet: true });
    expect(onProcessingDone).toHaveBeenCalledTimes(1);
    expect(onProcessingDone.mock.invocationCallOrder[0]).toBeLessThan(ingest.mock.invocationCallOrder[0]);

    for (const run of [2, 3]) {
      renderPanel(file, onProcessingDone);
      captured.onClick?.();
      await vi.waitFor(() => expect(onProcessingDone).toHaveBeenCalledTimes(run));
    }
    expect(ingest).toHaveBeenCalledTimes(1);
    expect(fileStore.getFiles().map((f) => f.path)).toEqual([SOURCE, OUTPUT]);
    expect(fileStore.getSelected()).toBe(selectedId);
  });

  it("neither publishes nor adds the result when the displayed image changed while it ran", async () => {
    loadAndSelectSource();
    const file = sourceFile("pm-run-displayed-changed");
    const key = seedPanel(file, "$T * (A / med(A))", [{ name: "A", path: SOURCE }]);
    const onProcessingDone = vi.fn();
    runPixelMathMock.mockImplementation(async () => {
      showBackgroundOutput(file);
      return pixelMathResult(OUTPUT);
    });

    renderPanel(file, onProcessingDone);
    captured.onClick?.();
    await vi.waitFor(() => expect(renderPanel(file, onProcessingDone)).toContain(INPUT_CHANGED_MESSAGE));
    expect(onProcessingDone).not.toHaveBeenCalled();
    expect(ingest).not.toHaveBeenCalled();
    expect(fileStore.getFiles().map((f) => f.path)).toEqual([SOURCE]);
    expect(retainedFor(key)?.slotsTouched).toBe(false);
  });

  it.each([
    {
      example: "Average three images",
      expression: "(A + B + C) / 3",
      slots: [{ name: "A", path: LOADED_502 }, { name: "B", path: LOADED_656 }, { name: "C", path: SOURCE }],
      others: [LOADED_502, LOADED_656],
    },
    {
      example: "Flat-field by A",
      expression: "$T * (A / med(A))",
      slots: [{ name: "A", path: SOURCE }],
      others: [],
    },
  ])("keeps the slots of a successful run ($example) when its result joins the loaded files", async ({ expression, slots, others }) => {
    loadAndSelectSource();
    fileStore.addFiles(others.map((path) => ({ name: path.split("/").pop() ?? path, path, size: 0 })));
    const file = sourceFile(`pm-run-keeps-slots-${slots.length}`);
    const key = seedPanel(file, expression, slots);
    runPixelMathMock.mockResolvedValue(pixelMathResult(OUTPUT));

    renderPanel(file, vi.fn());
    captured.onClick?.();
    await vi.waitFor(() => expect(ingest).toHaveBeenCalledTimes(1));
    expect(runPixelMathMock).toHaveBeenCalledWith(SOURCE, "C:/out", expression, expect.objectContaining({ slots }));

    const loadedAfterRun = fileStore.getFiles().map((f) => ({ path: f.path }));
    expect(loadedAfterRun.map((f) => f.path)).toEqual([SOURCE, ...others, OUTPUT]);
    const saved = retainedFor(key);
    expect(rebindUntouchedSlots(saved?.slots ?? [], loadedAfterRun, SOURCE, saved?.slotsTouched ?? false, OUTPUT)).toEqual(slots);
  });
});
