import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";

const { initialSection } = vi.hoisted(() => ({ initialSection: { value: null as string | null } }));

vi.mock("../../../hooks/useCompositeMode", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../hooks/useCompositeMode")>()),
  useCompositeMode: () => true,
}));

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  const useState = (init: unknown) =>
    actual.useState(() => {
      const value = typeof init === "function" ? (init as () => unknown)() : init;
      return value === "background" && initialSection.value !== null ? initialSection.value : value;
    });
  return { ...actual, useState };
});

import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import { compositeChainStore } from "../../../utils/compositeChainStore";
import ProcessingTab from "../ProcessingTab";
import type { CompositeChain, CompositeChainEntry, CompositeDisplayed } from "../../../shared/types/compositeChain";
import type { ChainStep } from "../../../shared/types/preview";
import type { ProcessedFile } from "../../../shared/types";

const FILE: ProcessedFile = {
  id: "composite-note",
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

const STF = { shadow: 0, midtone: 0.5, highlight: 1 };

function entry(label: string, displayed: CompositeDisplayed): CompositeChainEntry {
  return { previewUrl: `asset://out/${label.toLowerCase().replace(/ /g, "_")}.png`, label, displayed };
}

function chainOf(steps: Partial<Record<ChainStep, CompositeChainEntry>>): CompositeChain {
  return { generation: 3, base: { previewUrl: "asset://out/blend.png", stf: { r: STF, g: STF, b: STF, linked: true } }, steps, psfKernel: null };
}

async function renderTab(section: string | null = null): Promise<string> {
  initialSection.value = section;
  try {
    const tree = createElement(CompositeProvider, {
      children: createElement(ComposeWizardProvider, {
        children: createElement(PreviewProvider, { file: FILE, doneFiles: [FILE], children: createElement(ProcessingTab) }),
      }),
    });
    const { prelude } = await prerender(tree);
    return await new Response(prelude).text();
  } finally {
    initialSection.value = null;
  }
}

function replacedNote(html: string): { id: string; text: string } | null {
  const match = /<div id="([^"]+)" role="note" data-replaced-steps=""[^>]*>([\s\S]*?)<\/div>/.exec(html);
  return match ? { id: match[1], text: match[2].replace(/<!-- -->/g, "") } : null;
}

function crumbLabels(html: string): string[] {
  const bar = /<div class="flex items-center gap-1 px-4 py-1\.5[^"]*">([\s\S]*?)<\/div>/.exec(html)?.[1] ?? "";
  return [...bar.matchAll(/<(?:span|button)[^>]*class="(?:text-emerald-400\/80|text-zinc-500)[^"]*"[^>]*>([^<]*)<\/(?:span|button)>/g)].map((m) => m[1]);
}

function runDescribedBy(html: string, label: string): string[] {
  for (const match of html.matchAll(/<button([^>]*class="ab-run-btn[^"]*"[^>]*)>([\s\S]*?)<\/button>/g)) {
    if (match[2].replace(/<[^>]*>/g, "").trim().startsWith(label)) return (/aria-describedby="([^"]*)"/.exec(match[1])?.[1] ?? "").split(" ");
  }
  throw new Error(`no "${label}" run button`);
}

afterEach(() => {
  compositeChainStore.reset();
});

describe("ProcessingTab on the composite says which steps a run replaces", () => {
  it("names the steps after Background and says the composite cannot bring them back", async () => {
    compositeChainStore.set(chainOf({ denoise: entry("Denoised", "linear"), stretch: entry("Stretch", "stretched") }));
    const html = await renderTab();
    const note = replacedNote(html);
    expect(note?.text).toBe(
      "Running Background Extraction replaces Wavelet Denoise and Arcsinh Stretch on the composite; the composite has no step history yet, so they cannot be brought back.",
    );
    expect(runDescribedBy(html, "Extract Background")).toContain(note?.id);
  });

  it("says again when the composite already holds a Background result", async () => {
    compositeChainStore.set(chainOf({ background: entry("Background Removed", "linear"), denoise: entry("Denoised", "linear") }));
    expect(replacedNote(await renderTab())?.text).toBe(
      "Running Background Extraction again replaces Wavelet Denoise on the composite; the composite has no step history yet, so it cannot be brought back.",
    );
  });

  it("says that LHE and HDRMT share one slot", async () => {
    compositeChainStore.set(chainOf({ stretch: entry("Stretch", "stretched"), localContrast: entry("HDRMT", "stretched") }));
    expect(replacedNote(await renderTab("local_contrast"))?.text).toBe(
      "LHE and HDRMT share one slot: this run starts from Arcsinh Stretch and replaces HDRMT; the composite has no step history yet, so it cannot be brought back.",
    );
    expect(replacedNote(await renderTab("hdr"))).toBeNull();
  });

  it("says nothing for a tool that does not run on the composite", async () => {
    compositeChainStore.set(chainOf({ stretch: entry("Stretch", "stretched"), pixelMath: entry("PixelMath", "toned") }));
    expect(replacedNote(await renderTab("curves"))).toBeNull();
  });
});

describe("ProcessingTab on the composite names its crumbs", () => {
  it("names the local contrast crumb by the tool its entry records", async () => {
    compositeChainStore.set(chainOf({ background: entry("Background Removed", "linear"), stretch: entry("Stretch", "stretched"), localContrast: entry("HDRMT", "stretched") }));
    expect(crumbLabels(await renderTab())).toEqual(["Composite", "BG", "Stretch", "HDRMT"]);
    compositeChainStore.set(chainOf({ stretch: entry("Stretch", "stretched"), localContrast: entry("LHE", "stretched") }));
    expect(crumbLabels(await renderTab())).toEqual(["Composite", "Stretch", "LHE"]);
  });

  it("names a DBE background crumb by its model and keeps BG for the polynomial model", async () => {
    compositeChainStore.set(chainOf({ background: entry("DBE", "linear"), stretch: entry("Stretch", "stretched") }));
    expect(crumbLabels(await renderTab())).toEqual(["Composite", "DBE", "Stretch"]);
    compositeChainStore.set(chainOf({ background: entry("Background Removed", "linear"), stretch: entry("Stretch", "stretched") }));
    expect(crumbLabels(await renderTab())).toEqual(["Composite", "BG", "Stretch"]);
  });
});
