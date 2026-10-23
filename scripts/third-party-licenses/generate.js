import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("../../", import.meta.url));
const tauri = join(repo, "src-tauri");
const output = join(tauri, "licenses");
const spdx = fileURLToPath(new URL("./spdx/", import.meta.url));
const command = "node scripts/third-party-licenses/generate.js";

const TARGETS = [
  { triple: "x86_64-pc-windows-msvc", os: "Windows" },
  { triple: "x86_64-unknown-linux-gnu", os: "Linux" },
  { triple: "aarch64-apple-darwin", os: "macOS" },
  { triple: "x86_64-apple-darwin", os: "macOS" },
];
const OS_ORDER = ["Windows", "Linux", "macOS"];
const PREFERENCE = ["MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "ISC", "Zlib", "0BSD", "Unlicense", "MPL-2.0"];
const LICENCE_FILE = /^(licen[cs]e|copying|copyright|notice|patents|unlicense)/i;
const HAND_TEXTS = new Map([
  ["alloc-stdlib", { file: "alloc-stdlib-LICENSE", forCrate: true, note: "the LICENSE of its repository, which the crate does not ship" }],
  [
    "webview2-com-sys",
    { file: "webview2-loader-LICENSE.txt", forCrate: false, note: "for WebView2LoaderStatic.lib, which the Windows build links statically" },
  ],
]);
const STANDARD_LIBRARY = "Rust standard library (std, core, alloc)";
const RELEASE_WORKFLOW = ".github/workflows/release.yml";
const EXACT_RELEASE = /^\d+\.\d+\.\d+$/;
const LICENCE_DIR = /^licen[cs]es$/i;
const SKIPPED_DIRS = new Set(["tests", "test", "testdata", "benches", "examples", "fixtures", "target", "node_modules"]);
const CODE_FILE = /\.(rs|c|h|cc|cpp|hpp|py|sh|toml|json|ya?ml|html?|js|mjs|cjs|ts|in|am|ac|cmake|bat|ps1)$/i;

function compareText(a, b) {
  return a < b ? -1 : a > b ? 1 : 0;
}

function compareVersion(a, b) {
  const left = a.split(/[.+-]/);
  const right = b.split(/[.+-]/);
  for (let i = 0; i < Math.max(left.length, right.length); i += 1) {
    const x = left[i] ?? "";
    const y = right[i] ?? "";
    const order = /^\d+$/.test(x) && /^\d+$/.test(y) ? Number(x) - Number(y) : compareText(x, y);
    if (order !== 0) return order;
  }
  return 0;
}

function compareEntries(a, b) {
  return compareText(a.name, b.name) || compareVersion(a.version, b.version);
}

function normalise(text) {
  return (text.charCodeAt(0) === 0xfeff ? text.slice(1) : text)
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.replace(/\s+$/, ""))
    .join("\n")
    .replace(/^\n+|\n+$/g, "");
}

function cell(value) {
  return value.replace(/\s+/g, " ").replace(/\|/g, "\\|").trim();
}

function fence(text) {
  const longest = Math.max(0, ...[...text.matchAll(/`+/g)].map((match) => match[0].length));
  return "`".repeat(Math.max(3, longest + 1));
}

function standardText(id) {
  const path = join(spdx, `${id}.txt`);
  if (!existsSync(path)) throw new Error(`no SPDX standard text for ${id} in ${spdx}`);
  return readFileSync(path, "utf8");
}

function splitTopLevel(expression, operator) {
  const parts = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < expression.length; i += 1) {
    if (expression[i] === "(") depth += 1;
    if (expression[i] === ")") depth -= 1;
    if (depth === 0 && expression.startsWith(operator, i)) {
      parts.push(expression.slice(start, i));
      start = i + operator.length;
    }
  }
  parts.push(expression.slice(start));
  return parts.map((part) => part.trim());
}

function electedLicences(expression) {
  const normalised = expression.replace(/\s*\/\s*/g, " OR ");
  const choices = splitTopLevel(normalised, " OR ").map((choice) =>
    choice
      .replace(/[()]/g, " ")
      .split(/\s+/)
      .filter((token) => token && !["AND", "OR", "WITH"].includes(token)),
  );
  const rank = (ids) => Math.max(...ids.map((id) => (PREFERENCE.includes(id) ? PREFERENCE.indexOf(id) : PREFERENCE.length)));
  return choices.reduce((best, ids) => (rank(ids) < rank(best) ? ids : best));
}

function copyrightLines(texts) {
  const lines = [];
  for (const text of texts) {
    for (const raw of text.split("\n")) {
      const line = raw
        .replace(/^[\s#*/;!-]+/, "")
        .replace(/^PackageCopyrightText:\s*/, "Copyright ")
        .replace(/\s+/g, " ")
        .trim();
      const dated = /^(copyright\b.*(\d{4}|\(c\)|©)|©|\(c\)\s*\d)/i.test(line);
      const named = /^Copyright (?!(Notice|Owner|Holders?|Law|Licen[cs]e|Statement)\b)[A-Z]/.test(line);
      if (!dated && !named) continue;
      if (/[<[{]\s*(yyyy|year|name|owner|copyright holder|fullname)/i.test(line)) continue;
      if (/Free Software Foundation/i.test(line)) continue;
      if (!lines.includes(line)) lines.push(line);
    }
  }
  return lines;
}

function authorNames(authors) {
  return authors.map((author) => author.replace(/\s*<[^>]*>/g, "").replace(/\s*\([^)]*\)/g, "").trim()).filter(Boolean);
}

function licenceFilesIn(root, nested) {
  const found = [];
  const walk = (dir, depth) => {
    const entries = readdirSync(dir, { withFileTypes: true }).sort((a, b) => compareText(a.name, b.name));
    for (const entry of entries) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (LICENCE_DIR.test(entry.name)) {
          readdirSync(path, { withFileTypes: true })
            .filter((file) => file.isFile())
            .forEach((file) => found.push(join(path, file.name)));
        } else if (nested && depth < 4 && !entry.name.startsWith(".") && !SKIPPED_DIRS.has(entry.name.toLowerCase())) {
          walk(path, depth + 1);
        }
      } else if (entry.isFile() && LICENCE_FILE.test(entry.name) && !CODE_FILE.test(entry.name)) {
        found.push(path);
      }
    }
  };
  walk(root, 0);
  return found;
}

function collectTexts(root, declaredFile, nested) {
  const paths = licenceFilesIn(root, nested);
  if (declaredFile) {
    const declared = join(root, declaredFile);
    if (existsSync(declared) && !paths.includes(declared)) paths.push(declared);
  }
  return [...new Set(paths)]
    .map((path) => ({ file: relative(root, path).split(sep).join("/"), text: normalise(readFileSync(path, "utf8")) }))
    .filter((item) => item.text.length > 0)
    .sort((a, b) => compareText(a.file, b.file));
}

class TextStore {
  constructor(prefix) {
    this.prefix = prefix;
    this.byHash = new Map();
    this.ordered = [];
  }

  add(text, user) {
    const hash = createHash("sha256").update(text.replace(/\s+/g, " ").trim()).digest("hex");
    let record = this.byHash.get(hash);
    if (!record) {
      record = { id: `${this.prefix}${String(this.ordered.length + 1).padStart(3, "0")}`, text, users: [] };
      this.byHash.set(hash, record);
      this.ordered.push(record);
    }
    if (!record.users.includes(user)) record.users.push(user);
    return record.id;
  }
}

function handTexts(name, version) {
  const hand = HAND_TEXTS.get(name);
  if (!hand) return [];
  const path = join(output, hand.file);
  if (!existsSync(path)) throw new Error(`${hand.file} for ${name} is missing from ${output}`);
  return [
    {
      file: hand.file,
      text: normalise(readFileSync(path, "utf8")),
      forCrate: hand.forCrate,
      note: hand.note,
      user: `${name} ${version} (\`${hand.file}\` in this folder, ${hand.note})`,
    },
  ];
}

function attachTexts(entry, store, shipped, expression, hand = []) {
  const own = [...shipped, ...hand.filter((item) => item.forCrate)];
  const extra = hand.filter((item) => !item.forCrate);
  entry.texts = [...own, ...extra].map((item) => store.add(item.text, item.user ?? `${entry.name} ${entry.version} \`${item.file}\``));
  entry.copyright = copyrightLines(own.map((item) => item.text));
  entry.extraCopyright = extra.flatMap((item) => copyrightLines([item.text]).map((line) => `${line} (${item.note})`));
  if (!own.every((item) => item.file.toLowerCase().endsWith(".spdx"))) return;
  if (!expression) throw new Error(`${entry.name} ${entry.version} ships no licence text and declares no licence`);
  entry.texts.push(
    ...electedLicences(expression).map((id) =>
      store.add(normalise(standardText(id)), `${entry.name} ${entry.version} (SPDX standard ${id} text; the package ships no licence text)`),
    ),
  );
}

function finishCopyright(entry, authors) {
  if (entry.copyright.length === 0) {
    const names = authorNames(authors);
    entry.copyright = names.length > 0 ? [`Authors: ${names.join(", ")}`] : ["No copyright line in the package's files"];
  }
  entry.copyright.push(...entry.extraCopyright);
}

function readmeLicenceSection(dir) {
  const readme = readdirSync(dir)
    .sort(compareText)
    .find((name) => /^readme(\.(md|markdown|txt))?$/i.test(name));
  if (!readme) return "";
  const lines = normalise(readFileSync(join(dir, readme), "utf8"))
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&")
    .split("\n");
  const start = lines.findIndex((line) => /^#{1,6}\s*licen[cs]e\b/i.test(line));
  if (start < 0) return "";
  const level = /^#+/.exec(lines[start])[0].length;
  const end = lines.findIndex((line, index) => index > start && /^#+\s/.test(line) && /^#+/.exec(line)[0].length <= level);
  return lines.slice(start + 1, end < 0 ? lines.length : end).join("\n");
}

function summaryTable(entries, columns) {
  const counts = new Map();
  for (const entry of entries) {
    const row = counts.get(entry.licence) ?? columns.map(() => 0);
    columns.forEach((column, index) => {
      if (column.counts(entry)) row[index] += 1;
    });
    counts.set(entry.licence, row);
  }
  const header = `| Licence | ${columns.map((column) => column.title).join(" | ")} |`;
  const rule = `|---|${columns.map(() => "---").join("|")}|`;
  const rows = [...counts]
    .sort((a, b) => b[1][b[1].length - 1] - a[1][a[1].length - 1] || compareText(a[0], b[0]))
    .map(([licence, row]) => `| ${cell(licence)} | ${row.join(" | ")} |`);
  return [header, rule, ...rows].join("\n");
}

function textsSection(store) {
  return store.ordered
    .map((record) => {
      const marker = fence(record.text);
      return [`### Text ${record.id}`, "", `Used by: ${record.users.join(", ")}.`, "", `${marker}text`, record.text, marker].join("\n");
    })
    .join("\n\n");
}

function cargo(args) {
  return execFileSync("cargo", args, { cwd: tauri, encoding: "utf8", maxBuffer: 1 << 28, stdio: ["ignore", "pipe", "inherit"] });
}

function treeKeys(triple, edges) {
  return new Set(
    cargo(["tree", "--locked", "-e", edges, "--target", triple, "--prefix", "none", "--format", "{p}"])
      .split(/\r?\n/)
      .map((line) => /^(\S+) v(\S+)/.exec(line))
      .filter(Boolean)
      .map((match) => `${match[1]} ${match[2]}`),
  );
}

function rustc(args) {
  return execFileSync("rustc", args, { cwd: tauri, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
}

function releaseToolchain() {
  const workflow = readFileSync(join(repo, RELEASE_WORKFLOW), "utf8");
  const refs = [...new Set([...workflow.matchAll(/dtolnay\/rust-toolchain@([\w.-]+)/g)].map((match) => match[1]))];
  if (refs.length !== 1) throw new Error(`${RELEASE_WORKFLOW} should install one Rust toolchain with dtolnay/rust-toolchain, found ${refs.length}`);
  const [channel] = refs;
  const release = /^release: (\S+)$/m.exec(rustc(["-vV"]))[1];
  const exact = EXACT_RELEASE.test(channel);
  const matches = exact ? release === channel : channel === "stable" ? EXACT_RELEASE.test(release) : /^\d+\.\d+$/.test(channel) && release.startsWith(`${channel}.`);
  if (!matches) throw new Error(`rustc ${release} is not the toolchain dtolnay/rust-toolchain@${channel} installs in ${RELEASE_WORKFLOW}; run the generator with that toolchain`);
  return { channel, release, exact, docs: join(rustc(["--print", "sysroot"]).trim(), "share", "doc", "rust") };
}

function decodeHtml(text) {
  return text
    .replace(/&#(\d+);/g, (_, code) => String.fromCodePoint(Number(code)))
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&amp;/g, "&");
}

function htmlFields(chunk, label) {
  return [...chunk.matchAll(new RegExp(`<b>${label}:</b>([^<]*)<`, "g"))].map((match) => decodeHtml(match[1]).trim()).filter(Boolean);
}

function standardLibraryCopyright(docs) {
  const path = join(docs, "COPYRIGHT-library.html");
  if (!existsSync(path)) throw new Error(`${path} is missing; the rustc component of the toolchain installs it`);
  const html = readFileSync(path, "utf8").replace(/\r\n?/g, "\n");
  const inTree = html.indexOf('id="in-tree-files"');
  const outOfTree = html.indexOf('id="out-of-tree-dependencies"');
  if (inTree < 0 || outOfTree < inTree) throw new Error(`${path} has no in-tree and out-of-tree sections`);
  const files = html
    .slice(inTree, outOfTree)
    .split("<b>File/Directory:</b>")
    .slice(1)
    .map((chunk) => ({
      path: decodeHtml(/<code>([^<]*)<\/code>/.exec(chunk)[1]),
      licence: htmlFields(chunk, "License")[0],
      copyright: htmlFields(chunk, "Copyright"),
    }));
  const crates = html
    .slice(outOfTree)
    .split("<h3>")
    .slice(1)
    .map((chunk) => {
      const [, name, version] = /^\S+\s+(.+?)-(\d[^<]*)<\/h3>/.exec(chunk.trim());
      return {
        name,
        version,
        licence: htmlFields(chunk, "License")[0],
        authors: htmlFields(chunk, "Authors").flatMap((authors) => authors.split(/,\s*/)),
        notices: [...chunk.matchAll(/<summary><code>([^<]+)<\/code><\/summary>\s*<pre>([\s\S]*?)<\/pre>/g)].map((match) => ({
          file: decodeHtml(match[1]),
          text: normalise(decodeHtml(match[2])),
        })),
      };
    });
  if (files[0]?.path !== "." || crates.length === 0) throw new Error(`${path} lists no library root or no out-of-tree crate`);
  return { files, crates };
}

function standardLibraryEntry(store) {
  const toolchain = releaseToolchain();
  const { files, crates } = standardLibraryCopyright(toolchain.docs);
  const user = `${STANDARD_LIBRARY} ${toolchain.release}`;
  const ids = [...new Set(files.flatMap((file) => file.licence.replace(/[()]/g, " ").split(/\s+/)))].filter(
    (token) => token && !["AND", "OR", "WITH"].includes(token),
  );
  const texts = ids.map((id) => {
    const path = join(toolchain.docs, "licenses", `${id}.txt`);
    if (!existsSync(path)) throw new Error(`${path} is missing for the ${id} files of the standard library`);
    return store.add(normalise(readFileSync(path, "utf8")), `${user} (\`share/doc/rust/licenses/${id}.txt\` of the toolchain)`);
  });
  for (const crate of crates) {
    const shipped = crate.notices.map((notice) => store.add(notice.text, `${user}: ${crate.name} ${crate.version} \`${notice.file}\``));
    const standard =
      shipped.length > 0
        ? []
        : electedLicences(crate.licence).map((id) =>
            store.add(normalise(standardText(id)), `${user}: ${crate.name} ${crate.version} (SPDX standard ${id} text; the toolchain gives no licence text)`),
          );
    texts.push(...shipped, ...standard);
  }
  const [root, ...exceptions] = files;
  const credit = (crate) => {
    const lines = copyrightLines(crate.notices.map((notice) => notice.text));
    if (lines.length > 0) return lines.join(", ");
    const names = authorNames(crate.authors);
    return names.length > 0 ? `Authors: ${names.join(", ")}` : "no copyright line or author given";
  };
  return {
    name: STANDARD_LIBRARY,
    version: toolchain.release,
    licence: root.licence,
    linkedInto: OS_ORDER.join(", "),
    source: "https://github.com/rust-lang/rust",
    channel: toolchain.channel,
    exact: toolchain.exact,
    texts: [...new Set(texts)],
    copyright: [
      ...root.copyright,
      ...exceptions.map((file) => `\`${file.path}\` (${file.licence}): ${file.copyright.join(", ")}`),
      `out-of-tree crates as the toolchain's \`share/doc/rust/COPYRIGHT-library.html\` lists them, for every target and including the crates only the library's build and tests use: ${crates
        .map((crate) => `${crate.name} ${crate.version} (${crate.licence}; ${credit(crate)})`)
        .join(", ")}`,
    ],
  };
}

function toolchainNote({ channel, version, exact }) {
  const step = `\`${RELEASE_WORKFLOW}\` installs the toolchain of the release builds with \`dtolnay/rust-toolchain@${channel}\``;
  if (exact) return `${step}, that release exactly, and the generator stops when the \`rustc\` it runs is another release.`;
  const family = channel === "stable" ? "stable" : channel;
  return `${step}, which names no exact release: a release build made after a newer ${family} release came out uses that newer release, and this entry does not describe its library. The generator checks only that the \`rustc\` it runs is a ${family} release; with an exact release in the workflow (\`dtolnay/rust-toolchain@${version}\`) it requires that release.`;
}

function lockKeys() {
  return readFileSync(join(tauri, "Cargo.lock"), "utf8")
    .split("[[package]]")
    .slice(1)
    .filter((block) => /^source = /m.test(block))
    .map((block) => `${/^name = "(.+)"$/m.exec(block)[1]} ${/^version = "(.+)"$/m.exec(block)[1]}`);
}

function rustList() {
  const metadata = JSON.parse(cargo(["metadata", "--locked", "--all-features", "--format-version", "1"]));
  const packages = metadata.packages.filter((pkg) => pkg.source !== null);
  const known = new Set(packages.map((pkg) => `${pkg.name} ${pkg.version}`));
  const locked = lockKeys();
  const absent = locked.filter((key) => !known.has(key));
  if (absent.length > 0) throw new Error(`cargo metadata lacks locked crates: ${absent.join(", ")}`);
  const linked = new Map();
  const compileTime = new Set();
  for (const target of TARGETS) {
    const linkedHere = treeKeys(target.triple, "normal,no-proc-macro");
    linkedHere.forEach((key) => linked.set(key, new Set([...(linked.get(key) ?? []), target.os])));
    treeKeys(target.triple, "normal").forEach((key) => {
      if (!linkedHere.has(key)) compileTime.add(key);
    });
  }
  const store = new TextStore("R");
  const lockedSet = new Set(locked);
  const crates = packages
    .filter((pkg) => lockedSet.has(`${pkg.name} ${pkg.version}`))
    .map((pkg) => ({ pkg, name: pkg.name, version: pkg.version }))
    .sort(compareEntries)
    .map(({ pkg, name, version }) => {
      const key = `${name} ${version}`;
      const systems = OS_ORDER.filter((os) => linked.get(key)?.has(os));
      const entry = {
        name,
        version,
        licence: pkg.license ?? (pkg.license_file ? `Custom (\`${pkg.license_file}\`)` : "Not declared"),
        linkedInto: systems.length > 0 ? systems.join(", ") : compileTime.has(key) ? "compile time" : "no",
        source: pkg.repository ?? pkg.homepage ?? `https://crates.io/crates/${name}`,
      };
      attachTexts(entry, store, collectTexts(dirname(pkg.manifest_path), pkg.license_file, true), pkg.license, handTexts(name, version));
      finishCopyright(entry, pkg.authors ?? []);
      return entry;
    });
  const standardLibrary = standardLibraryEntry(store);
  const entries = [standardLibrary, ...crates];
  const linkedCount = crates.filter((entry) => !["compile time", "no"].includes(entry.linkedInto)).length;
  const lines = [
    "# Third-party licences: Rust crates",
    "",
    `Generated by \`${command}\` from \`src-tauri/Cargo.lock\`, the crate sources in the Cargo registry and the Rust toolchain. Do not edit by hand: run the command again after any change to \`Cargo.lock\` and with each new toolchain the release builds use.`,
    "",
    `Scope: the Rust standard library and every crate in \`src-tauri/Cargo.lock\` except AstroBurst itself, ${crates.length} crates. ${linkedCount} of the crates are linked into a release build of the desktop app. "Linked into" names the release builds whose \`astroburst\` binary links the crate with the default features (\`cargo tree -e normal,no-proc-macro --target <triple>\` for ${TARGETS.map((target) => target.triple).join(", ")}; macOS is the universal build of the two Apple targets). "compile time" marks procedural macros and the crates only they use: they run during the build and are not linked. "no" marks build-script, test-only, headless-server-only and other-platform crates; they are listed so that the list covers the whole lock file.`,
    "",
    `Rust standard library: the first entry. Every release build links \`std\`, \`core\` and \`alloc\` into the \`astroburst\` binary, but they come with the Rust toolchain, not from \`Cargo.lock\`. The entry's version is the \`rustc\` release this list was generated with (\`rustc -vV\` in \`src-tauri\`). ${toolchainNote(standardLibrary)} The copyright cell and texts follow the toolchain's own \`share/doc/rust/COPYRIGHT-library.html\`: the library's licence, the in-tree files under other terms, and the out-of-tree crates built into the library for any target, with the licence texts it reproduces and, for the library's own licences, the texts in the toolchain's \`share/doc/rust/licenses\` folder.`,
    "",
    `Licence texts: each entry names the texts the crate ships: files named LICENSE, LICENCE, COPYING, COPYRIGHT, NOTICE, PATENTS or UNLICENSE at the crate root or in its source folders (tests, benches and examples excluded), any \`LICENSES\` folder and the manifest's \`license-file\`. ${[...HAND_TEXTS].map(([name, hand]) => `${name} also names \`${hand.file}\` from this folder (${hand.note})`).join("; ")}; \`THIRD-PARTY-NOTICES.md\` explains these files. Where a crate ships no licence text (none, or only an SPDX metadata file) and no file of this folder stands in for it, the entry adds the SPDX standard text of the licence taken from its expression (MIT where the expression offers it). The copyright column repeats the copyright lines of those files; where they have none it gives the authors from the manifest. Texts that differ only in white space are printed once under Texts, as their first user ships them.`,
    "",
    "## Licence summary",
    "",
    summaryTable(entries, [
      { title: "Linked entries", counts: (entry) => !["compile time", "no"].includes(entry.linkedInto) },
      { title: "All entries", counts: () => true },
    ]),
    "",
    "## Entries",
    "",
    "| Name | Version | Licence | Linked into | Copyright | Source | Texts |",
    "|---|---|---|---|---|---|---|",
    ...entries.map(
      (entry) =>
        `| ${cell(entry.name)} | ${cell(entry.version)} | ${cell(entry.licence)} | ${entry.linkedInto} | ${entry.copyright.map(cell).join("; ")} | ${cell(entry.source)} | ${entry.texts.join(", ")} |`,
    ),
    "",
    "## Texts",
    "",
    textsSection(store),
    "",
  ];
  return lines.join("\n");
}

function pnpmLock() {
  return readFileSync(join(repo, "pnpm-lock.yaml"), "utf8").replace(/\r\n/g, "\n");
}

function importerVersions(lock) {
  const importer = /^importers:\n\n {2}\.:\n([\s\S]*?)\n\S/m.exec(lock)?.[1] ?? "";
  return new Map([...importer.matchAll(/^ {6}'?([^':\n]+)'?:\n {8}specifier: .*\n {8}version: (.+)$/gm)].map((match) => [match[1], match[2]]));
}

function snapshotDependencies(lock) {
  const blocks = lock.slice(lock.indexOf("\nsnapshots:\n")).split(/\n(?= {2}\S)/).slice(1);
  return new Map(
    blocks.map((block) => [
      /^ {2}'?(.+?)'?:/.exec(block)[1],
      [...block.matchAll(/^ {6}'?([^':\s]+)'?: (.+)$/gm)].map((match) => `${match[1]}@${match[2]}`),
    ]),
  );
}

function cssImportedPackages() {
  const walk = (dir) =>
    readdirSync(dir, { withFileTypes: true })
      .sort((a, b) => compareText(a.name, b.name))
      .flatMap((entry) => {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) return walk(path);
        return entry.name.endsWith(".css") ? [path] : [];
      });
  return [
    ...new Set(
      walk(join(repo, "src")).flatMap((file) =>
        [...readFileSync(file, "utf8").matchAll(/@(?:import|plugin)\s+["']([^"'./][^"']*)["']/g)].map((match) =>
          match[1].split("/").slice(0, match[1].startsWith("@") ? 2 : 1).join("/"),
        ),
      ),
    ),
  ];
}

function splitKey(key) {
  const plain = key.split("(")[0];
  const at = plain.lastIndexOf("@");
  return { name: plain.slice(0, at), version: plain.slice(at + 1) };
}

function packageDir(store, name, version) {
  const encoded = name.replace("/", "+");
  const match = store.find((dir) => {
    if (dir !== `${encoded}@${version}` && !dir.startsWith(`${encoded}@${version}_`)) return false;
    const manifest = join(repo, "node_modules", ".pnpm", dir, "node_modules", name, "package.json");
    return existsSync(manifest) && JSON.parse(readFileSync(manifest, "utf8")).version === version;
  });
  if (!match) throw new Error(`node_modules has no ${name}@${version}; run pnpm install --frozen-lockfile`);
  return join(repo, "node_modules", ".pnpm", match, "node_modules", name);
}

function npmLicence(manifest) {
  if (typeof manifest.license === "string") return manifest.license;
  if (manifest.license?.type) return manifest.license.type;
  if (Array.isArray(manifest.licenses)) return manifest.licenses.map((item) => item.type ?? item).join(" OR ");
  return null;
}

function npmSource(manifest, name) {
  const repository = typeof manifest.repository === "string" ? manifest.repository : manifest.repository?.url;
  if (repository) {
    const url = repository
      .replace(/^git\+/, "")
      .replace(/^git:\/\//, "https://")
      .replace(/^ssh:\/\/git@/, "https://")
      .replace(/\.git$/, "")
      .replace(/^github:/, "https://github.com/");
    return /^[\w.-]+\/[\w.-]+$/.test(url) ? `https://github.com/${url}` : url;
  }
  return manifest.homepage ?? `https://www.npmjs.com/package/${name}`;
}

function npmAuthors(manifest) {
  const people = [manifest.author, ...(manifest.authors ?? []), ...(manifest.contributors ?? [])].filter(Boolean);
  return people.map((person) => (typeof person === "string" ? person : person.name ?? "")).filter(Boolean);
}

function npmList() {
  const lock = pnpmLock();
  const versions = importerVersions(lock);
  const manifest = JSON.parse(readFileSync(join(repo, "package.json"), "utf8"));
  const roots = [
    ...Object.keys(manifest.dependencies).map((name) => ({ name, via: "package.json dependencies" })),
    ...cssImportedPackages().map((name) => ({ name, via: "Tailwind CSS build of `src/index.css`" })),
  ];
  const dependencies = snapshotDependencies(lock);
  const reached = new Map();
  const visit = (key, via) => {
    if (!dependencies.has(key)) throw new Error(`pnpm-lock.yaml has no snapshot ${key}`);
    const { name, version } = splitKey(key);
    const id = `${name} ${version}`;
    const vias = reached.get(id) ?? new Set();
    if (vias.has(via)) return;
    vias.add(via);
    reached.set(id, vias);
    dependencies.get(key).forEach((dependency) => visit(dependency, `via ${name}`));
  };
  for (const root of roots) {
    const version = versions.get(root.name);
    if (!version) throw new Error(`pnpm-lock.yaml has no importer entry for ${root.name}`);
    visit(`${root.name}@${version}`, root.via);
  }
  const pnpmDirs = readdirSync(join(repo, "node_modules", ".pnpm")).sort(compareText);
  const store = new TextStore("N");
  const entries = [...reached]
    .map(([id, vias]) => {
      const at = id.lastIndexOf(" ");
      return { name: id.slice(0, at), version: id.slice(at + 1), vias };
    })
    .sort(compareEntries)
    .map(({ name, version, vias }) => {
      const dir = packageDir(pnpmDirs, name, version);
      const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
      const licence = npmLicence(pkg);
      const declared = /^SEE LICEN[CS]E IN (.+)$/i.exec(licence ?? "")?.[1];
      const entry = {
        name,
        version,
        licence: licence ?? "Not declared",
        reachedFrom: [...vias].sort(compareText).join("; "),
        source: npmSource(pkg, name),
      };
      attachTexts(entry, store, collectTexts(dir, declared, false), licence);
      if (entry.copyright.length === 0) entry.copyright = copyrightLines([readmeLicenceSection(dir)]);
      finishCopyright(entry, npmAuthors(pkg));
      return entry;
    });
  const lines = [
    "# Third-party licences: npm packages",
    "",
    `Generated by \`${command}\` from \`package.json\`, \`pnpm-lock.yaml\` and the installed packages in \`node_modules\`. Do not edit by hand: run the command again after any change to \`pnpm-lock.yaml\`.`,
    "",
    `Scope: the ${entries.length} npm packages of the production dependency closure: the production dependencies of \`package.json\` with everything they depend on in \`pnpm-lock.yaml\`, and the packages that \`src\` style sheets pull into the Tailwind CSS build with \`@import\` or \`@plugin\`. This is a superset of the packages whose code, styles or fonts reach the frontend bundle: a production dependency that no module imports is still listed, and so are the dependencies of jszip, whose prebuilt \`dist/jszip.min.js\` already contains the parts it uses. Build and test tooling is not listed; the helpers that vite injects into the bundle have their own row in \`THIRD-PARTY-NOTICES.md\`. "Reached from" says why a package is in the set.`,
    "",
    "Licence texts: each entry names the licence files the package ships at its root (LICENSE, LICENCE, COPYING, COPYRIGHT, NOTICE, PATENTS or UNLICENSE, and the file named by `SEE LICENSE IN`). Where a package ships no licence text (none, or only an SPDX metadata file), the entry adds the SPDX standard text of the licence taken from its expression (MIT where the expression offers it). The copyright column repeats the copyright lines of those files; where they have none it gives the copyright lines of the README's licence section, and failing that the authors from the manifest. Texts that differ only in white space are printed once under Texts, as their first user ships them.",
    "",
    "## Licence summary",
    "",
    summaryTable(entries, [{ title: "Packages", counts: () => true }]),
    "",
    "## Entries",
    "",
    "| Name | Version | Licence | Reached from | Copyright | Source | Texts |",
    "|---|---|---|---|---|---|---|",
    ...entries.map(
      (entry) =>
        `| ${cell(entry.name)} | ${cell(entry.version)} | ${cell(entry.licence)} | ${cell(entry.reachedFrom)} | ${entry.copyright.map(cell).join("; ")} | ${cell(entry.source)} | ${entry.texts.join(", ")} |`,
    ),
    "",
    "## Texts",
    "",
    textsSection(store),
    "",
  ];
  return lines.join("\n");
}

writeFileSync(join(output, "THIRD-PARTY-LICENSES-RUST.md"), rustList());
writeFileSync(join(output, "THIRD-PARTY-LICENSES-NPM.md"), npmList());
