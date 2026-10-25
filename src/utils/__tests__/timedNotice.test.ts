import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createTimedNotice } from "../timedNotice";

const NOTICE_MS = 6000;

describe("createTimedNotice", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows the value and hides it once the notice time passes", () => {
    const onChange = vi.fn();
    const { show } = createTimedNotice<string>(onChange, NOTICE_MS);
    show("C:/out/a_etd.txt");
    expect(onChange).toHaveBeenLastCalledWith("C:/out/a_etd.txt");
    vi.advanceTimersByTime(NOTICE_MS - 1);
    expect(onChange).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);
    expect(onChange).toHaveBeenLastCalledWith(null);
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it("keeps a second notice shown 3 s after the first for its own full time (CDP L13)", () => {
    const shown: (string | null)[] = [];
    const { show } = createTimedNotice<string>((value) => shown.push(value), NOTICE_MS);
    show("C:/out/a_etd.txt");
    vi.advanceTimersByTime(3000);
    show("C:/out/a_exoclock.csv");
    vi.advanceTimersByTime(5000);
    expect(shown.at(-1)).toBe("C:/out/a_exoclock.csv");
    vi.advanceTimersByTime(NOTICE_MS - 5000);
    expect(shown).toEqual(["C:/out/a_etd.txt", "C:/out/a_exoclock.csv", null]);
  });

  it("clears the notice and its pending timer at once", () => {
    const shown: (string | null)[] = [];
    const notice = createTimedNotice<string>((value) => shown.push(value), NOTICE_MS);
    notice.show("C:/out/a_etd.txt");
    notice.clear();
    vi.advanceTimersByTime(NOTICE_MS);
    expect(shown).toEqual(["C:/out/a_etd.txt", null]);
  });
});
