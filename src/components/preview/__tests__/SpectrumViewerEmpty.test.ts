import { describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import SpectrumViewerEmpty from "../SpectrumViewerEmpty";
import { SPECTRUM_VIEWER_HINT, SPECTRUM_VIEWER_OPEN_CUBE, spectrumViewerTitle } from "../../../utils/previewShell";

const NAME = "jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";

function findButton(node: ReactNode): { onClick: () => void; children: ReactNode } | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findButton(child);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Record<string, unknown>>(node)) return null;
  if (node.type === "button") return node.props as unknown as { onClick: () => void; children: ReactNode };
  return findButton(node.props.children as ReactNode);
}

describe("SpectrumViewerEmpty", () => {
  it("names the spectrum file and the Cube tool and shows no image", () => {
    const html = renderToStaticMarkup(createElement(SpectrumViewerEmpty, { name: NAME, onOpenCubeTool: () => {} }));
    expect(html).toContain("data-spectrum-empty");
    expect(html).toContain(`>${spectrumViewerTitle(NAME)}<`);
    expect(html).toContain(`>${SPECTRUM_VIEWER_HINT}<`);
    expect(html).toContain(`>${SPECTRUM_VIEWER_OPEN_CUBE}</button>`);
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<canvas");
  });

  it("opens the Cube tool from its button", () => {
    const onOpenCubeTool = vi.fn();
    const button = findButton(SpectrumViewerEmpty({ name: NAME, onOpenCubeTool }));
    expect(button?.children).toBe(SPECTRUM_VIEWER_OPEN_CUBE);
    button?.onClick();
    expect(onOpenCubeTool).toHaveBeenCalledTimes(1);
  });
});
