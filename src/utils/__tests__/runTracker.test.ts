import { describe, it, expect } from "vitest";
import { abandonRun, beginRun, createRunTracker, isCurrentRun, settleRun } from "../runTracker";

describe("abandonRun", () => {
  it("asks for a backend cancel when the file switches while a time series run is in flight, and marks that run stale", () => {
    const t = createRunTracker();
    const seq = beginRun(t);
    expect(abandonRun(t)).toBe(true);
    expect(isCurrentRun(t, seq)).toBe(false);
  });

  it("does not cancel after the run settled, so a later run sharing the progress event is not killed", () => {
    const t = createRunTracker();
    const seq = beginRun(t);
    settleRun(t, seq);
    expect(abandonRun(t)).toBe(false);
  });

  it("cancels the orphan once across a file switch followed by unmount", () => {
    const t = createRunTracker();
    beginRun(t);
    expect(abandonRun(t)).toBe(true);
    expect(abandonRun(t)).toBe(false);
  });

  it("a late settle of the cancelled orphan does not clear the in-flight marker of the newer run", () => {
    const t = createRunTracker();
    const first = beginRun(t);
    abandonRun(t);
    const second = beginRun(t);
    expect(isCurrentRun(t, second)).toBe(true);
    settleRun(t, first);
    expect(abandonRun(t)).toBe(true);
    expect(isCurrentRun(t, second)).toBe(false);
  });

  it("does not ask for a cancel when nothing ever ran", () => {
    expect(abandonRun(createRunTracker())).toBe(false);
  });
});
