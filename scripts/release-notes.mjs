import { readFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CHANGELOG = "CHANGELOG.md";
const LICENCE_NOTES = "docs/release-notes-licences.md";
const LICENCE_SECTION = /^## \S+ and later\n([\s\S]*?)(?=^## |(?![\s\S]))/m;
const USAGE = "usage: node scripts/release-notes.mjs <tag> <output file>";

class Refused extends Error {}

function read(root, path) {
  return readFileSync(join(root, path), "utf8").replace(/\r\n/g, "\n");
}

function escaped(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function cargoPackageVersion(manifest) {
  const table = /^\[package\]\n((?:(?!\[).*\n?)*)/m.exec(manifest)?.[1] ?? "";
  return /^version = "([^"]+)"$/m.exec(table)?.[1];
}

export function versionFields(root = REPO_ROOT) {
  return {
    "package.json": JSON.parse(read(root, "package.json")).version,
    "src-tauri/tauri.conf.json": JSON.parse(read(root, "src-tauri/tauri.conf.json")).version,
    "src-tauri/Cargo.toml": cargoPackageVersion(read(root, "src-tauri/Cargo.toml")),
  };
}

function changelogSection(changelog, version) {
  const section = new RegExp(`^## \\[${escaped(version)}\\][^\\n]*\\n([\\s\\S]*?)(?=^## |(?![\\s\\S]))`, "m").exec(changelog);
  return section?.[1].trim() ?? "";
}

export function releaseNotes(tag, root = REPO_ROOT) {
  const version = tag.replace(/^v/, "");
  const mismatched = Object.entries(versionFields(root)).filter(([, value]) => value !== version);
  if (mismatched.length > 0) {
    const fields = mismatched.map(([file, value]) => `${file} says ${value ?? "nothing"}`).join(", ");
    throw new Refused(`the tag ${tag} names version ${version}, but ${fields}`);
  }
  const changes = changelogSection(read(root, CHANGELOG), version);
  if (!/^\s*[-*] \S/m.test(changes)) {
    throw new Refused(`${CHANGELOG} has no "## [${version}]" section with entries; write it before tagging ${tag}`);
  }
  const licences = LICENCE_SECTION.exec(read(root, LICENCE_NOTES))?.[1].trim();
  if (!licences) throw new Refused(`${LICENCE_NOTES} has no "## <version> and later" section`);
  return `${changes}\n\n${licences}\n`;
}

function main(args) {
  if (args.length !== 2) throw new Refused(USAGE);
  const [tag, out] = args;
  writeFileSync(out, releaseNotes(tag));
  return `wrote the notes of ${tag} to ${out}`;
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  try {
    console.log(`release notes: ${main(process.argv.slice(2))}`);
  } catch (error) {
    console.error(`release notes: ${error instanceof Refused ? error.message : error.stack ?? error}`);
    process.exit(1);
  }
}
