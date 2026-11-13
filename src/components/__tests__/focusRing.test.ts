import { describe, expect, it } from "vitest";
import masksPanelSource from "../masks/MasksPanel.tsx?raw";

const SOURCES = import.meta.glob<string>("../../**/*.tsx", { query: "?raw", import: "default", eager: true });
const OUTLINE_REMOVED = /(?<![\w-])(?:focus:|focus-visible:)?outline-none(?![\w-])/;

function componentSources(): [string, string][] {
  return Object.entries(SOURCES)
    .map(([key, text]): [string, string] => [new URL(key, "file:///src/components/__tests__/").pathname.replace(/^\/src\//, ""), text])
    .filter(([file]) => !file.includes("__tests__/") && !/\.test\.tsx$/.test(file));
}

function linesWithoutRing(text: string): number {
  return text.split(/\r?\n/).filter((line) => OUTLINE_REMOVED.test(line) && !line.includes("focus-visible:ring-") && !line.includes("focus:ring-")).length;
}

describe("focus ring", () => {
  it("draws a focus mark wherever a class removes the outline", () => {
    const found: Record<string, number> = {};
    for (const [file, text] of componentSources()) {
      const count = linesWithoutRing(text);
      if (count > 0) found[file] = count;
    }
    expect(found).toEqual({ "App.tsx": 1, "components/compose/steps/ChannelStep.tsx": 1 });
  });

  it("gives the mask fields a ring in the panel's accent", () => {
    expect(masksPanelSource).toMatch(/const FIELD = "[^"]*focus:outline-none[^"]*focus-visible:ring-1 focus-visible:ring-amber-500\/60[^"]*";/);
  });
});
