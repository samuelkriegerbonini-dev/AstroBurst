import { existsSync, readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, it, expect } from "vitest";

const NOTICES = "THIRD-PARTY-NOTICES.md";

const repoPath = (path: string) => fileURLToPath(new URL(`../../${path}`, import.meta.url));

const tauriPath = (path: string) => repoPath(`src-tauri/${path}`);

interface NoticeRow {
  name: string;
  version: string;
  files: string[];
}

interface LockPackage {
  name: string;
  version: string;
  dependencies: string[];
}

function noticeRows(): NoticeRow[] {
  return readFileSync(tauriPath(`licenses/${NOTICES}`), "utf8")
    .split(/\r?\n/)
    .filter((line) => line.startsWith("|") && !/^\|\s*:?-/.test(line))
    .map((line) => line.split("|").slice(1, -1).map((cell) => cell.trim()))
    .filter((cells) => cells[0] !== "Name")
    .map(([name, version, , , file]) => ({
      name,
      version,
      files: [...file.matchAll(/`([^`]+)`/g)].map((match) => match[1]),
    }));
}

function lockPackages(): LockPackage[] {
  return readFileSync(tauriPath("Cargo.lock"), "utf8")
    .split("[[package]]")
    .slice(1)
    .map((block) => ({
      name: /^name = "(.+)"$/m.exec(block)?.[1] ?? "",
      version: /^version = "(.+)"$/m.exec(block)?.[1] ?? "",
      dependencies: [...(/^dependencies = \[([\s\S]*?)\]/m.exec(block)?.[1] ?? "").matchAll(/"([^"]+)"/g)].map((match) => match[1]),
    }));
}

function lockedClosure(root: string): string[] {
  const packages = lockPackages();
  const resolve = (spec: string) => {
    const [name, version] = spec.split(" ");
    const found = packages.find((pkg) => pkg.name === name && (version === undefined || pkg.version === version));
    if (!found) throw new Error(`Cargo.lock has no package ${spec}`);
    return found;
  };
  const seen = new Set<string>();
  const visit = (pkg: LockPackage) => {
    const key = `${pkg.name} ${pkg.version}`;
    if (seen.has(key)) return;
    seen.add(key);
    pkg.dependencies.forEach((dependency) => visit(resolve(dependency)));
  };
  visit(resolve(root));
  return [...seen];
}

describe("third-party notices", () => {
  it("names at least one licence file that exists for every row", () => {
    const rows = noticeRows();
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(row.files, row.name).not.toHaveLength(0);
      for (const file of row.files) expect(existsSync(tauriPath(`licenses/${file}`)), `${row.name}: ${file}`).toBe(true);
    }
  });

  it("gives every file in the licences folder a row", () => {
    const named = new Set(noticeRows().flatMap((row) => row.files));
    const unnamed = readdirSync(tauriPath("licenses")).filter((file) => file !== NOTICES && !named.has(file));
    expect(unnamed).toEqual([]);
  });

  it("has a row at the locked version for fitsio-pure and every crate it pulls in", () => {
    const rows = noticeRows().map((row) => `${row.name} ${row.version}`);
    const locked = lockedClosure("fitsio-pure");
    expect(locked).toContain("fitsio-pure 0.21.6");
    expect(rows).toEqual(expect.arrayContaining(locked));
  });

  it("names only repository paths that exist", () => {
    const paths = [...readFileSync(tauriPath(`licenses/${NOTICES}`), "utf8").matchAll(/`((?:src|src-tauri)\/[^`]+)`/g)].map((match) => match[1]);
    expect(paths.length).toBeGreaterThan(0);
    expect(paths.filter((path) => !existsSync(repoPath(path)))).toEqual([]);
  });

  it("ships the licences folder with the desktop bundle", () => {
    const config = JSON.parse(readFileSync(tauriPath("tauri.conf.json"), "utf8"));
    expect(config.bundle.resources).toMatchObject({ "licenses/": "licenses/" });
  });

  it("ships the licences folder in the Linux release tarball", () => {
    const steps = readFileSync(repoPath(".github/workflows/release.yml"), "utf8").split(/^\s*- name: /m);
    const tarball = steps.find((step) => step.startsWith("Create Flatpak tarball")) ?? "";
    const copy = tarball.search(/cp -r src-tauri\/licenses astroburst-\$\{VERSION\}\//);
    expect(copy).toBeGreaterThan(-1);
    expect(copy).toBeLessThan(tarball.indexOf("tar czf"));
  });

  it("keeps the licence texts byte-identical across checkouts", () => {
    const rules = readFileSync(repoPath(".gitattributes"), "utf8")
      .split(/\r?\n/)
      .map((line) => line.trim().split(/\s+/));
    expect(rules).toContainEqual(["src-tauri/licenses/**", "-text"]);
  });
});
