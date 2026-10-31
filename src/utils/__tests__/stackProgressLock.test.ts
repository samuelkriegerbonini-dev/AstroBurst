import { afterEach, describe, expect, it } from "vitest";
import { claimStackProgress, releaseStackProgress, stackProgressBusyLine, stackProgressHolder } from "../stackProgressLock";

describe("stack progress lock", () => {
  afterEach(() => {
    for (const run of ["stack", "drizzle", "compose"] as const) releaseStackProgress(run);
  });

  it("lets one run at a time report on stack-progress", () => {
    expect(stackProgressHolder()).toBeNull();
    expect(claimStackProgress("stack")).toBe(true);
    expect(claimStackProgress("drizzle")).toBe(false);
    expect(claimStackProgress("compose")).toBe(false);
    expect(claimStackProgress("stack")).toBe(false);
    expect(stackProgressHolder()).toBe("stack");
  });

  it("is released only by the run that holds it", () => {
    claimStackProgress("drizzle");
    releaseStackProgress("stack");
    expect(stackProgressHolder()).toBe("drizzle");
    releaseStackProgress("drizzle");
    expect(stackProgressHolder()).toBeNull();
    expect(claimStackProgress("compose")).toBe(true);
  });

  it("names the run that holds it to every other surface", () => {
    expect(stackProgressBusyLine(null, "stack")).toBeNull();
    expect(stackProgressBusyLine("stack", "stack")).toBeNull();
    expect(stackProgressBusyLine("drizzle", "stack")).toBe(
      "Stacking › Drizzle RGB (mono) is running; wait until it ends, as both use the same progress bar and Cancel.",
    );
    expect(stackProgressBusyLine("stack", "drizzle")).toBe(
      "Stacking › Stack is running; wait until it ends, as both use the same progress bar and Cancel.",
    );
    expect(stackProgressBusyLine("compose", "drizzle")).toBe(
      "Compose › Stack is running; wait until it ends, as both use the same progress bar and Cancel.",
    );
  });

  it("names a run of its own kind to a view that is not running it", () => {
    expect(stackProgressBusyLine(null, null)).toBeNull();
    expect(stackProgressBusyLine("stack", null)).toBe(
      "Stacking › Stack is running; wait until it ends, as both use the same progress bar and Cancel.",
    );
  });
});
