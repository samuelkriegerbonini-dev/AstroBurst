import { readdirSync, readFileSync } from "node:fs";
import { posix } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const repoPath = (path: string) => fileURLToPath(new URL(`../../${path}`, import.meta.url));

const workflow = (file: string) => readFileSync(repoPath(`.github/workflows/${file}`), "utf8").replace(/\r\n/g, "\n");

const tauriConfig = () => JSON.parse(readFileSync(repoPath("src-tauri/tauri.conf.json"), "utf8"));

function jobs(text: string): Map<string, string> {
  const body = text.slice(text.indexOf("\njobs:\n") + "\njobs:\n".length);
  return new Map([...body.matchAll(/^ {2}([\w-]+):\n([\s\S]*?)(?=^ {2}[\w-]+:\n|(?![\s\S]))/gm)].map((match) => [match[1], match[2]]));
}

function steps(job: string | undefined): string[] {
  return (job ?? "").split(/^ {6}- /m).slice(1);
}

function tarballStep(): string {
  const found = steps(jobs(workflow("release.yml")).get("build")).filter((step) => step.includes("tar czf"));
  expect(found).toHaveLength(1);
  return found[0];
}

const withoutTrailingSlash = (path: string) => path.replace(/\/+$/, "");

describe("release pipeline", () => {
  it("lays the Linux tarball out so the extracted binary resolves the bundled tier 0", () => {
    const config = tauriConfig();
    const script = tarballStep().replace(/\$\{VERSION\}/g, "VERSION");
    const binaryName = posix.basename(/^\s*BIN=(\S+)$/m.exec(script)?.[1] ?? "");
    const binaryDest = /cp "\$BIN" (\S+)/.exec(script)?.[1] ?? "";
    const catalogueDest = /cp -r src-tauri\/resources\/catalogs (\S+)/.exec(script)?.[1] ?? "";
    const verified = /node scripts\/fetch-tier0\.mjs --verify (\S+)/.exec(script)?.[1] ?? "";
    expect([binaryName, binaryDest, catalogueDest, verified].every(Boolean), script).toBe(true);
    const binary = binaryDest.endsWith("/") ? `${binaryDest}${binaryName}` : binaryDest;
    const linuxResourceDir = posix.join(posix.dirname(binary), "..", "lib", config.productName);
    const probed = posix.join(linuxResourceDir, withoutTrailingSlash(config.bundle.resources["resources/catalogs/"]));
    expect(posix.join(catalogueDest, "catalogs")).toBe(probed);
    const created = (/mkdir -p (.+)/.exec(script)?.[1] ?? "").split(/\s+/).map(withoutTrailingSlash);
    expect(created).toContain(withoutTrailingSlash(catalogueDest));
    expect(withoutTrailingSlash(posix.normalize(verified))).toBe(probed);
    expect(script.indexOf("--verify")).toBeLessThan(script.indexOf("tar czf"));
  });

  it("names the tarball step after what it builds", () => {
    expect(tarballStep()).toMatch(/^name: Create Linux tarball\n/);
  });

  it("builds the release with the exact Rust release that CI tests", () => {
    const pins = ["release.yml", "ci.yml"].flatMap((file) =>
      [...workflow(file).matchAll(/dtolnay\/rust-toolchain@([\w.-]+)/g)].map((match) => `${file} ${match[1]}`),
    );
    expect(pins.length).toBeGreaterThanOrEqual(3);
    const releases = new Set(pins.map((pin) => pin.split(" ")[1]));
    expect([...releases], pins.join(", ")).toHaveLength(1);
    expect([...releases][0]).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it("runs the release jobs on the Node release that CI tests", () => {
    const versions = (file: string) => [...new Set([...workflow(file).matchAll(/node-version: (\S+)/g)].map((match) => match[1]))];
    expect(versions("ci.yml")).toHaveLength(1);
    expect(versions("release.yml")).toEqual(versions("ci.yml"));
  });

  it("runs the node:test files of scripts/ in the CI frontend job", () => {
    const manifest = JSON.parse(readFileSync(repoPath("package.json"), "utf8"));
    expect(manifest.scripts["test:scripts"]).toBe("node --test scripts/*.test.mjs");
    expect(readdirSync(repoPath("scripts")).filter((file) => file.endsWith(".test.mjs")).length).toBeGreaterThan(0);
    expect(steps(jobs(workflow("ci.yml")).get("frontend-tests"))).toContain("run: pnpm test:scripts\n");
  });

  it("checks the tag against the version fields and builds the release notes before the draft release exists", () => {
    const create = steps(jobs(workflow("release.yml")).get("create-release"));
    const notes = create.findIndex((step) => /run: node scripts\/release-notes\.mjs "\$\{GITHUB_REF_NAME\}" \S+\n/.test(step));
    const draft = create.findIndex((step) => step.includes("softprops/action-gh-release"));
    expect(notes).toBeGreaterThan(-1);
    expect(notes).toBeLessThan(draft);
    const out = /release-notes\.mjs "\$\{GITHUB_REF_NAME\}" (\S+)\n/.exec(create[notes])?.[1];
    expect(create[draft]).toContain(`body_path: ${out}\n`);
    expect(workflow("release.yml")).not.toMatch(/^\s+body:/m);
  });

  it("keeps the release token out of the checkouts and the build steps", () => {
    const text = workflow("release.yml");
    const checkouts = [...text.matchAll(/uses: actions\/checkout@\S+\n((?: {8}.*\n)*)/g)];
    expect(checkouts.length).toBeGreaterThan(0);
    for (const [, options] of checkouts) expect(options).toMatch(/^ {10}persist-credentials: false$/m);
    expect(text).not.toMatch(/secrets\.GITHUB_TOKEN|github\.token/);
  });

  it("fetches tier 0 before the frontend build, so an offline build stops before vite runs", () => {
    const commands = tauriConfig().build.beforeBuildCommand.split("&&").map((command: string) => command.trim());
    expect(commands).toEqual(["node scripts/fetch-tier0.mjs", "npm run build"]);
  });
});
