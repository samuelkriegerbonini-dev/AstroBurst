import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { renderToStaticMarkup } from "react-dom/server";

interface ToneDone {
  fits_path: string;
  previewUrl: string;
  dimensions: number[];
}

const { captured, initialSection } = vi.hoisted(() => ({
  captured: {
    selectedFile: null as null | { path: string },
    onProcessingDone: null as null | ((res: ToneDone) => void),
  },
  initialSection: { value: null as string | null },
}));

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  const useState = (init: unknown) =>
    actual.useState(init === "background" && initialSection.value !== null ? initialSection.value : init);
  return { ...actual, useState };
});

vi.mock("../CurvesPanel", () => ({
  default: (props: { selectedFile: { path: string } | null; onProcessingDone?: (res: ToneDone) => void }) => {
    captured.selectedFile = props.selectedFile;
    captured.onProcessingDone = props.onProcessingDone ?? null;
    return null;
  },
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider, fileKeyOf, getRenderRecord, useRenderActions } from "../../../context/PreviewContext";
import { withStep } from "../../../utils/processingChain";
import ProcessingTab from "../ProcessingTab";
import { RGB_FITS_NOTICE } from "../rgbFitsNotice";
import type { ProcessedFile } from "../../../shared/types";

const RAW = "C:/data/wf_502nmos.fits";
const STRETCH = "C:/out/wf_502nmos_stretch.fits";
const TONE = "C:/out/wf_502nmos_stretch_tone.fits";

function fileOf(id: string, isRgb = false): ProcessedFile {
  return {
    id,
    name: "wf_502nmos.fits",
    path: RAW,
    sourcePath: RAW,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/wf_502nmos.png", previewUrl: "asset://C:/out/wf_502nmos.png", dimensions: [8, 8], elapsed_ms: 1, is_rgb: isRgb },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function showStretchOutput(key: string): void {
  function Publisher() {
    const { publishProcessed } = useRenderActions();
    publishProcessed(
      key,
      { fitsPath: STRETCH, previewUrl: "asset://out/stretch.png", dimensions: [8, 8], label: "Stretch", kind: "processing", inputPath: RAW },
      (c) => withStep(c, "stretch", { fitsPath: STRETCH, previewUrl: "asset://out/stretch.png", dimensions: [8, 8] }),
    );
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

async function renderTab(file: ProcessedFile, section: string | null = null): Promise<string> {
  initialSection.value = section;
  try {
    const tree = createElement(CompositeProvider, {
      children: createElement(ComposeWizardProvider, {
        children: createElement(PreviewProvider, { file, doneFiles: [file], children: createElement(ProcessingTab) }),
      }),
    });
    const { prelude } = await prerender(tree);
    return await new Response(prelude).text();
  } finally {
    initialSection.value = null;
  }
}

function takeOnProcessingDone(): (res: ToneDone) => void {
  const done = captured.onProcessingDone;
  if (!done) throw new Error("ProcessingTab did not render the Curves panel");
  return done;
}

function activePill(html: string): string | null {
  return /class="ab-processing-pill bg-[a-z]+-600\/20[^"]*" title="([^"]+) processing step"/.exec(html)?.[1] ?? null;
}

function tabNote(html: string): string | null {
  return /<div id="[^"]+" role="note"[^>]*>([^<]*)<\/div>/.exec(html)?.[1] ?? null;
}

beforeEach(() => {
  captured.selectedFile = null;
  captured.onProcessingDone = null;
});

describe("ProcessingTab Curves wiring", () => {
  it("hands Curves the stretch output as its input", async () => {
    const file = fileOf("tone-input");
    showStretchOutput(fileKeyOf(file) as string);
    await renderTab(file);
    expect(captured.selectedFile?.path).toBe(STRETCH);
  });

  it("publishes a Curves run as the tone step, recorded as read from the stretch output", async () => {
    const file = fileOf("tone-publish");
    const key = fileKeyOf(file) as string;
    showStretchOutput(key);
    await renderTab(file);
    takeOnProcessingDone()({ fits_path: TONE, previewUrl: "asset://out/tone.png", dimensions: [8, 8] });
    const record = getRenderRecord(key);
    expect(record?.chain.steps.tone?.fitsPath).toBe(TONE);
    expect(record?.chain.steps.stretch?.fitsPath).toBe(STRETCH);
    expect(record?.processed).toMatchObject({ fitsPath: TONE, label: "Curves", inputPath: STRETCH });
  });
});

describe("ProcessingTab RGB note", () => {
  it.each([
    ["curves", "Curves"],
    ["geometry", "Geometry"],
  ])("is not shown while %s, which handles RGB planes, is active", async (section, pill) => {
    const html = await renderTab(fileOf(`rgb-${section}`, true), section);
    expect(activePill(html)).toBe(pill);
    expect(tabNote(html)).toBeNull();
  });

  it("is still shown on a section that cannot handle RGB planes", async () => {
    const html = await renderTab(fileOf("rgb-debayer", true), "debayer");
    expect(activePill(html)).toBe("Debayer");
    expect(tabNote(html)).toBe(RGB_FITS_NOTICE);
  });
});
