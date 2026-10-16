import { describe, expect, it } from "vitest";
import {
  BREAKER_LIMIT,
  MAX_THUMBS_IN_FLIGHT,
  createThumbQueue,
  markFailed,
  markRateLimited,
  markReady,
  markStarted,
  nextStarts,
  resumeNow,
  retryFailed,
  setActive,
  setDemand,
  thumbStatus,
  tickQueue,
  type ThumbQueue,
} from "../logic/thumbQueue";

function demand(q: ThumbQueue, selected: string[], visible: string[], ahead: string[] = []) {
  return setDemand(q, { selected, visible, ahead });
}

describe("thumbQueue priorities", () => {
  it("starts at most four, selected first, then visible, then one screen ahead", () => {
    let q = demand(createThumbQueue(), ["s"], ["v1", "v2", "v3", "v4"], ["a1"]);
    expect(MAX_THUMBS_IN_FLIGHT).toBe(4);
    expect(nextStarts(q, 0)).toEqual(["s", "v1", "v2", "v3"]);
    q = markStarted(q, ["s", "v1", "v2", "v3"]);
    expect(nextStarts(q, 0)).toEqual([]);
    q = markReady(q, "v1");
    expect(nextStarts(q, 0)).toEqual(["v4"]);
    q = markStarted(q, ["v4"]);
    q = markReady(q, "v2");
    expect(nextStarts(q, 0)).toEqual(["a1"]);
  });

  it("never starts a key twice or one that is already done", () => {
    let q = demand(createThumbQueue(), ["x"], ["x", "y"]);
    expect(nextStarts(q, 0)).toEqual(["x", "y"]);
    q = markStarted(q, ["x", "y"]);
    q = markReady(q, "x");
    q = markFailed(q, "y", "network");
    q = demand(q, [], ["x", "y", "z"]);
    expect(nextStarts(q, 0)).toEqual(["z"]);
  });

  it("drops keys that leave the viewport but lets in-flight ones finish", () => {
    let q = demand(createThumbQueue(), [], ["a", "b", "c", "d", "e", "f"]);
    q = markStarted(q, nextStarts(q, 0));
    q = demand(q, [], ["e", "f"]);
    expect(q.inflight).toEqual(["a", "b", "c", "d"]);
    expect(nextStarts(q, 0)).toEqual([]);
    q = markReady(q, "a");
    expect(nextStarts(q, 0)).toEqual(["e"]);
    expect(thumbStatus(q, "a", 0)).toBe("ready");
  });

  it("starts nothing while inactive", () => {
    let q = demand(createThumbQueue(), [], ["a"]);
    q = setActive(q, false);
    expect(nextStarts(q, 0)).toEqual([]);
    q = setActive(q, true);
    expect(nextStarts(q, 0)).toEqual(["a"]);
  });
});

describe("thumbQueue statuses", () => {
  it("reports pending, loading, ready and failed with the reason", () => {
    let q = demand(createThumbQueue(), [], ["a", "b", "c"]);
    q = markStarted(q, ["a", "b"]);
    q = markReady(q, "a");
    q = markFailed(q, "b", "not an image");
    expect(thumbStatus(q, "a", 0)).toBe("ready");
    expect(thumbStatus(q, "b", 0)).toBe("failed");
    expect(q.reasons.b).toBe("not an image");
    expect(thumbStatus(q, "c", 0)).toBe("pending");
  });
});

describe("thumbQueue rate limiting", () => {
  it("pauses until Retry-After, marks queued cards paused and resumes on its own", () => {
    let q = demand(createThumbQueue(), [], ["a", "b", "c", "d", "e"]);
    q = markStarted(q, nextStarts(q, 1000));
    q = markRateLimited(q, "a", 1000, 5, 1);
    expect(q.pausedUntil).toBe(6000);
    expect(nextStarts(q, 2000)).toEqual([]);
    expect(thumbStatus(q, "a", 2000)).toBe("paused");
    expect(thumbStatus(q, "e", 2000)).toBe("paused");
    expect(thumbStatus(q, "b", 2000)).toBe("loading");
    q = tickQueue(q, 6000);
    expect(q.pausedUntil).toBeNull();
    expect(nextStarts(q, 6000)).toEqual(["a"]);
  });

  it("stops resuming on its own once the breaker count reaches three, until R", () => {
    let q = demand(createThumbQueue(), [], ["a", "b"]);
    q = markStarted(q, ["a"]);
    q = markRateLimited(q, "a", 0, 10, BREAKER_LIMIT);
    expect(BREAKER_LIMIT).toBe(3);
    expect(q.gaveUp).toBe(true);
    q = tickQueue(q, 60_000);
    expect(q.pausedUntil).not.toBeNull();
    expect(nextStarts(q, 60_000)).toEqual([]);
    q = retryFailed(q);
    expect(q.gaveUp).toBe(false);
    expect(q.pausedUntil).toBeNull();
    expect(nextStarts(q, 60_000)).toEqual(["a", "b"]);
  });

  it("lets Resume now skip the countdown", () => {
    let q = demand(createThumbQueue(), [], ["a"]);
    q = markStarted(q, ["a"]);
    q = markRateLimited(q, "a", 0, 42, 1);
    q = resumeNow(q);
    expect(nextStarts(q, 1)).toEqual(["a"]);
  });

  it("clears the consecutive count after a success", () => {
    let q = demand(createThumbQueue(), [], ["a", "b"]);
    q = markStarted(q, ["a", "b"]);
    q = markRateLimited(q, "a", 0, 1, 2);
    expect(q.consecutive).toBe(2);
    q = markReady(q, "b");
    expect(q.consecutive).toBe(0);
  });
});

describe("retryFailed", () => {
  it("puts failed keys back in the queue, all of them or the ones given", () => {
    let q = demand(createThumbQueue(), [], ["a", "b"]);
    q = markStarted(q, ["a", "b"]);
    q = markFailed(q, "a", "network");
    q = markFailed(q, "b", "network");
    const one = retryFailed(q, ["b"]);
    expect(thumbStatus(one, "b", 0)).toBe("pending");
    expect(thumbStatus(one, "a", 0)).toBe("failed");
    const all = retryFailed(q);
    expect(nextStarts(all, 0)).toEqual(["a", "b"]);
    expect(all.reasons).toEqual({});
  });
});
