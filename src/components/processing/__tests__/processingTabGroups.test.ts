import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import ProcessingTab from "../ProcessingTab";
import type { ProcessedFile } from "../../../shared/types";

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

async function renderTab(): Promise<string> {
  const tree = createElement(CompositeProvider, {
    children: createElement(ComposeWizardProvider, {
      children: createElement(PreviewProvider, { file: FILE, doneFiles: [FILE], children: createElement(ProcessingTab) }),
    }),
  });
  const { prelude } = await prerender(tree);
  return new Response(prelude).text();
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
