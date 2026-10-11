import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ProcessedFile } from "../../../../shared/types";
import type { WizardState } from "../../../../utils/wizard";

const { spccProps, composite } = vi.hoisted(() => ({
  spccProps: { current: null as { rPath: string | null; wcsPath?: string | null; wcsReason?: string | null } | null },
  composite: { linked: true },
}));

vi.mock("../../SpccPanel", () => ({
  default: (props: { rPath: string | null; wcsPath?: string | null; wcsReason?: string | null }) => {
    spccProps.current = props;
    return null;
  },
}));

vi.mock("../../../../context/CompositeContext", () => ({
  useCompositeStf: () => ({ compositeStfLinked: composite.linked }),
}));

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
import { WB_STRETCH_NOTE, wbStretchNote } from "../../../../utils/compositeStfResponse";
import { rememberSolvedWcs } from "../../../../utils/solvedWcs";

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

describe("wbStretchNote", () => {
  it("explains that a real white balance replaced the unlinked Blend stretch", () => {
    expect(wbStretchNote(false, [1.2, 1, 0.8])).toBe(WB_STRETCH_NOTE);
    expect(WB_STRETCH_NOTE).toBe("White balance needs a linked stretch, so the unlinked Blend stretch was replaced by a linked one.");
  });

  it("says nothing when the unlinked stretch is kept for neutral factors", () => {
    expect(wbStretchNote(false, [1, 1, 1])).toBeNull();
    expect(wbStretchNote(false, [1 + 1e-7, 1, 1 - 1e-7])).toBeNull();
  });

  it("says nothing when the stretch was already linked", () => {
    expect(wbStretchNote(true, [1.2, 1, 0.8])).toBeNull();
  });
});

const BROADBAND = [frame("/c/r.fits", "Red"), frame("/c/g.fits", "Green"), frame("/c/b.fits", "Blue")];

async function renderSpcc(state: WizardState): Promise<NonNullable<typeof spccProps.current>> {
  spccProps.current = null;
  render(state, BROADBAND);
  await import("../../SpccPanel");
  await new Promise((resolve) => setTimeout(resolve, 0));
  render(state, BROADBAND);
  if (!spccProps.current) throw new Error("SpccPanel was not rendered");
  return spccProps.current;
}

describe("Color Balance SPCC WCS source", () => {
  const spccState = (overrides: Partial<WizardState> = {}): WizardState => ({
    ...stateWith({ r: ["/c/r.fits"], g: ["/c/g.fits"], b: ["/c/b.fits"] }),
    wbMode: "spcc",
    ...overrides,
  });

  it("passes the WCS plate-solved on the R channel's own file", async () => {
    rememberSolvedWcs("/c/r.fits", "/out/r_wcs.fits");
    const props = await renderSpcc(spccState());
    expect(props.wcsPath).toBe("/out/r_wcs.fits");
    expect(props.wcsReason).toBeNull();
  });

  it("passes no WCS and the reason when R was aligned onto another channel", async () => {
    rememberSolvedWcs("/c/r.fits", "/out/r_wcs.fits");
    const aligned = { r: "__wizard_aligned_r", g: "__wizard_aligned_g", b: "__wizard_aligned_b" };
    const props = await renderSpcc(spccState({ alignedPaths: aligned, alignRefBinId: "g" }));
    expect(props.rPath).toBe("__wizard_aligned_r");
    expect(props.wcsPath).toBeNull();
    expect(props.wcsReason).toMatch(/^SPCC can use a plate-solved WCS only on the R channel's own grid; R was aligned onto G\./);
  });
});
