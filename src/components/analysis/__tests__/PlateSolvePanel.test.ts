import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import PlateSolvePanel from "../PlateSolvePanel";

function inputTitled(markup: string, titleStart: string): string {
  const input = markup.match(new RegExp(`<input[^>]*title="${titleStart}[^"]*"[^>]*>`));
  expect(input, `input titled "${titleStart}..."`).not.toBeNull();
  return input![0];
}

function buttonEndingWith(markup: string, text: string): string {
  const buttons = markup.match(/<button[^>]*>(?:(?!<\/button>)[\s\S])*<\/button>/g) ?? [];
  const button = buttons.find((b) => b.endsWith(`${text}</button>`));
  expect(button, `button ending with "${text}"`).toBeDefined();
  return button!;
}

describe("PlateSolvePanel position hint fields", () => {
  const markup = renderToStaticMarkup(createElement(PlateSolvePanel, { filePath: "C:/x.fits", solvePath: "C:/x.fits" }));

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

describe("PlateSolvePanel solve target", () => {
  it("renders the solve badge in the Plate Solve header", () => {
    const markup = renderToStaticMarkup(
      createElement(PlateSolvePanel, {
        filePath: "/data/frame1.fits",
        solvePath: "/out/stack.fits",
        solveBadge: createElement("b", null, "stack-badge"),
      }),
    );
    expect(markup).toMatch(/Plate Solve\s*<\/span><b>stack-badge<\/b>/);
  });

  it("disables the solve while there is no measured image to solve", () => {
    const markup = renderToStaticMarkup(createElement(PlateSolvePanel, { filePath: "/data/frame1.fits", solvePath: null }));
    expect(buttonEndingWith(markup, "Plate Solve")).toMatch(/^<button[^>]*\sdisabled=""/);
  });

  it("enables the solve on the measured image", () => {
    const markup = renderToStaticMarkup(createElement(PlateSolvePanel, { filePath: "/data/frame1.fits", solvePath: "/out/stack.fits" }));
    expect(buttonEndingWith(markup, "Plate Solve")).not.toMatch(/^<button[^>]*\sdisabled=""/);
  });
});
