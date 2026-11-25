import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const SRC = join(__dirname, "..", "..", "..");
const ROOTS = ["components/compose", "components/processing", "components/stacking", "components/synth", "utils/wizard.ts"];

function sources(path: string): string[] {
  if (statSync(path).isFile()) return /\.(ts|tsx)$/.test(path) ? [path] : [];
  return readdirSync(path)
    .filter((name) => name !== "__tests__")
    .flatMap((name) => sources(join(path, name)));
}

describe("Compose, Processing and Stacking text", () => {
  it("uses no em dash, which the project writing rule forbids", () => {
    const offenders = ROOTS.flatMap((root) => sources(join(SRC, root)))
      .flatMap((file) => readFileSync(file, "utf-8").split("\n").map((line, i) => ({ file, line, n: i + 1 })))
      .filter(({ line }) => line.includes("—"))
      .map(({ file, n }) => `${relative(SRC, file)}:${n}`);
    expect(offenders).toEqual([]);
  });
});
