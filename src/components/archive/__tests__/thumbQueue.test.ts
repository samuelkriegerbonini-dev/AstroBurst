import { describe, expect, it } from "vitest";
import type { MastError } from "../../../shared/types/mast";
import {
  BREAKER_LIMIT,
  MAX_THUMBS_IN_FLIGHT,
  createThumbQueue,
  failedCount,
  markDropped,
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

const NET: MastError = { code: "mast_network", message: "reset", context: {} };
const NOT_IMAGE: MastError = { code: "mast_not_image", message: "response is not an image", context: {} };

function demand(q: ThumbQueue, selected: string[], visible: string[], ahead: string[] = []) {
  return setDemand(q, { selected, visible, ahead });
}

describe("thumbQueue priorities", () => {
  it("starts at most four, selected first, then visible, then one screen ahead", () => {
    let q = demand(createThumbQueue(), ["s"], ["v1", "v2", "v3", "v4"], ["a1"]);
    expect(MAX_THUMBS_IN_FLIGHT).toBe(4);
    expect(nextStarts(q, 0)).toEqual(["s", "v1", "v2", "v3"]);
    q = markStarted(q, ["s", "v1", "v2", "v3"], 0);
    expect(nextStarts(q, 0)).toEqual([]);
    q = markReady(q, "v1");
    expect(nextStarts(q, 0)).toEqual(["v4"]);
    q = markStarted(q, ["v4"], 0);
    q = markReady(q, "v2");
    expect(nextStarts(q, 0)).toEqual(["a1"]);
  });

  it("never starts a key twice or one that is already done", () => {
    let q = demand(createThumbQueue(), ["x"], ["x", "y"]);
    expect(nextStarts(q, 0)).toEqual(["x", "y"]);
    q = markStarted(q, ["x", "y"], 0);
    q = markReady(q, "x");
    q = markFailed(q, "y", NET, 0);
    q = demand(q, [], ["x", "y", "z"]);
    expect(nextStarts(q, 0)).toEqual(["z"]);
  });

  it("drops keys that leave the viewport but lets in-flight ones finish", () => {
    let q = demand(createThumbQueue(), [], ["a", "b", "c", "d", "e", "f"]);
    q = markStarted(q, nextStarts(q, 0), 0);
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
    q = markStarted(q, ["a", "b"], 0);
    q = markReady(q, "a");
    q = markFailed(q, "b", NOT_IMAGE, 0);
    expect(thumbStatus(q, "a", 0)).toBe("ready");
    expect(thumbStatus(q, "b", 0)).toBe("failed");
    expect(q.failures.b.error).toBe(NOT_IMAGE);
    expect(thumbStatus(q, "c", 0)).toBe("pending");
  });
});

describe("thumbQueue rate limiting", () => {
  it("pauses until Retry-After, marks queued cards paused and resumes on its own", () => {
    let q = demand(createThumbQueue(), [], ["a", "b", "c", "d", "e"]);
    q = markStarted(q, nextStarts(q, 1000), 1000);
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

  it("keeps MAST's HTTP status for the pause when a request held back by the pause reports none", () => {
    let q = demand(createThumbQueue(), [], ["a", "b", "c"]);
    q = markStarted(q, ["a", "b"], 1000);
    q = markRateLimited(q, "a", 1000, 5, 1, 429);
    q = markRateLimited(q, "b", 1200, 5, 1, null);
    expect(q.status).toBe(429);
    expect(q.pausedUntil).toBe(6200);
    q = tickQueue(q, 6200);
    q = markStarted(q, ["c"], 6200);
    q = markRateLimited(q, "c", 6300, 5, 1, null);
    expect(q.status).toBeNull();
  });

  it("stops resuming on its own once the breaker count reaches three, until R", () => {
    let q = demand(createThumbQueue(), [], ["a", "b"]);
    q = markStarted(q, ["a"], 0);
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
    q = markStarted(q, ["a"], 0);
    q = markRateLimited(q, "a", 0, 42, 1);
    q = resumeNow(q);
    expect(nextStarts(q, 1)).toEqual(["a"]);
  });

  it("clears the consecutive count after a success", () => {
    let q = demand(createThumbQueue(), [], ["a", "b"]);
    q = markStarted(q, ["a", "b"], 0);
    q = markRateLimited(q, "a", 0, 1, 2);
    expect(q.consecutive).toBe(2);
    q = markReady(q, "b");
    expect(q.consecutive).toBe(0);
  });
});

describe("thumbQueue failures", () => {
  const timeout: MastError = { code: "mast_timeout", message: "no answer within 30 s", context: { timeoutSec: 30, operation: "search/hst/api/v0.1/get_image_thumbnail" } };

  it("keeps MAST's error and the time waited since the request started", () => {
    let q = demand(createThumbQueue(), [], ["a", "b"]);
    q = markStarted(q, ["a"], 1000);
    expect(q.startedAt.a).toBe(1000);
    q = markFailed(q, "a", timeout, 42_000);
    expect(q.failures.a).toEqual({ error: timeout, waitedMs: 41_000 });
    expect(q.startedAt.a).toBeUndefined();
    expect(failedCount(q)).toBe(1);
    expect(failedCount(q, "mast_timeout")).toBe(1);
    expect(failedCount(q, "mast_server")).toBe(0);
    q = retryFailed(q, ["a"]);
    expect(q.failures).toEqual({});
    expect(thumbStatus(q, "a", 0)).toBe("pending");
  });

  it("keeps the first start time of a key already in flight", () => {
    let q = markStarted(createThumbQueue(), ["a"], 1000);
    q = markStarted(q, ["a", "b"], 5000);
    expect(q.startedAt).toEqual({ a: 1000, b: 5000 });
  });

  it("has no wait for a failure that never started, and forgets the start time of every finished key", () => {
    let q = markFailed(createThumbQueue(), "x", timeout, 5000);
    expect(q.failures.x.waitedMs).toBeNull();
    q = markStarted(q, ["r", "d", "l"], 10);
    q = markReady(q, "r");
    q = markDropped(q, "d");
    q = markRateLimited(q, "l", 20, 5, 1);
    expect(q.startedAt).toEqual({});
  });
});

describe("retryFailed", () => {
  it("puts failed keys back in the queue, all of them or the ones given", () => {
    let q = demand(createThumbQueue(), [], ["a", "b"]);
    q = markStarted(q, ["a", "b"], 0);
    q = markFailed(q, "a", NET, 0);
    q = markFailed(q, "b", NET, 0);
    const one = retryFailed(q, ["b"]);
    expect(thumbStatus(one, "b", 0)).toBe("pending");
    expect(thumbStatus(one, "a", 0)).toBe("failed");
    const all = retryFailed(q);
    expect(nextStarts(all, 0)).toEqual(["a", "b"]);
    expect(all.failures).toEqual({});
  });
});
