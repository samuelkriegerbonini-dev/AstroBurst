import { beforeEach, describe, expect, it, vi } from "vitest";

const { ingest, dock } = vi.hoisted(() => ({
  ingest: { calls: [] as string[], showWhenLoaded: vi.fn(), ingestUnlessLoaded: vi.fn() },
  dock: { dispatch: vi.fn() },
}));

vi.mock("../../../hooks/useFileIngest", () => ({
  showWhenLoaded: (path: string) => {
    ingest.calls.push(`show ${path}`);
    return ingest.showWhenLoaded(path);
  },
  ingestUnlessLoaded: (path: string) => {
    ingest.calls.push(`ingest ${path}`);
    return ingest.ingestUnlessLoaded(path);
  },
}));
vi.mock("../../../hooks/useDockLayout", () => ({ dockStore: dock }));
vi.mock("../../../services/progress", () => ({ cancelProgress: vi.fn(() => Promise.resolve(true)) }));

import { composeMasters, openInFiles } from "../resultHandoff";
import { wizardReducer, type WizardAction } from "../../../context/ComposeWizardContext";
import { INITIAL_STATE, STEPS, type WizardState } from "../../../utils/wizard";

const MASTERS = ["R", "G", "B"].map((label) => ({ label, fits_path: `C:\\out\\m42_pipeline12_${label}.fits` }));

describe("openInFiles", () => {
  beforeEach(() => {
    ingest.calls = [];
  });

  it("shows the first file in Files and adds the others after it", () => {
    openInFiles(MASTERS.map((m) => m.fits_path));
    expect(ingest.calls).toEqual([`show ${MASTERS[0].fits_path}`, `ingest ${MASTERS[1].fits_path}`, `ingest ${MASTERS[2].fits_path}`]);
  });

  it("does nothing without a file", () => {
    openInFiles([]);
    expect(ingest.calls).toEqual([]);
  });
});

describe("composeMasters", () => {
  beforeEach(() => {
    ingest.calls = [];
    dock.dispatch.mockReset();
  });

  it("opens the masters, sets the Compose bins from their lanes, and opens Compose on Channels with Align available", () => {
    let state: WizardState = { ...INITIAL_STATE, completedSteps: { channels: true, stack: true } };
    const order: string[] = [];
    const wizard = {
      getState: () => state,
      dispatch: (action: WizardAction) => {
        order.push(action.type);
        state = wizardReducer(state, action);
      },
      setActiveStep: (step: string) => order.push(`step ${step}`),
    };
    dock.dispatch.mockImplementation((action: { type: string; tool: string }) => order.push(`${action.type} ${action.tool}`));

    composeMasters(MASTERS, wizard);

    expect(ingest.calls).toEqual([`show ${MASTERS[0].fits_path}`, `ingest ${MASTERS[1].fits_path}`, `ingest ${MASTERS[2].fits_path}`]);
    expect(order).toEqual(["SET_BINS", "step channels", "open compose"]);
    expect(Object.fromEntries(state.bins.filter((bin) => bin.files.length > 0).map((bin) => [bin.id, bin.files]))).toEqual({
      r: [MASTERS[0].fits_path],
      g: [MASTERS[1].fits_path],
      b: [MASTERS[2].fits_path],
    });
    expect(state.completedSteps).toEqual({ channels: true });
    expect(STEPS.find((step) => step.id === "align")?.enabled(state)).toBe(true);
  });
});
