import { afterEach, describe, expect, it, vi } from "vitest";
import { formatCount } from "../formatCount";

describe("formatCount", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("groups thousands with a comma so a dot is never read as a thousands separator", () => {
    expect(formatCount(50000)).toBe("50,000");
    expect(formatCount(1234567)).toBe("1,234,567");
    expect(formatCount(1000)).toBe("1,000");
  });

  it("leaves numbers below a thousand ungrouped", () => {
    expect(formatCount(0)).toBe("0");
    expect(formatCount(7)).toBe("7");
    expect(formatCount(999)).toBe("999");
  });

  it("rounds to a whole count and keeps the sign", () => {
    expect(formatCount(1234.6)).toBe("1,235");
    expect(formatCount(-1234)).toBe("-1,234");
    expect(formatCount(-0.4)).toBe("0");
  });

  it("does not depend on the operating system locale", () => {
    vi.spyOn(Number.prototype, "toLocaleString").mockReturnValue("50.000");
    expect(formatCount(50000)).toBe("50,000");
  });

  it("shows a dash for a value that is not a finite number", () => {
    expect(formatCount(Number.NaN)).toBe("—");
    expect(formatCount(Number.POSITIVE_INFINITY)).toBe("—");
  });
});
