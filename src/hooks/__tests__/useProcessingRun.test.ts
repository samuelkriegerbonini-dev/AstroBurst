import { describe, it, expect } from "vitest";
import {
  BUSY_TITLE,
  OTHER_FILE_TITLE,
  EMPTY_RUN_STATE,
  acquireRun,
  settleRun,
  forgetOtherKeys,
  runView,
  sameChainInput,
  sameSource,
  sameCompositeRun,
  beginCompositeCheck,
  clearRunError,
  bustPreviewUrl,
  isCancelMessage,
  type RunState,
} from "../useProcessingRun";
import { EMPTY_CHAIN, withStep } from "../../utils/processingChain";
import { noteCompositeAction } from "../../context/CompositeContext";
import type { ChainEntry } from "../../shared/types/preview";

const A = "1|C:/data/a.fits";
const B = "2|C:/data/b.fits";
const STF = { shadow: 0, midtone: 0.5, highlight: 1 };

function entry(fitsPath: string): ChainEntry {
  return { fitsPath, previewUrl: null, dimensions: null };
}

function acquired(state: RunState, panel: string, key: string, token: number): RunState {
  const next = acquireRun(state, panel, key, token);
  if (!next) throw new Error("expected the run to start");
  return next;
}

describe("acquireRun", () => {
  it("refuses a second step on the same file while one is running", () => {
    const s = acquired(EMPTY_RUN_STATE, "background", A, 1);
    expect(acquireRun(s, "denoise", A, 2)).toBeNull();
    expect(acquireRun(s, "background", A, 3)).toBeNull();
  });

  it("lets a step start on another file", () => {
    const s = acquired(EMPTY_RUN_STATE, "background", A, 1);
    expect(acquireRun(s, "denoise", B, 2)).not.toBeNull();
  });

  it("refuses the same step on another file while it runs, since both runs share one progress event and cancel flag", () => {
    const s = acquired(EMPTY_RUN_STATE, "deconv", A, 1);
    expect(acquireRun(s, "deconv", B, 2)).toBeNull();
  });

  it("lets the step run on another file once the first run settled", () => {
    let s = acquired(EMPTY_RUN_STATE, "deconv", A, 1);
    s = forgetOtherKeys(s, "deconv", B);
    s = settleRun(s, "deconv", A, 1, { iterations_run: 200 }, null);
    expect(runView(s, "deconv", B)).toMatchObject({ running: false, blocked: false });
    expect(acquireRun(s, "deconv", B, 2)).not.toBeNull();
  });

  it("clears the panel's previous result when a new run starts", () => {
    let s = acquired(EMPTY_RUN_STATE, "denoise", A, 1);
    s = settleRun(s, "denoise", A, 1, { v: 1 }, null);
    s = acquired(s, "denoise", A, 2);
    expect(runView(s, "denoise", A).result).toBeNull();
  });
});

describe("runView", () => {
  it("marks the running panel as running and every other panel on that file as blocked", () => {
    const s = acquired(EMPTY_RUN_STATE, "deconv", A, 1);
    expect(runView(s, "deconv", A)).toMatchObject({ running: true, blocked: false, busyTitle: undefined });
    expect(runView(s, "stretch", A)).toMatchObject({ running: false, blocked: true, busyTitle: BUSY_TITLE });
    expect(runView(s, "stretch", B)).toMatchObject({ running: false, blocked: false, busyTitle: undefined });
  });

  it("blocks the same step on another file and says why", () => {
    const s = acquired(EMPTY_RUN_STATE, "deconv", A, 1);
    expect(runView(s, "deconv", B)).toMatchObject({ running: false, blocked: true, busyTitle: OTHER_FILE_TITLE });
  });

  it("prefers the per-file title when the file is busy with another step", () => {
    let s = acquired(EMPTY_RUN_STATE, "deconv", A, 1);
    s = acquired(s, "background", B, 2);
    expect(runView(s, "deconv", B)).toMatchObject({ blocked: true, busyTitle: BUSY_TITLE });
  });

  it("keeps the running state visible to a remounted panel until the run settles", () => {
    let s = acquired(EMPTY_RUN_STATE, "deconv", A, 1);
    s = forgetOtherKeys(s, "deconv", A);
    expect(runView(s, "deconv", A).running).toBe(true);
    s = settleRun(s, "deconv", A, 1, { iterations_run: 20 }, null);
    expect(runView(s, "deconv", A)).toMatchObject({ running: false, result: { iterations_run: 20 } });
  });

  it("does not show one file's result while another file is selected", () => {
    let s = acquired(EMPTY_RUN_STATE, "denoise", A, 1);
    s = settleRun(s, "denoise", A, 1, { v: "a" }, null);
    expect(runView(s, "denoise", A).result).toEqual({ v: "a" });
    expect(runView(s, "denoise", B).result).toBeNull();
  });
});

describe("settleRun", () => {
  it("releases the lock and stores the result for the file it ran on", () => {
    let s = acquired(EMPTY_RUN_STATE, "stretch", A, 1);
    s = settleRun(s, "stretch", A, 1, { factor: 20 }, null);
    expect(s.locks.has(A)).toBe(false);
    expect(runView(s, "stretch", A).result).toEqual({ factor: 20 });
  });

  it("drops a result that lands after the panel moved to another file", () => {
    let s = acquired(EMPTY_RUN_STATE, "denoise", A, 1);
    s = forgetOtherKeys(s, "denoise", B);
    s = settleRun(s, "denoise", A, 1, { v: "late" }, null);
    expect(s.locks.has(A)).toBe(false);
    expect(runView(s, "denoise", A).result).toBeNull();
    expect(runView(s, "denoise", B).result).toBeNull();
  });

  it("delivers the result and error to a panel that left the file and came back before the run ended", () => {
    let s = acquired(EMPTY_RUN_STATE, "deconv", A, 1);
    s = forgetOtherKeys(s, "deconv", B);
    s = forgetOtherKeys(s, "deconv", A);
    expect(runView(s, "deconv", A).running).toBe(true);
    s = settleRun(s, "deconv", A, 1, null, "some error");
    expect(runView(s, "deconv", A)).toMatchObject({ running: false, result: null, error: "some error" });

    let t = acquired(EMPTY_RUN_STATE, "deconv", A, 2);
    t = forgetOtherKeys(t, "deconv", B);
    t = forgetOtherKeys(t, "deconv", A);
    t = settleRun(t, "deconv", A, 2, { iterations_run: 200 }, null);
    expect(runView(t, "deconv", A)).toMatchObject({ running: false, result: { iterations_run: 200 }, error: null });
  });

  it("clears a delivered result when the panel moves to another file", () => {
    let s = acquired(EMPTY_RUN_STATE, "deconv", A, 1);
    s = settleRun(s, "deconv", A, 1, { iterations_run: 200 }, null);
    s = forgetOtherKeys(s, "deconv", B);
    s = forgetOtherKeys(s, "deconv", A);
    expect(runView(s, "deconv", A).result).toBeNull();
  });

  it("ignores a settle from a run that no longer owns the panel", () => {
    let s = acquired(EMPTY_RUN_STATE, "denoise", A, 1);
    s = settleRun(s, "denoise", A, 1, { v: "a" }, null);
    s = forgetOtherKeys(s, "denoise", B);
    s = acquired(s, "denoise", B, 2);
    const stale = settleRun(s, "denoise", A, 1, { v: "stale" }, null);
    expect(stale).toBe(s);
    expect(runView(stale, "denoise", B)).toMatchObject({ running: true, result: null });
  });

  it("keeps the error of a failed run", () => {
    let s = acquired(EMPTY_RUN_STATE, "pixelmath", A, 1);
    s = settleRun(s, "pixelmath", A, 1, null, "bad expression");
    expect(runView(s, "pixelmath", A)).toMatchObject({ running: false, result: null, error: "bad expression" });
  });
});

describe("sameChainInput", () => {
  const bg = withStep(EMPTY_CHAIN, "background", entry("C:/out/a_bg_corrected.fits"));

  it("detects a reset that removed the step's upstream input", () => {
    expect(sameChainInput(bg, EMPTY_CHAIN, "denoise")).toBe(false);
  });

  it("detects an upstream re-run with a different output", () => {
    const other = withStep(EMPTY_CHAIN, "background", entry("C:/out/a_dbe.fits"));
    expect(sameChainInput(bg, other, "denoise")).toBe(false);
  });

  it("accepts the same upstream output written with other separators or case", () => {
    const same = withStep(EMPTY_CHAIN, "background", entry("c:\\OUT\\a_bg_corrected.fits"));
    expect(sameChainInput(bg, same, "denoise")).toBe(true);
  });

  it("ignores downstream changes and the first step's own input", () => {
    const withStretch = withStep(bg, "stretch", entry("C:/out/a_arcsinh.fits"));
    expect(sameChainInput(bg, withStretch, "denoise")).toBe(true);
    expect(sameChainInput(bg, EMPTY_CHAIN, "background")).toBe(true);
  });
});

describe("sameSource", () => {
  it("treats a reset of the displayed image as a change", () => {
    expect(sameSource("C:/out/a_bg.fits", null)).toBe(false);
    expect(sameSource(null, "C:/out/a_bg.fits")).toBe(false);
    expect(sameSource(null, null)).toBe(true);
    expect(sameSource("C:/out/a_bg.fits", "c:\\out\\A_BG.fits")).toBe(true);
  });
});

describe("sameCompositeRun", () => {
  it("rejects a composite result after the composite changed or another file was selected", () => {
    const start = { version: 3, fileKey: A };
    expect(sameCompositeRun(start, { version: 3, fileKey: A })).toBe(true);
    expect(sameCompositeRun(start, { version: 4, fileKey: A })).toBe(false);
    expect(sameCompositeRun(start, { version: 3, fileKey: B })).toBe(false);
  });
});

describe("beginCompositeCheck", () => {
  it("applies a composite result that lands after the panel closed when the composite and file are unchanged", () => {
    const stillCurrent = beginCompositeCheck(() => A);
    noteCompositeAction({ type: "SET_STF", r: STF, g: STF, b: STF });
    expect(stillCurrent()).toBe(true);
  });

  it("drops the result after 'Back to file' even though no processing panel is mounted to observe it", () => {
    const stillCurrent = beginCompositeCheck(() => A);
    noteCompositeAction({ type: "RESET" });
    expect(stillCurrent()).toBe(false);
  });

  it("drops the result when the composite preview was replaced after the click", () => {
    const stillCurrent = beginCompositeCheck(() => A);
    noteCompositeAction({ type: "SET_PREVIEW_URL", url: "asset://blend.png" });
    expect(stillCurrent()).toBe(false);
  });

  it("drops the result when another file was selected while no panel observed the composite", () => {
    let key: string | null = A;
    const stillCurrent = beginCompositeCheck(() => key);
    key = B;
    expect(stillCurrent()).toBe(false);
  });

  it("reads the version from the getter it is given", () => {
    let version = 7;
    const stillCurrent = beginCompositeCheck(() => A, () => version);
    expect(stillCurrent()).toBe(true);
    version = 8;
    expect(stillCurrent()).toBe(false);
  });
});

describe("clearRunError", () => {
  it("clears a failed run's error for the file so a later batch does not show it", () => {
    let s = acquired(EMPTY_RUN_STATE, "debayer", A, 1);
    s = settleRun(s, "debayer", A, 1, null, "no Bayer pattern");
    expect(runView(s, "debayer", A).error).toBe("no Bayer pattern");
    s = clearRunError(s, "debayer", A);
    expect(runView(s, "debayer", A)).toMatchObject({ running: false, result: null, error: null });
  });

  it("leaves another file's entry and a successful result untouched", () => {
    let s = acquired(EMPTY_RUN_STATE, "debayer", A, 1);
    s = settleRun(s, "debayer", A, 1, null, "no Bayer pattern");
    expect(clearRunError(s, "debayer", B)).toBe(s);
    let t = acquired(EMPTY_RUN_STATE, "debayer", A, 2);
    t = settleRun(t, "debayer", A, 2, { r_path: "C:/out/a_R.fits" }, null);
    expect(clearRunError(t, "debayer", A)).toBe(t);
  });
});

describe("bustPreviewUrl", () => {
  const url = "http://asset.localhost/C%3A%2Fout%2Fa_denoised.png";

  it("gives a re-run of the same output file a different URL", () => {
    const first = bustPreviewUrl(url, 1000);
    const second = bustPreviewUrl(url, 2000);
    expect(first).not.toBe(url);
    expect(second).not.toBe(first);
  });

  it("replaces an earlier version stamp instead of stacking them", () => {
    expect(bustPreviewUrl(`${url}?v=1`, 5)).toBe(`${url}?v=5`);
  });

  it("leaves data and blob URLs alone and maps a missing URL to undefined", () => {
    expect(bustPreviewUrl("data:image/png;base64,AAAA", 5)).toBe("data:image/png;base64,AAAA");
    expect(bustPreviewUrl("blob:http://x/1", 5)).toBe("blob:http://x/1");
    expect(bustPreviewUrl(null, 5)).toBeUndefined();
    expect(bustPreviewUrl("", 5)).toBeUndefined();
  });
});

describe("isCancelMessage", () => {
  it("recognises a user cancel", () => {
    expect(isCancelMessage("Operation cancelled by user")).toBe(true);
    expect(isCancelMessage("File not found")).toBe(false);
  });
});
