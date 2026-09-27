import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import ProcessingTab from "../ProcessingTab";
import { RGB_FITS_NOTICE } from "../rgbFitsNotice";
import type { ProcessedFile } from "../../../shared/types";

const PANEL_RUN_LABELS = [
  "Debayer Current File",
  "Extract Background",
  "Run Noise Reduction",
  "Estimate PSF",
  "Run Deconvolution",
  "Apply Stretch",
  "Run Masked Stretch",
  "Run Local Contrast",
  "Run HDRMT",
  "Run PixelMath",
];

function fileOf(isRgb: boolean): ProcessedFile {
  return {
    id: "f1",
    name: isRgb ? "drizzle_rgb.fits" : "m31_L.fits",
    path: isRgb ? "C:/data/drizzle_rgb.fits" : "C:/data/m31_L.fits",
    sourcePath: isRgb ? "C:/data/drizzle_rgb.fits" : "C:/data/m31_L.fits",
    imageRef: null,
    size: 1,
    status: "done",
    result: {
      png_path: "C:/out/f1.png",
      previewUrl: "asset://C:/out/f1.png",
      dimensions: [64, 64],
      elapsed_ms: 1,
      is_rgb: isRgb,
    },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

async function renderTab(isRgb: boolean): Promise<string> {
  const file = fileOf(isRgb);
  const tree = createElement(CompositeProvider, {
    children: createElement(ComposeWizardProvider, {
      children: createElement(PreviewProvider, { file, doneFiles: [file], children: createElement(ProcessingTab) }),
    }),
  });
  const { prelude } = await prerender(tree);
  return new Response(prelude).text();
}

function runButtons(html: string): { label: string; describedBy: string | null; disabled: boolean }[] {
  const buttons: { label: string; describedBy: string | null; disabled: boolean }[] = [];
  for (const match of html.matchAll(/<button([^>]*class="ab-run-btn[^"]*"[^>]*)>([\s\S]*?)<\/button>/g)) {
    const attrs = match[1];
    const label = match[2].replace(/<[^>]*>/g, "").trim();
    buttons.push({
      label,
      describedBy: /aria-describedby="([^"]*)"/.exec(attrs)?.[1] ?? null,
      disabled: /data-disabled="true"/.test(attrs),
    });
  }
  return buttons;
}

function panelRunButtons(html: string) {
  const buttons = runButtons(html);
  return PANEL_RUN_LABELS.map((prefix) => {
    const button = buttons.find((b) => b.label.startsWith(prefix));
    if (!button) throw new Error(`run button "${prefix}" not rendered`);
    return button;
  });
}

describe("ProcessingTab RGB FITS notice", () => {
  it("links every panel's disabled Run button to the notice with aria-describedby", async () => {
    const html = await renderTab(true);
    const noticeId = /<div id="([^"]+)" role="note"[^>]*>([^<]*)<\/div>/.exec(html);
    expect(noticeId?.[2]).toBe(RGB_FITS_NOTICE);
    for (const button of panelRunButtons(html)) {
      expect(button.disabled).toBe(true);
      expect(button.describedBy).toBe(noticeId?.[1]);
    }
  });

  it("renders no notice and no aria-describedby for a mono file", async () => {
    const html = await renderTab(false);
    expect(html).not.toContain('role="note"');
    expect(html).not.toContain("aria-describedby");
    expect(panelRunButtons(html)).toHaveLength(PANEL_RUN_LABELS.length);
  });
});
