import { describe, expect, it } from "vitest";
import { createElement, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import Slider from "../Slider";
import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import WaveletPanel from "../../processing/WaveletPanel";
import type { ProcessedFile } from "../../../shared/types";

function attr(tag: string, name: string): string | null {
  return new RegExp(`\\s${name}="([^"]*)"`).exec(tag)?.[1] ?? null;
}

function rangeTags(html: string): string[] {
  return [...html.matchAll(/<input[^>]*type="range"[^>]*>/g)].map((m) => m[0]);
}

function accessibleName(html: string, range: string): string {
  const label = attr(range, "aria-label");
  if (label !== null) return label;
  const id = attr(range, "id");
  const forLabel = id === null ? null : new RegExp(`<label[^>]*for="${id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"[^>]*>(.*?)</label>`).exec(html)?.[1];
  return (forLabel ?? "").replace(/<[^>]*>/g, "").trim();
}

function renderSlider(props: Partial<ComponentProps<typeof Slider>>): string {
  return renderToStaticMarkup(createElement(Slider, { label: "", value: 1, min: 0, max: 5, step: 0.1, ...props }));
}

describe("Slider accessible name", () => {
  it("names the range input from ariaLabel when the visible label is empty", () => {
    const html = renderSlider({ ariaLabel: "Fine detail threshold" });
    const [range] = rangeTags(html);
    expect(attr(range, "aria-label")).toBe("Fine detail threshold");
    expect(accessibleName(html, range)).toBe("Fine detail threshold");
  });

  it("names the value edit button after ariaLabel instead of the empty label", () => {
    const html = renderSlider({ ariaLabel: "Fine detail threshold" });
    expect(html).toContain('aria-label="Edit Fine detail threshold value"');
    expect(html).not.toContain('aria-label="Edit  value"');
  });

  it("keeps the visible label as the name without ariaLabel", () => {
    const html = renderSlider({ label: "Scales" });
    const [range] = rangeTags(html);
    expect(attr(range, "aria-label")).toBeNull();
    expect(accessibleName(html, range)).toBe("Scales");
    expect(html).toContain('aria-label="Edit Scales value"');
  });
});

describe("Wavelet per-scale sliders", () => {
  const file: ProcessedFile = {
    id: "w1",
    name: "656nmos.fits",
    path: "C:/data/656nmos.fits",
    sourcePath: "C:/data/656nmos.fits",
    imageRef: null,
    size: 1,
    status: "done",
    result: null,
    error: null,
    startedAt: null,
    finishedAt: null,
  };

  function renderWavelet(): string {
    const panel = createElement(WaveletPanel, {
      selectedFile: file,
      fileKey: file.path,
      compositeMode: false,
      compositeInput: null,
      onCompositeDone: () => {},
      fileName: file.name,
    });
    const preview = createElement(PreviewProvider, { file, doneFiles: [], children: panel });
    return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
  }

  it("gives every range input a distinct, non-empty name", () => {
    const html = renderWavelet();
    const names = rangeTags(html).map((range) => accessibleName(html, range));
    expect(names).toEqual([
      "Scales",
      "Fine detail threshold",
      "Small structures threshold",
      "Medium structures threshold",
      "Large structures threshold",
      "Very large threshold",
      "Fine detail bias",
      "Small structures bias",
      "Medium structures bias",
      "Large structures bias",
      "Very large bias",
    ]);
  });
});
