import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const css = readFileSync(fileURLToPath(new URL("../index.css", import.meta.url)), "utf8");

function reducedMotionBlock(text: string): string {
  const start = text.indexOf("@media (prefers-reduced-motion: reduce)");
  if (start < 0) return "";
  let depth = 0;
  for (let i = text.indexOf("{", start); i < text.length; i += 1) {
    if (text[i] === "{") depth += 1;
    if (text[i] === "}") {
      depth -= 1;
      if (depth === 0) return text.slice(start, i + 1);
    }
  }
  return "";
}

function universalRule(): string {
  return /\*,\s*\*::before,\s*\*::after\s*\{([^}]*)\}/.exec(reducedMotionBlock(css))?.[1] ?? "";
}

describe("reduced motion in index.css", () => {
  it("removes every animation, so no finished fill-mode animation stays in effect", () => {
    expect(universalRule()).toMatch(/(^|[;\s])animation:\s*none\s*!important/);
    expect(universalRule()).not.toMatch(/animation-(duration|iteration-count)/);
  });

  it("keeps transitions short instead of removing them, because the dock waits for transitionend", () => {
    expect(universalRule()).toMatch(/transition-duration:\s*0\.01ms\s*!important/);
  });
});
