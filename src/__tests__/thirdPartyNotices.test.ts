import { existsSync, readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, it, expect } from "vitest";

const NOTICES = "THIRD-PARTY-NOTICES.md";

const RUST_LIST = "THIRD-PARTY-LICENSES-RUST.md";

const NPM_LIST = "THIRD-PARTY-LICENSES-NPM.md";

const GENERATED_LISTS = [RUST_LIST, NPM_LIST];

const STANDARD_LIBRARY = "Rust standard library (std, core, alloc)";

const EXACT_RELEASE = /^\d+\.\d+\.\d+$/;

const RELEASE_TOOLCHAIN_CLAIM = /\bat the toolchain (version )?(of |that )?(the )?release (builds?|workflow)\b/i;

const RELEASE_NOTES = "docs/release-notes-licences.md";

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
  source: string;
  dependencies: string[];
}

interface ListEntry {
  name: string;
  version: string;
  licence: string;
  linkedInto: string;
  copyright: string;
  texts: string[];
}

interface RequiredNotice {
  list: string;
  name: string;
  notice: string;
}

const REQUIRED_NOTICES: RequiredNotice[] = [
  { list: RUST_LIST, name: "webview2-com-sys", notice: "Copyright (C) Microsoft Corporation. All rights reserved." },
  { list: RUST_LIST, name: "rav1e", notice: "Alliance for Open Media Patent License 1.0" },
  { list: RUST_LIST, name: "alloc-stdlib", notice: "Copyright (c) 2016 Dropbox, Inc." },
  { list: NPM_LIST, name: "isarray", notice: "Copyright (c) 2013 Julian Gruber" },
];

function generatedList(file: string): { entries: ListEntry[]; textBodies: Map<string, string> } {
  const path = tauriPath(`licenses/${file}`);
  const text = existsSync(path) ? readFileSync(path, "utf8").replace(/\r\n/g, "\n") : "";
  const table = /^## Entries\n([\s\S]*?)(?=^## |(?![\s\S]))/m.exec(text)?.[1] ?? "";
  const rows = table
    .split("\n")
    .filter((line) => line.startsWith("|") && !/^\|\s*:?-/.test(line))
    .map((line) => line.split(/(?<!\\)\|/).slice(1, -1).map((cell) => cell.trim()));
  const header = rows.find((cells) => cells[0] === "Name") ?? [];
  const cell = (cells: string[], title: string) => cells[header.indexOf(title)] ?? "";
  const entries = rows
    .filter((cells) => cells[0] !== "Name")
    .map((cells) => ({
      name: cell(cells, "Name"),
      version: cell(cells, "Version"),
      licence: cell(cells, "Licence"),
      linkedInto: cell(cells, "Linked into"),
      copyright: cell(cells, "Copyright"),
      texts: cell(cells, "Texts").split(/,\s*/).filter(Boolean),
    }));
  const sections = [...text.matchAll(/^### Text ([A-Z]\d+)\n[\s\S]*?^(`{3,})text\n([\s\S]*?)^\2$/gm)];
  const textBodies = new Map(sections.map((match) => [match[1], match[3].trim()]));
  return { entries, textBodies };
}

function releaseWorkflow(): string {
  return readFileSync(repoPath(".github/workflows/release.yml"), "utf8").replace(/\r\n/g, "\n");
}

function releaseToolchain(): string {
  const toolchains = [...new Set([...releaseWorkflow().matchAll(/dtolnay\/rust-toolchain@([\w.-]+)/g)].map((match) => match[1]))];
  expect(toolchains).toHaveLength(1);
  return toolchains[0];
}

function noticesText(): string {
  return readFileSync(tauriPath(`licenses/${NOTICES}`), "utf8").replace(/\r\n/g, "\n");
}

function appImageSection(): string {
  return /^## Linux AppImage\n([\s\S]*?)(?=^## |(?![\s\S]))/m.exec(noticesText())?.[1] ?? "";
}

function cargoField(field: string): string {
  const value = new RegExp(`^${field} = \\[?"([^"]+)"`, "m").exec(readFileSync(tauriPath("Cargo.toml"), "utf8"))?.[1];
  expect(value, field).toBeDefined();
  return value ?? "";
}

function markdownSections(text: string, level: string): Map<string, string> {
  const pattern = new RegExp(`^${level} (.+)\\n([\\s\\S]*?)(?=^${level} |(?![\\s\\S]))`, "gm");
  return new Map([...text.matchAll(pattern)].map((match) => [match[1].trim(), match[2]]));
}

function pnpmLockText(): string {
  return readFileSync(repoPath("pnpm-lock.yaml"), "utf8").replace(/\r\n/g, "\n");
}

function importerVersions(lock: string): Map<string, string> {
  const importer = /^importers:\n\n {2}\.:\n([\s\S]*?)\n\S/m.exec(lock)?.[1] ?? "";
  const pairs = [...importer.matchAll(/^ {6}'?([^':\n]+)'?:\n {8}specifier: .*\n {8}version: (.+)$/gm)];
  return new Map(pairs.map((match) => [match[1], match[2]]));
}

function snapshotDependencies(lock: string): Map<string, string[]> {
  const snapshots = lock.slice(lock.indexOf("\nsnapshots:\n"));
  const blocks = snapshots.split(/\n(?= {2}\S)/).slice(1);
  return new Map(
    blocks.map((block) => {
      const key = /^ {2}'?(.+?)'?:/.exec(block)?.[1] ?? "";
      const dependencies = [...block.matchAll(/^ {6}'?([^':\s]+)'?: (.+)$/gm)].map((match) => `${match[1]}@${match[2]}`);
      return [key, dependencies];
    }),
  );
}

function cssImportedPackages(): string[] {
  const walk = (dir: string): string[] =>
    readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
      const path = `${dir}/${entry.name}`;
      if (entry.isDirectory()) return walk(path);
      return entry.name.endsWith(".css") ? [path] : [];
    });
  return walk(repoPath("src")).flatMap((file) =>
    [...readFileSync(file, "utf8").matchAll(/@(?:import|plugin)\s+["']([^"'./][^"']*)["']/g)].map((match) =>
      match[1].split("/").slice(0, match[1].startsWith("@") ? 2 : 1).join("/"),
    ),
  );
}

function productionNpmPackages(): string[] {
  const lock = pnpmLockText();
  const versions = importerVersions(lock);
  const manifest = JSON.parse(readFileSync(repoPath("package.json"), "utf8"));
  const roots = [...Object.keys(manifest.dependencies), ...cssImportedPackages()].map((name) => {
    const version = versions.get(name);
    if (!version) throw new Error(`pnpm-lock.yaml has no importer entry for ${name}`);
    return `${name}@${version}`;
  });
  const dependencies = snapshotDependencies(lock);
  const seen = new Set<string>();
  const visit = (key: string) => {
    if (seen.has(key)) return;
    if (!dependencies.has(key)) throw new Error(`pnpm-lock.yaml has no snapshot ${key}`);
    seen.add(key);
    dependencies.get(key)?.forEach(visit);
  };
  roots.forEach(visit);
  return [...new Set([...seen].map((key) => key.split("(")[0]))].map((key) => {
    const at = key.lastIndexOf("@");
    return `${key.slice(0, at)} ${key.slice(at + 1)}`;
  });
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
      source: /^source = "(.+)"$/m.exec(block)?.[1] ?? "",
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
    const unnamed = readdirSync(tauriPath("licenses")).filter(
      (file) => file !== NOTICES && !GENERATED_LISTS.includes(file) && !named.has(file),
    );
    expect(unnamed).toEqual([]);
  });

  it("points from the notices to the complete Rust and npm lists by repository path", () => {
    const notices = readFileSync(tauriPath(`licenses/${NOTICES}`), "utf8");
    for (const file of GENERATED_LISTS) {
      expect(notices, file).toContain(`\`src-tauri/licenses/${file}\``);
      expect(existsSync(tauriPath(`licenses/${file}`)), file).toBe(true);
    }
  });

  it("lists every crate of Cargo.lock at its locked version", () => {
    const listed = new Set(generatedList(RUST_LIST).entries.map((entry) => `${entry.name} ${entry.version}`));
    const locked = lockPackages().filter((pkg) => pkg.source !== "");
    expect(locked.length).toBeGreaterThan(400);
    const missing = locked.map((pkg) => `${pkg.name} ${pkg.version}`).filter((key) => !listed.has(key));
    expect(missing, `${missing.length} of ${locked.length} locked crates have no entry in ${RUST_LIST}`).toEqual([]);
  });

  it("lists every npm package of the production dependency closure at its locked version", () => {
    const listed = new Set(generatedList(NPM_LIST).entries.map((entry) => `${entry.name} ${entry.version}`));
    const production = productionNpmPackages();
    expect(production.map((key) => key.split(" ")[0])).toEqual(expect.arrayContaining(["react", "jszip", "tailwindcss"]));
    const missing = production.filter((key) => !listed.has(key));
    expect(missing, `${missing.length} of ${production.length} production npm packages have no entry in ${NPM_LIST}`).toEqual([]);
  });

  it("gives every list entry a copyright cell and licence texts that the list contains", () => {
    for (const file of GENERATED_LISTS) {
      const { entries, textBodies } = generatedList(file);
      expect(entries.length, file).toBeGreaterThan(0);
      const untexted = entries.filter((entry) => entry.texts.length === 0 || entry.texts.some((id) => !textBodies.get(id)));
      expect(untexted.map((entry) => `${entry.name} ${entry.version}`), `${file}: entries without a licence text`).toEqual([]);
      const uncredited = entries.filter((entry) => entry.copyright === "");
      expect(uncredited.map((entry) => `${entry.name} ${entry.version}`), `${file}: entries without a copyright cell`).toEqual([]);
    }
  });

  it("carries the notices that the package files alone do not give", () => {
    const present = new Set([...lockPackages().map((pkg) => pkg.name), ...productionNpmPackages().map((key) => key.split(" ")[0])]);
    const missing = REQUIRED_NOTICES.filter((required) => present.has(required.name)).filter((required) => {
      const { entries, textBodies } = generatedList(required.list);
      const found = entries.filter((entry) => entry.name === required.name);
      return (
        found.length === 0 ||
        found.some((entry) => !entry.copyright.includes(required.notice) && !entry.texts.some((id) => textBodies.get(id)?.includes(required.notice)))
      );
    });
    expect(missing.map((required) => `${required.name}: ${required.notice}`)).toEqual([]);
  });

  it("has a row at the locked version for every crate it names from Cargo.lock", () => {
    const locked = lockPackages();
    const stale = noticeRows()
      .filter((row) => locked.some((pkg) => pkg.name === row.name))
      .filter((row) => !locked.some((pkg) => pkg.name === row.name && pkg.version === row.version))
      .map((row) => `${row.name} ${row.version}`);
    expect(stale).toEqual([]);
  });

  it("has a row at the locked version for vite, whose helpers the production build injects", () => {
    const locked = importerVersions(pnpmLockText()).get("vite")?.split("(")[0];
    expect(locked).toBeDefined();
    expect(noticeRows().find((row) => row.name === "vite")?.version).toBe(locked);
  });

  it("covers the system libraries that the Linux AppImage carries", () => {
    expect(releaseWorkflow()).toContain("*.AppImage");
    const rows = noticeRows().filter((row) => row.name.includes("AppImage"));
    expect(rows.map((row) => row.name)).not.toHaveLength(0);
    const texts = rows.flatMap((row) => row.files).map((file) => readFileSync(tauriPath(`licenses/${file}`), "utf8"));
    expect(texts.some((text) => /GNU LESSER GENERAL PUBLIC LICENSE\s+Version 2\.1/.test(text))).toBe(true);
  });

  it("makes a three-year written offer for the source of the LGPL libraries in the Linux AppImage", () => {
    const ubuntu = /- platform: ubuntu-(\d+\.\d+)\n\s+target: x86_64-unknown-linux-gnu$/m.exec(releaseWorkflow())?.[1];
    expect(ubuntu).toBeDefined();
    const section = appImageSection();
    const start = section.search(/^Written offer\b/m);
    expect(start).toBeGreaterThan(-1);
    const offer = section.slice(start);
    expect(offer).toMatch(/\bthree years\b/i);
    expect(offer).toMatch(/\bas long as\b[^.]*\boffered for download\b/i);
    expect(offer).toMatch(/complete corresponding source code/i);
    for (const library of ["WebKitGTK", "GTK 3", "libsoup 3", "linuxdeploy"]) expect(offer, library).toContain(library);
    expect(offer).toContain(`Ubuntu ${ubuntu}`);
    expect(offer).toContain(cargoField("authors"));
    expect(offer).toContain(`${cargoField("repository")}/issues`);
  });

  it("identifies the AppImage library versions by a record that outlives the build log", () => {
    const identification = appImageSection()
      .split("\n")
      .find((line) => line.startsWith("How the libraries and their versions are identified"));
    expect(identification).toBeDefined();
    expect(identification).toMatch(/\bGNU build ID\b/);
    expect(identification).toContain("Launchpad");
    expect(identification).not.toMatch(/published on the build date/);
  });

  it("gives the release notes a licence paragraph for new releases and one for earlier releases with an AppImage", () => {
    const sections = markdownSections(readFileSync(repoPath(RELEASE_NOTES), "utf8").replace(/\r\n/g, "\n"), "##");
    const pick = (pattern: RegExp) => [...sections].find(([heading]) => pattern.test(heading))?.[1] ?? "";
    const current = pick(/\band later\b/);
    const earlier = pick(/\bearlier releases\b/i);
    for (const [name, paragraph] of [
      ["new releases", current],
      ["earlier releases", earlier],
    ]) {
      expect(paragraph, name).toMatch(/\bthree years\b/i);
      expect(paragraph, name).toMatch(/complete corresponding source code/i);
      expect(paragraph, name).toContain(cargoField("authors"));
      expect(paragraph, name).toContain(`${cargoField("repository")}/issues`);
    }
    expect(current).toContain("`licenses/`");
    for (const file of [NOTICES, ...GENERATED_LISTS]) {
      expect(current, file).toContain(file);
      expect(existsSync(tauriPath(`licenses/${file}`)), file).toBe(true);
    }
    expect(earlier).toContain("https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html");
    expect(earlier).not.toMatch(/\bthis release\b/i);
    for (const file of GENERATED_LISTS) expect(earlier, file).not.toContain(file);
  });

  it("claims the release toolchain for the standard library only when the release workflow pins an exact release", () => {
    const list = readFileSync(tauriPath(`licenses/${RUST_LIST}`), "utf8").replace(/\r\n/g, "\n");
    const claims = [noticesText(), list.slice(0, list.indexOf("\n## "))]
      .flatMap((text) => text.split(/(?<=\.)\s+/))
      .filter((sentence) => /standard library/i.test(sentence) && RELEASE_TOOLCHAIN_CLAIM.test(sentence));
    const unpinnedClaims = EXACT_RELEASE.test(releaseToolchain()) ? [] : claims;
    expect(unpinnedClaims).toEqual([]);
  });

  it("prints a licence text once when another differs from it only in white space", () => {
    for (const file of GENERATED_LISTS) {
      const first = new Map<string, string>();
      const repeated: string[] = [];
      for (const [id, body] of generatedList(file).textBodies) {
        const key = body.replace(/\s+/g, " ").trim();
        const earlier = first.get(key);
        if (earlier) repeated.push(`${earlier} ${id}`);
        else first.set(key, id);
      }
      expect(repeated, file).toEqual([]);
    }
  });

  it("lists the Rust standard library, at the exact release when the release workflow pins one", () => {
    const toolchain = releaseToolchain();
    const { entries, textBodies } = generatedList(RUST_LIST);
    const found = entries.filter((entry) => entry.name === STANDARD_LIBRARY);
    expect(found.map((entry) => entry.name)).toEqual([STANDARD_LIBRARY]);
    const [entry] = found;
    expect(entry.version).toMatch(EXACT_RELEASE);
    if (/^\d/.test(toolchain)) expect(`${entry.version}.`.slice(0, toolchain.length + 1)).toBe(`${toolchain}.`);
    expect(entry.licence.split(" OR ").sort()).toEqual(["Apache-2.0", "MIT"]);
    expect(entry.linkedInto).toBe("Windows, Linux, macOS");
    expect(entry.copyright).toContain("The Rust Project Developers");
    expect(entry.copyright).toContain("COPYRIGHT-library.html");
    expect(entry.copyright).toMatch(/\b[\w-]+ \d+\.\d+\.\d+\S* \((MIT|Apache-2\.0)/);
    const bodies = entry.texts.map((id) => textBodies.get(id) ?? "");
    expect(bodies.some((body) => body.includes("Permission is hereby granted, free of charge"))).toBe(true);
    expect(bodies.some((body) => /Apache License\s+Version 2\.0/.test(body))).toBe(true);
    expect(bodies.some((body) => body.includes("UNICODE LICENSE V3"))).toBe(true);
    const list = readFileSync(tauriPath(`licenses/${RUST_LIST}`), "utf8");
    expect(list).toContain(`dtolnay/rust-toolchain@${toolchain}`);
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
    const steps = releaseWorkflow().split(/^\s*- name: /m);
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
