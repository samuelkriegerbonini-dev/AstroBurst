import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PROGRESS_STRIP_LINGER_MS, scheduleIdleExpiry } from "../useLingeringVisibility";

describe("scheduleIdleExpiry", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("expires once the linger window passes after the work is done", () => {
    const onExpire = vi.fn();
    scheduleIdleExpiry(false, PROGRESS_STRIP_LINGER_MS, onExpire);
    vi.advanceTimersByTime(PROGRESS_STRIP_LINGER_MS - 1);
    expect(onExpire).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onExpire).toHaveBeenCalledTimes(1);
  });

  it("never expires while work is in progress", () => {
    const onExpire = vi.fn();
    scheduleIdleExpiry(true, PROGRESS_STRIP_LINGER_MS, onExpire);
    vi.advanceTimersByTime(PROGRESS_STRIP_LINGER_MS * 10);
    expect(onExpire).not.toHaveBeenCalled();
  });

  it("cancels the pending expiry when work starts again before the window ends", () => {
    const onExpire = vi.fn();
    const cancel = scheduleIdleExpiry(false, PROGRESS_STRIP_LINGER_MS, onExpire);
    vi.advanceTimersByTime(PROGRESS_STRIP_LINGER_MS / 2);
    cancel();
    vi.advanceTimersByTime(PROGRESS_STRIP_LINGER_MS);
    expect(onExpire).not.toHaveBeenCalled();
  });

  it("lingers long enough to read the result but frees the space quickly", () => {
    expect(PROGRESS_STRIP_LINGER_MS).toBeGreaterThanOrEqual(1500);
    expect(PROGRESS_STRIP_LINGER_MS).toBeLessThanOrEqual(4000);
  });
});
