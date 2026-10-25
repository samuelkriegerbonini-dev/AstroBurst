import { execFileSync, spawnSync } from "node:child_process";
import { closeSync, copyFileSync, existsSync, mkdirSync, openSync, readdirSync, readFileSync, readSync, realpathSync, writeFileSync } from "node:fs";
import { join, posix } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

export const MANIFEST = "README.md";
const PACKAGES_DIR = "packages";
const ELF_MAGIC = Buffer.from([0x7f, 0x45, 0x4c, 0x46]);
const NATIVE_ARCH = "amd64";
const DPKG_FIELDS = "${binary:Package}\t${Version}\t${source:Package}\t${source:Version}\n";
const ROW = /^\| `([^`]+)` \| (\S+) \| .* \| `([^`]+)` \|$/gm;
const UNOWNED = /^- `([^`]+)`$/gm;
const USAGE = "usage: node scripts/appimage-licences.mjs collect <AppDir> <output folder> | check <AppDir> <folder inside the AppDir>, each with --skip <path> for the app's own files";

class Refused extends Error {}

function isElf(path) {
  const head = Buffer.alloc(ELF_MAGIC.length);
  const fd = openSync(path, "r");
  try {
    return readSync(fd, head, 0, head.length, 0) === head.length && head.equals(ELF_MAGIC);
  } finally {
    closeSync(fd);
  }
}

function skipped(relative, skip) {
  return skip.some((prefix) => relative === prefix || relative.startsWith(`${prefix}/`));
}

export function carriedFiles(appDir, skip) {
  const found = [];
  const walk = (relative) => {
    for (const entry of readdirSync(join(appDir, relative), { withFileTypes: true })) {
      const path = relative ? `${relative}/${entry.name}` : entry.name;
      if (skipped(path, skip)) continue;
      if (entry.isDirectory()) walk(path);
      else if (entry.isFile() && (path.startsWith("usr/bin/") || isElf(join(appDir, path)))) found.push(path);
    }
  };
  walk("");
  return found.sort();
}

export function parseOwners(output, name) {
  return output
    .split("\n")
    .filter((line) => line !== "" && !line.startsWith("diversion by "))
    .flatMap((line) => {
      const at = line.indexOf(": ");
      const path = line.slice(at + 2).trim();
      if (at < 0 || !path.endsWith(`/${name}`)) return [];
      return line.slice(0, at).split(", ").map((owner) => ({ package: owner.trim(), path }));
    });
}

export function parsePackages(output) {
  return new Map(
    output
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        const [name, version, source, sourceVersion] = line.split("\t");
        return [name, { version, source, sourceVersion }];
      }),
  );
}

const dpkgQuery = {
  owners(name) {
    const run = spawnSync("dpkg-query", ["-S", `*/${name}`], { encoding: "utf8" });
    if (run.error) throw run.error;
    if (run.status !== 0 && !/no path found/.test(run.stderr)) throw new Refused(`dpkg-query -S failed for ${name}: ${run.stderr.trim()}`);
    return parseOwners(run.stdout, name);
  },
  packages(names) {
    return parsePackages(execFileSync("dpkg-query", ["-W", `-f=${DPKG_FIELDS}`, ...names], { encoding: "utf8" }));
  },
};

function isNative(name) {
  const arch = /:([\w-]+)$/.exec(name)?.[1];
  return arch === undefined || arch === NATIVE_ARCH;
}

const docName = (name) => name.replace(/:[\w-]+$/, "");

export function ownersOf(file, owners) {
  const native = owners.filter((owner) => isNative(owner.package));
  const exact = native.filter((owner) => owner.path === `/${file}`);
  if (exact.length > 0) return exact;
  const systemDir = /^usr\/s?bin\//.test(file) ? /^\/(usr\/)?s?bin\// : /^\/(usr\/)?lib(32|64|x32)?\//;
  return native.filter((owner) => systemDir.test(owner.path));
}

function manifestText(system, rows, unowned) {
  const lines = [
    "# System files in the AstroBurst Linux AppImage",
    "",
    `The release job built this AppImage on ${system} and lists here every library and command it carries outside AstroBurst's own files. Each row names the Ubuntu package the file comes from, its version and its source package; \`${PACKAGES_DIR}/<package>/copyright\` is that package's copyright file as installed on the build machine, with the package's licence terms and notices. The source code of each version is the Ubuntu source package of the same name and version.`,
    "",
    "| File in the AppImage | Package | Version | Source package | Copyright file |",
    "|---|---|---|---|---|",
    ...rows.map((row) => `| \`${row.file}\` | ${row.package} | ${row.version} | ${row.source} ${row.sourceVersion} | \`${PACKAGES_DIR}/${docName(row.package)}/copyright\` |`),
    "",
  ];
  if (unowned.length > 0) {
    lines.push("Files that no Ubuntu package owns:", "", ...unowned.map((file) => `- \`${file}\``), "");
  }
  return lines.join("\n");
}

export function collect({ appDir, out, skip, query = dpkgQuery, docRoot = "/usr/share/doc", system }) {
  const files = carriedFiles(appDir, skip);
  if (files.length === 0) throw new Refused(`${appDir} carries no file outside the app's own`);
  const owned = files.map((file) => ({ file, owners: ownersOf(file, query.owners(posix.basename(file))) }));
  const names = [...new Set(owned.flatMap(({ owners }) => owners.map((owner) => owner.package)))].sort();
  const versions = names.length > 0 ? query.packages(names) : new Map();
  mkdirSync(join(out, PACKAGES_DIR), { recursive: true });
  for (const name of names) {
    const copyright = join(docRoot, docName(name), "copyright");
    if (!existsSync(copyright)) throw new Refused(`${name} has no copyright file at ${copyright}`);
    if (!versions.get(name)) throw new Refused(`dpkg-query gave no version for ${name}`);
    mkdirSync(join(out, PACKAGES_DIR, docName(name)), { recursive: true });
    copyFileSync(copyright, join(out, PACKAGES_DIR, docName(name), "copyright"));
  }
  const rows = owned.flatMap(({ file, owners }) =>
    [...new Set(owners.map((owner) => owner.package))].sort().map((name) => ({ file, package: name, ...versions.get(name) })),
  );
  const unowned = owned.filter(({ owners }) => owners.length === 0).map(({ file }) => file);
  writeFileSync(join(out, MANIFEST), manifestText(system, rows, unowned));
  return `listed ${files.length} files from ${names.length} packages in ${join(out, MANIFEST)}`;
}

export function check({ appDir, folder, skip }) {
  const manifestPath = join(appDir, folder, MANIFEST);
  if (!existsSync(manifestPath)) throw new Refused(`the AppImage does not carry ${posix.join(folder, MANIFEST)}`);
  const manifest = readFileSync(manifestPath, "utf8");
  const rows = [...manifest.matchAll(ROW)];
  const missingCopyright = rows.map((row) => row[3]).filter((file) => !existsSync(join(appDir, folder, file)));
  const listed = new Set([...rows.map((row) => row[1]), ...[...manifest.matchAll(UNOWNED)].map((match) => match[1])]);
  const carried = carriedFiles(appDir, skip);
  const unlisted = carried.filter((file) => !listed.has(file));
  const vanished = [...listed].filter((file) => !carried.includes(file));
  const problems = [
    unlisted.length > 0 && `not listed: ${unlisted.join(", ")}`,
    vanished.length > 0 && `listed but not carried: ${vanished.join(", ")}`,
    missingCopyright.length > 0 && `missing copyright files: ${[...new Set(missingCopyright)].join(", ")}`,
  ].filter(Boolean);
  if (problems.length > 0) throw new Refused(`${posix.join(folder, MANIFEST)} does not match the AppImage: ${problems.join("; ")}`);
  return `${posix.join(folder, MANIFEST)} lists all ${carried.length} files the AppImage carries outside the app's own`;
}

function systemName() {
  const release = existsSync("/etc/os-release") ? readFileSync("/etc/os-release", "utf8") : "";
  return /^PRETTY_NAME="?([^"\n]+)"?$/m.exec(release)?.[1] ?? "an unknown system";
}

function main(args) {
  let parsed;
  try {
    parsed = parseArgs({ args, options: { skip: { type: "string", multiple: true } }, allowPositionals: true, strict: true });
  } catch (error) {
    throw new Refused(`${error.message}; ${USAGE}`);
  }
  const [command, appDir, target, ...rest] = parsed.positionals;
  const skip = parsed.values.skip ?? [];
  if (rest.length > 0 || !appDir || !target || skip.length === 0) throw new Refused(USAGE);
  if (command === "collect") return collect({ appDir, out: target, skip, system: systemName() });
  if (command === "check") return check({ appDir, folder: target, skip });
  throw new Refused(USAGE);
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  try {
    console.log(`AppImage licences: ${main(process.argv.slice(2))}`);
  } catch (error) {
    console.error(`AppImage licences: ${error instanceof Refused ? error.message : error.stack ?? error}`);
    process.exit(1);
  }
}
