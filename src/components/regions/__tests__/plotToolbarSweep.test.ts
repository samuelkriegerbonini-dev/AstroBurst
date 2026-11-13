import { describe, expect, it } from "vitest";
import plotSource from "../ProfilePlot.tsx?raw";

const TARGET_CLASSES = ["min-h-6", "min-w-6", "inline-flex", "items-center", "justify-center", "text-[9px]"];

function classesOf(pattern: RegExp): string[] {
  const match = plotSource.match(pattern);
  expect(match, String(pattern)).not.toBeNull();
  return match![1].split(/\s+/);
}

describe("plot toolbar sweep (a11y-17, a11y-7)", () => {
  it("makes the plot toolbar 24 px tall", () => {
    expect(plotSource).toMatch(/const TOOLBAR_H = 24;/);
  });

  it("gives every toolbar button a 24 px target", () => {
    for (const pattern of [/const TOOLBAR_BUTTON_CLASS =\s*"([^"]+)";/, /const TOOLBAR_ACTIVE_CLASS =\s*"([^"]+)";/]) {
      expect(classesOf(pattern)).toEqual(expect.arrayContaining(TARGET_CLASSES));
    }
  });

  it("tells whether the log axis is on", () => {
    expect(plotSource).toContain("aria-pressed={logY}");
  });
});
