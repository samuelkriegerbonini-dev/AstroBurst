import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import PlateSolvePanel from "../PlateSolvePanel";

function inputTitled(markup: string, titleStart: string): string {
  const input = markup.match(new RegExp(`<input[^>]*title="${titleStart}[^"]*"[^>]*>`));
  expect(input, `input titled "${titleStart}..."`).not.toBeNull();
  return input![0];
}

describe("PlateSolvePanel position hint fields", () => {
  const markup = renderToStaticMarkup(createElement(PlateSolvePanel, { filePath: "C:/x.fits" }));

  it.each(["Right ascension", "Declination", "Search radius"])(
    "keeps the %s field a decimal text box, so a typed comma reaches the parser instead of becoming an empty value",
    (titleStart) => {
      const input = inputTitled(markup, titleStart);
      expect(input).toContain('type="text"');
      expect(input).toContain('inputMode="decimal"');
      expect(input).not.toMatch(/\s(min|max|step)="/);
    },
  );
});
