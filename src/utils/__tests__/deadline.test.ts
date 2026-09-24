import { describe, it, expect, vi, afterEach } from "vitest";
import { withDeadline } from "../deadline";

describe("withDeadline", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("gives up after the configured time when the task never settles", async () => {
    vi.useFakeTimers();
    const pending = withDeadline(new Promise<string>(() => {}), 30_000, "timed out");
    const outcome = pending.then(() => "resolved", (e: Error) => e.message);
    await vi.advanceTimersByTimeAsync(30_000);
    await expect(outcome).resolves.toBe("timed out");
  });

  it("returns the task result when it settles first", async () => {
    vi.useFakeTimers();
    const pending = withDeadline(Promise.resolve(42), 30_000, "timed out");
    await expect(pending).resolves.toBe(42);
  });

  it("passes the task error through", async () => {
    await expect(withDeadline(Promise.reject(new Error("boom")), 1000, "timed out")).rejects.toThrow("boom");
  });

  it("does not impose a deadline for a non-positive timeout", async () => {
    await expect(withDeadline(Promise.resolve("ok"), 0, "timed out")).resolves.toBe("ok");
  });
});
