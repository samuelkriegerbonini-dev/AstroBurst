import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { renderToStaticMarkup } from "react-dom/server";

const { compositeFlag } = vi.hoisted(() => ({ compositeFlag: { value: false } }));

vi.mock("../../../hooks/useCompositeMode", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../hooks/useCompositeMode")>()),
  useCompositeMode: () => compositeFlag.value,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { compositeChainStore } from "../../../utils/compositeChainStore";
import { withStep } from "../../../utils/processingChain";
import ProcessingTab from "../ProcessingTab";
import type { CompositeChain, CompositeChainEntry, CompositeDisplayed } from "../../../shared/types/compositeChain";
import type { ChainStep } from "../../../shared/types/preview";
import type { ProcessedFile } from "../../../shared/types";

const LHE_NOTE = "No stretch step in this chain: LHE on linear data equalizes the noise floor; Stretch or Masked first gives the usual result.";
const HDR_NOTE = "No stretch step in this chain: HDRMT refuses images whose median is under 1 % of the maximum; Stretch or Masked first.";
const CURVES_NOTE = "No stretch step in this chain: if the input is linear, Curves applies an automatic STF first, then your levels and curve.";
const LHE_COMPOSITE = "The composite input is linear: LHE runs only on a stretched composite; Stretch or Masked first.";
const HDR_COMPOSITE = "The composite input is linear: HDRMT runs only on a stretched composite; Stretch or Masked first.";
const RAW = "C:/data/673nmos.fits";
const STF = { shadow: 0, midtone: 0.5, highlight: 1 };

function fileOf(id: string, isRgb = false): ProcessedFile {
  return {
    id,
    name: "673nmos.fits",
    path: RAW,
    sourcePath: RAW,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/673nmos.png", previewUrl: "asset://C:/out/673nmos.png", dimensions: [64, 64], elapsed_ms: 1, is_rgb: isRgb },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function showOutputs(file: ProcessedFile, steps: ChainStep[]): void {
  const key = fileKeyOf(file) as string;
  function Publisher() {
    const { publishProcessed } = useRenderActions();
    const last = steps[steps.length - 1];
    const entryOf = (step: ChainStep) => ({ fitsPath: `C:/app/history/${step}.fits`, previewUrl: `asset://out/${step}.png`, dimensions: [64, 64] as [number, number] });
    publishProcessed(
      key,
      { ...entryOf(last), label: last, kind: "processing", inputPath: RAW },
      (c) => steps.reduce((chain, step) => withStep(chain, step, entryOf(step)), c),
    );
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function entry(label: string, displayed: CompositeDisplayed): CompositeChainEntry {
  return { previewUrl: `asset://out/${label.toLowerCase()}.png`, label, displayed };
}

function compositeChainOf(steps: Partial<Record<ChainStep, CompositeChainEntry>>): CompositeChain {
  return { generation: 2, base: { previewUrl: "asset://out/blend.png", stf: { r: STF, g: STF, b: STF, linked: true } }, steps, psfKernel: null };
}

async function renderTab(file: ProcessedFile): Promise<string> {
  const tree = createElement(CompositeProvider, {
    children: createElement(ComposeWizardProvider, {
      children: createElement(PreviewProvider, { file, doneFiles: [file], children: createElement(ProcessingTab) }),
    }),
  });
  const { prelude } = await prerender(tree);
  return await new Response(prelude).text();
}

function inputNotes(html: string): { id: string; text: string }[] {
  return [...html.matchAll(/<div id="([^"]+)" role="note" data-non-linear-input=""[^>]*>([^<]*)<\/div>/g)].map((m) => ({ id: m[1], text: m[2] }));
}

function describedBy(html: string, label: string): string[] {
  for (const match of html.matchAll(/<button([^>]*class="ab-run-btn[^"]*"[^>]*)>([\s\S]*?)<\/button>/g)) {
    if (match[2].replace(/<[^>]*>/g, "").trim().startsWith(label)) return (/aria-describedby="([^"]*)"/.exec(match[1])?.[1] ?? "").split(" ");
  }
  throw new Error(`no "${label}" run button`);
}

function noteId(html: string, text: string): string | undefined {
  return inputNotes(html).find((n) => n.text === text)?.id;
}

afterEach(() => {
  compositeFlag.value = false;
  compositeChainStore.reset();
});

describe("ProcessingTab says before Run when no stretch precedes LHE, HDRMT and Curves", () => {
  it("on the original, gives each tool its note and links its Run button to it", async () => {
    const html = await renderTab(fileOf("nl-original"));
    expect(inputNotes(html).map((n) => n.text)).toEqual([LHE_NOTE, HDR_NOTE, CURVES_NOTE]);
    expect(describedBy(html, "Run Local Contrast")).toContain(noteId(html, LHE_NOTE));
    expect(describedBy(html, "Run HDRMT")).toContain(noteId(html, HDR_NOTE));
    expect(describedBy(html, "Apply Curves")).toContain(noteId(html, CURVES_NOTE));
    expect(describedBy(html, "Run HDRMT")).not.toContain(noteId(html, LHE_NOTE));
  });

  it("keeps the notes after the linear steps", async () => {
    const file = fileOf("nl-linear");
    showOutputs(file, ["background", "denoise", "deconv"]);
    expect(inputNotes(await renderTab(file)).map((n) => n.text)).toEqual([LHE_NOTE, HDR_NOTE, CURVES_NOTE]);
  });

  it.each([["stretch"], ["maskedStretch"]] as [ChainStep][])("drops every note once %s is in the chain", async (step) => {
    const file = fileOf(`nl-${step}`);
    showOutputs(file, ["background", step]);
    expect(inputNotes(await renderTab(file))).toEqual([]);
  });

  it("keeps the Curves note after LHE on the linear original", async () => {
    const file = fileOf("nl-lhe-linear");
    showOutputs(file, ["localContrast"]);
    expect(inputNotes(await renderTab(file)).map((n) => n.text)).toEqual([LHE_NOTE, HDR_NOTE, CURVES_NOTE]);
  });

  it("on an RGB FITS, keeps only the Curves note, since LHE and HDRMT cannot run", async () => {
    expect(inputNotes(await renderTab(fileOf("nl-rgb", true))).map((n) => n.text)).toEqual([CURVES_NOTE]);
  });
});

describe("ProcessingTab on the composite", () => {
  it("says definitely that LHE and HDRMT need a stretched composite when their input is linear", async () => {
    compositeFlag.value = true;
    compositeChainStore.set(compositeChainOf({ background: entry("Background", "linear") }));
    const html = await renderTab(fileOf("nl-composite-linear"));
    expect(inputNotes(html).map((n) => n.text)).toEqual([LHE_COMPOSITE, HDR_COMPOSITE]);
    expect(describedBy(html, "Run Local Contrast")).toContain(noteId(html, LHE_COMPOSITE));
  });

  it("says nothing on a stretched composite input", async () => {
    compositeFlag.value = true;
    compositeChainStore.set(compositeChainOf({ stretch: entry("Stretch", "stretched") }));
    expect(inputNotes(await renderTab(fileOf("nl-composite-stretched")))).toEqual([]);
  });

  it("says nothing on the composite base, whose tier the frontend does not hold", async () => {
    compositeFlag.value = true;
    compositeChainStore.set(compositeChainOf({}));
    expect(inputNotes(await renderTab(fileOf("nl-composite-base")))).toEqual([]);
  });
});
