import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TIMER_TICK_MS, createStopwatch } from "../useTimer";

describe("createStopwatch", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("records the elapsed time on stop even when busy work kept every tick from running", () => {
    const seen: number[] = [];
    const watch = createStopwatch((ms) => seen.push(ms));
    watch.start();
    vi.setSystemTime(140);
    watch.stop();
    expect(seen.at(-1)).toBe(140);
  });

  it("ticks while running and stays still after stop", () => {
    const seen: number[] = [];
    const watch = createStopwatch((ms) => seen.push(ms));
    watch.start();
    vi.advanceTimersByTime(TIMER_TICK_MS * 3 + 30);
    expect(seen).toEqual([100, 200, 300]);
    watch.stop();
    expect(seen.at(-1)).toBe(330);
    const count = seen.length;
    vi.advanceTimersByTime(TIMER_TICK_MS * 5);
    expect(seen).toHaveLength(count);
  });

  it("adds a later run to the time already counted, and reset goes back to zero", () => {
    const seen: number[] = [];
    const watch = createStopwatch((ms) => seen.push(ms));
    watch.start();
    vi.setSystemTime(120);
    watch.stop();
    vi.setSystemTime(5000);
    watch.start();
    vi.setSystemTime(5080);
    watch.stop();
    expect(seen.at(-1)).toBe(200);
    watch.reset();
    expect(seen.at(-1)).toBe(0);
    watch.start();
    vi.setSystemTime(5130);
    watch.stop();
    expect(seen.at(-1)).toBe(50);
  });

  it("keeps the time counted when it is disposed while running", () => {
    const seen: number[] = [];
    const watch = createStopwatch((ms) => seen.push(ms));
    watch.start();
    vi.setSystemTime(70);
    watch.dispose();
    vi.setSystemTime(1000);
    watch.start();
    vi.setSystemTime(1030);
    watch.stop();
    expect(seen.at(-1)).toBe(100);
  });
});
