import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import ProcessingTab from "../ProcessingTab";
import type { ProcessedFile } from "../../../shared/types";
import type { ChainStep } from "../../../shared/types/preview";

const FILE: ProcessedFile = {
  id: "groups",
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

const GROUPS = [
  { caption: "Linear", sections: ["background", "denoise", "psf", "deconvolution"] },
  { caption: "Stretch", sections: ["stretch", "masked_stretch"] },
  { caption: "Non-linear", sections: ["local_contrast", "hdr", "curves"] },
  { caption: "Tools", sections: ["pixelmath", "geometry", "debayer"] },
];

const CARD_TITLES: Record<string, string> = {
  background: "Remove gradients and vignetting; on linear data, first",
  deconvolution: "Sharpen with Richardson-Lucy; on linear data",
  local_contrast: "Local contrast on a stretched image",
  debayer: "Colour-camera frames: debayer or extract CFA planes",
};

interface Pill {
  section: string | null;
  title: string | null;
  label: string;
}

async function renderTab(file: ProcessedFile = FILE): Promise<string> {
  const tree = createElement(CompositeProvider, {
    children: createElement(ComposeWizardProvider, {
      children: createElement(PreviewProvider, { file, doneFiles: [file], children: createElement(ProcessingTab) }),
    }),
  });
  const { prelude } = await prerender(tree);
  return new Response(prelude).text();
}

function publishStep(file: ProcessedFile, step: ChainStep): void {
  const key = fileKeyOf(file) as string;
  const entry = { fitsPath: `C:/out/${step}.fits`, previewUrl: null, dimensions: [64, 64] as [number, number] };
  function Publisher() {
    useRenderActions().setChain(key, (c) => ({ ...c, steps: { ...c.steps, [step]: entry } }));
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function pillTags(html: string): [string, string][] {
  return [...html.matchAll(/<button[^>]*data-processing-section="([^"]+)"[^>]*>/g)].map((m) => [m[1], m[0]]);
}

function pillContent(html: string, section: string): string {
  return new RegExp(String.raw`<button[^>]*data-processing-section="${section}"[^>]*>([\s\S]*?)</button>`).exec(html)?.[1] ?? "";
}

function pillsOf(html: string): Pill[] {
  return [...html.matchAll(/<button([^>]*class="ab-processing-pill[ "][^>]*)>([\s\S]*?)<\/button>/g)].map((m) => ({
    section: /data-processing-section="([^"]*)"/.exec(m[1])?.[1] ?? null,
    title: /title="([^"]*)"/.exec(m[1])?.[1] ?? null,
    label: m[2].replace(/<[^>]*>/g, ""),
  }));
}

function pillGroupsOf(html: string): { caption: string; sections: string[] }[] {
  const starts = [...html.matchAll(/<div role="group" aria-label="([^"]+)"[^>]*>/g)];
  return starts
    .map((m, i) => ({
      caption: m[1],
      sections: [...html.slice(m.index, starts[i + 1]?.index ?? html.length).matchAll(/data-processing-section="([^"]+)"/g)].map((s) => s[1]),
    }))
    .filter((g) => g.sections.length > 0);
}

describe("ProcessingTab tool groups", () => {
  it("groups the pills as Linear, Stretch, Non-linear and Tools, in workflow order", async () => {
    const html = await renderTab();
    expect(pillGroupsOf(html)).toEqual(GROUPS);
    const captions = [...html.matchAll(/<span aria-hidden="true"[^>]*>([^<]+)<\/span>/g)].map((m) => m[1]);
    expect(captions.slice(0, GROUPS.length)).toEqual(GROUPS.map((g) => g.caption));
  });

  it("marks every pill with its section, in the tab order of the groups", async () => {
    const pills = pillsOf(await renderTab());
    expect(pills.map((p) => p.section)).toEqual(GROUPS.flatMap((g) => g.sections));
  });

  it("titles each pill with when to use it instead of repeating the label", async () => {
    const pills = pillsOf(await renderTab());
    expect(pills).toHaveLength(12);
    for (const pill of pills) {
      expect(pill.title).toBeTruthy();
      expect(pill.title).not.toMatch(/processing step$/);
      expect(pill.title).not.toBe(pill.label);
    }
    for (const [section, title] of Object.entries(CARD_TITLES)) {
      expect(pills.find((p) => p.section === section)?.title).toBe(title);
    }
  });
});

describe("ProcessingTab section state for assistive technology", () => {
  it("marks only the open section as current, without toggle-button semantics", async () => {
    const tags = pillTags(await renderTab());
    expect(tags).toHaveLength(12);
    expect(tags.filter(([, tag]) => tag.includes('aria-current="true"')).map(([section]) => section)).toEqual(["background"]);
    expect(tags.filter(([, tag]) => tag.includes("aria-pressed"))).toEqual([]);
  });

  it("says in text that a section has a result, next to the dot that only shows it", async () => {
    const file = { ...FILE, id: "groups-result-dot" };
    publishStep(file, "background");
    const html = await renderTab(file);
    const background = pillContent(html, "background");
    expect(background).toMatch(/<span class="ab-processing-pill-dot[^"]*" title="Has a result in this chain"><\/span>/);
    expect(background).toContain('<span class="sr-only">, has a result in this chain</span>');
    expect(pillContent(html, "denoise")).not.toContain("sr-only");
  });
});
