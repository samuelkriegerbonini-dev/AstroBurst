import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ProcessedFile } from "../../../../shared/types";
import type { WizardState } from "../../../../utils/wizard";

vi.mock("../../../../services/compose", () => ({
  calibrateAndScnr: vi.fn(),
  computeAutoWb: vi.fn(() => new Promise(() => {})),
  resetWb: vi.fn(),
}));

vi.mock("../../../../infrastructure/tauri/client", () => ({
  getPreviewUrl: vi.fn(async (p: string) => `asset://${p}`),
}));

vi.mock("../../../../infrastructure/tauri", () => ({
  getOutputDir: vi.fn(async () => "/out"),
}));

import ColorBalanceStep from "../ColorBalanceStep";
import { INITIAL_STATE } from "../../../../utils/wizard";

function stateWith(files: Record<string, string[]>): WizardState {
  return {
    ...INITIAL_STATE,
    compositeReady: true,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: files[b.id] ?? [] })),
  };
}

function frame(path: string, filter: string): ProcessedFile {
  return {
    id: path,
    name: path.split("/").pop() ?? path,
    path,
    sourcePath: path,
    imageRef: null,
    size: 0,
    status: "done",
    result: { header: { FILTER: filter } },
    error: null,
    startedAt: null,
    finishedAt: null,
  } as unknown as ProcessedFile;
}

function render(state: WizardState, doneFiles: ProcessedFile[]): string {
  return renderToStaticMarkup(createElement(ColorBalanceStep, {
    state,
    doneFiles,
    onWbChange: vi.fn(),
    onSpccFactors: vi.fn(),
    onScnrChange: vi.fn(),
    onResult: vi.fn(),
    onCompositeOp: vi.fn(),
  }));
}

function spccOption(html: string): { text: string; disabled: boolean } | null {
  const match = /<option value="spcc"([^>]*)>([^<]*)<\/option>/.exec(html);
  return match ? { text: match[2], disabled: /\bdisabled\b/.test(match[1]) } : null;
}

describe("Color Balance SPCC option label", () => {
  it("names SPCC a blackbody approximation for a broadband RGB set", () => {
    const html = render(
      stateWith({ r: ["/c/r.fits"], g: ["/c/g.fits"], b: ["/c/b.fits"] }),
      [frame("/c/r.fits", "Red"), frame("/c/g.fits", "Green"), frame("/c/b.fits", "Blue")],
    );
    expect(spccOption(html)).toEqual({ text: "SPCC (blackbody approximation)", disabled: false });
    expect(html).not.toMatch(/spectrophotometric/i);
  });

  it("keeps the broadband-only label when SPCC is blocked", () => {
    const html = render(stateWith({ ha: ["/c/h.fits"], oiii: ["/c/o.fits"], sii: ["/c/s.fits"] }), []);
    expect(spccOption(html)).toEqual({ text: "SPCC (broadband RGB only)", disabled: true });
    expect(html).not.toMatch(/spectrophotometric/i);
  });
});
