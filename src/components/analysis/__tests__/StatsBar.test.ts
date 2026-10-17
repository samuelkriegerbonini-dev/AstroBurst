import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import StatsBar from "../StatsBar";
import type { QueueStats } from "../../../shared/types";

function renderStats(stats: Partial<QueueStats>): string {
  return renderToStaticMarkup(
    createElement(StatsBar, {
      stats: { total: 0, done: 0, failed: 0, spectra: 0, totalBytes: 0, ...stats },
      isProcessing: false,
      isComplete: true,
    }),
  );
}

describe("StatsBar spectra count", () => {
  it("prints 1 spectrum for a single spectrum file", () => {
    expect(renderStats({ total: 2, done: 1, spectra: 1 })).toContain(">1 spectrum<");
  });

  it("prints the plural for several spectrum files", () => {
    expect(renderStats({ total: 3, spectra: 3 })).toContain(">3 spectra<");
  });

  it("prints no spectra count without spectrum files", () => {
    const html = renderStats({ total: 2, done: 2 });
    expect(html).not.toContain("spectr");
    expect(html).not.toContain("table");
  });
});
