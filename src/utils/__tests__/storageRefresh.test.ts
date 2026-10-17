import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { STORAGE_REFRESH_DELAY_MS, watchStorage } from "../storageRefresh";

function source() {
  const listeners = new Set<() => void>();
  return {
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    emit: () => {
      for (const listener of listeners) listener();
    },
    count: () => listeners.size,
  };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("watchStorage", () => {
  it("refreshes the storage totals once after a burst of history and output events", () => {
    const history = source();
    const outputs = source();
    const refresh = vi.fn();
    watchStorage(refresh, [history.subscribe, outputs.subscribe]);

    history.emit();
    outputs.emit();
    vi.advanceTimersByTime(STORAGE_REFRESH_DELAY_MS - 1);
    history.emit();
    vi.advanceTimersByTime(STORAGE_REFRESH_DELAY_MS - 1);
    expect(refresh).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(refresh).toHaveBeenCalledTimes(1);

    outputs.emit();
    vi.advanceTimersByTime(STORAGE_REFRESH_DELAY_MS);
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("stops listening and drops a pending refresh when disposed", () => {
    const history = source();
    const refresh = vi.fn();
    const dispose = watchStorage(refresh, [history.subscribe]);

    history.emit();
    dispose();
    vi.advanceTimersByTime(STORAGE_REFRESH_DELAY_MS * 2);

    expect(refresh).not.toHaveBeenCalled();
    expect(history.count()).toBe(0);
  });
});
