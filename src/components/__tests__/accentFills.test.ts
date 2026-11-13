import { describe, expect, it } from "vitest";

const SOURCES = import.meta.glob<string>(["../compose/ComposeWizard.tsx", "../ErrorBoundary.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
});

const composeWizard = SOURCES["../compose/ComposeWizard.tsx"];
const errorBoundary = SOURCES["../ErrorBoundary.tsx"];

describe("text on accent fills", () => {
  it("writes the done step badge of Compose in dark text on its green fill", () => {
    const badge = composeWizard.match(/\$\{isDone \? "([^"]+)" : colors\.dot/);
    expect(badge).not.toBeNull();
    const classes = badge![1].split(/\s+/);
    expect(classes).toContain("bg-emerald-500");
    expect(classes).toContain("text-zinc-900");
    expect(classes).not.toContain("text-white");
  });

  it("gives the crash screen's restart button the primary button style", () => {
    const restartButton = errorBoundary.match(/<button\b[^]*?<\/button>/g)?.find((button) => button.includes("Restart App"));
    expect(restartButton).toBeDefined();
    expect(restartButton).toContain('className="ab-btn-primary"');
    expect(errorBoundary).not.toContain("bg-blue-500");
  });
});
