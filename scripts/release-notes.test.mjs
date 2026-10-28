import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { releaseNotes, versionFields } from "./release-notes.mjs";

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), "release-notes.mjs");
const REPO_ROOT = join(dirname(SCRIPT), "..");

const CHANGELOG = `# Changelog

## [0.7.6] - 2028-10-25

### Added
- Something not released yet

## [1.2.0] - 2026-11-01

### Added
- Offline plate solving

### Fixed
- A crash

## [1.1.0] - 2026-10-01

### Added
- An older entry
`;

const LICENCE_NOTES = `# Licence paragraphs for the GitHub release notes

Intro that is not part of the notes.

## 1.2.0 and later

### Licences

Every installer carries the licences folder.

## Earlier releases with a Linux AppImage

### Licences of the Linux AppImage

Not for new releases.
`;

function fixture(work, { npm = "1.2.0", tauri = "1.2.0", cargo = "1.2.0", changelog = CHANGELOG, licences = LICENCE_NOTES } = {}) {
  mkdirSync(join(work, "src-tauri"), { recursive: true });
  mkdirSync(join(work, "docs"), { recursive: true });
  writeFileSync(join(work, "package.json"), JSON.stringify({ name: "fixture", version: npm }));
  writeFileSync(join(work, "src-tauri", "tauri.conf.json"), JSON.stringify({ productName: "Fixture", version: tauri }));
  writeFileSync(
    join(work, "src-tauri", "Cargo.toml"),
    `[package]\nname = "fixture"\nversion = "${cargo}"\n\n[dependencies.serde]\nversion = "9.9.9"\n`,
  );
  writeFileSync(join(work, "CHANGELOG.md"), changelog);
  writeFileSync(join(work, "docs", "release-notes-licences.md"), licences);
  return work;
}

function withWork(body) {
  const work = mkdtempSync(join(tmpdir(), "release-notes-"));
  try {
    return body(work);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

test("the notes are the changelog section of the tag's version followed by the licence paragraph for new releases", () =>
  withWork((work) => {
    const notes = releaseNotes("v1.2.0", fixture(work));
    assert.equal(
      notes,
      "### Added\n- Offline plate solving\n\n### Fixed\n- A crash\n\n### Licences\n\nEvery installer carries the licences folder.\n",
    );
  }));

test("a tag whose version one of the version fields does not carry is refused", () => {
  for (const [field, override, named] of [
    ["npm", { npm: "1.1.0" }, /package\.json says 1\.1\.0/],
    ["tauri", { tauri: "1.2.1" }, /src-tauri\/tauri\.conf\.json says 1\.2\.1/],
    ["cargo", { cargo: "1.0.0" }, /src-tauri\/Cargo\.toml says 1\.0\.0/],
  ]) {
    withWork((work) => {
      assert.throws(() => releaseNotes("v1.2.0", fixture(work, override)), named, field);
    });
  }
  withWork((work) => {
    assert.throws(() => releaseNotes("v1.3.0", fixture(work)), /the tag v1\.3\.0 names version 1\.3\.0/);
  });
});

test("a four-part maintenance tag takes its changelog section and needs the three-part version in the version fields", () => {
  const changelog = CHANGELOG.replace("## [1.2.0] - 2026-11-01", "## [1.2.0.1] - 2026-11-05\n\n### Fixed\n- A maintenance fix\n\n## [1.2.0] - 2026-11-01");
  withWork((work) => {
    assert.equal(
      releaseNotes("v1.2.0.1", fixture(work, { changelog })),
      "### Fixed\n- A maintenance fix\n\n### Licences\n\nEvery installer carries the licences folder.\n",
    );
  });
  withWork((work) => {
    assert.throws(() => releaseNotes("v1.2.1.1", fixture(work, { changelog })), /the tag v1\.2\.1\.1 names version 1\.2\.1/);
  });
  withWork((work) => {
    assert.throws(() => releaseNotes("v1.2.0.2", fixture(work, { changelog })), /no "## \[1\.2\.0\.2\]" section/);
  });
});

test("a version without changelog entries is refused, and the Unreleased section never stands in for it", () => {
  withWork((work) => {
    const changelog = CHANGELOG.replace("## [1.2.0] - 2026-11-01", "## [1.2.0-draft] - 2026-11-01");
    assert.throws(() => releaseNotes("v1.2.0", fixture(work, { changelog })), /CHANGELOG\.md has no "## \[1\.2\.0\]" section with entries/);
  });
  withWork((work) => {
    const changelog = "# Changelog\n\n## [1.2.0] - 2026-11-01\n\n## [1.1.0]\n\n- old\n";
    assert.throws(() => releaseNotes("v1.2.0", fixture(work, { changelog })), /no "## \[1\.2\.0\]" section with entries/);
  });
  withWork((work) => {
    assert.throws(() => releaseNotes("v1.2.0", fixture(work, { changelog: CHANGELOG.replace(/## \[1\.2\.0\][\s\S]*?(?=## \[1\.1\.0\])/, "") })), /no "## \[1\.2\.0\]"/);
  });
});

test("the notes are refused when the licence paragraph for new releases is missing", () =>
  withWork((work) => {
    const licences = LICENCE_NOTES.replace("## 1.2.0 and later", "## Current releases");
    assert.throws(() => releaseNotes("v1.2.0", fixture(work, { licences })), /no "## <version> and later" section/);
  }));

test("the command line writes the notes, and exits 1 with the reason when it refuses", () =>
  withWork((work) => {
    const root = fixture(join(work, "repo"));
    mkdirSync(join(root, "scripts"));
    const script = join(root, "scripts", "release-notes.mjs");
    copyFileSync(SCRIPT, script);
    const out = join(work, "notes.md");
    const written = spawnSync(process.execPath, [script, "v1.2.0", out], { encoding: "utf8" });
    assert.equal(written.status, 0, written.stderr);
    assert.equal(readFileSync(out, "utf8"), releaseNotes("v1.2.0", root));
    const refused = spawnSync(process.execPath, [script, "v9.9.9", join(work, "other.md")], { encoding: "utf8" });
    assert.equal(refused.status, 1);
    assert.match(refused.stderr, /release notes: the tag v9\.9\.9 names version 9\.9\.9/);
    assert.equal(existsSync(join(work, "other.md")), false);
    const usage = spawnSync(process.execPath, [script, "v1.2.0"], { encoding: "utf8" });
    assert.equal(usage.status, 1);
    assert.match(usage.stderr, /usage: node scripts\/release-notes\.mjs <tag> <output file>/);
  }));

test("the repository's own version fields agree and its licence notes have a paragraph for new releases", () => {
  const versions = Object.values(versionFields(REPO_ROOT));
  assert.equal(versions.length, 3);
  assert.equal(new Set(versions).size, 1, JSON.stringify(versionFields(REPO_ROOT)));
  const licences = readFileSync(join(REPO_ROOT, "docs", "release-notes-licences.md"), "utf8");
  assert.match(licences.replace(/\r\n/g, "\n"), /^## \S+ and later\n\n### Licences\n/m);
});
