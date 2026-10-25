import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { carriedFiles, check, collect, MANIFEST, ownersOf, parseOwners, parsePackages } from "./appimage-licences.mjs";

const ELF = Buffer.from([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0]);
const SKIP = ["usr/bin/astroburst", "usr/lib/AstroBurst"];

function withWork(body) {
  const work = mkdtempSync(join(tmpdir(), "appimage-licences-"));
  try {
    return body(work);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

function put(root, path, content) {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), content);
}

function appDir(work) {
  const root = join(work, "squashfs-root");
  put(root, "AppRun", ELF);
  put(root, "usr/bin/astroburst", ELF);
  put(root, "usr/bin/xdg-open", "#!/bin/sh\n");
  put(root, "usr/lib/AstroBurst/catalogs/tycho2-l6-v1.abcat", "catalogue");
  put(root, "usr/lib/AstroBurst/helper.so", ELF);
  put(root, "usr/lib/libicuuc.so.70", ELF);
  put(root, "usr/lib/libxml2.so.2", ELF);
  put(root, "usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitNetworkProcess", ELF);
  put(root, "usr/share/glib-2.0/schemas/gschemas.compiled", "generated");
  symlinkSync("libxml2.so.2", join(root, "usr/lib/libxml2.so"));
  return root;
}

function docs(work) {
  const root = join(work, "doc");
  for (const name of ["libicu70", "libxml2", "libwebkit2gtk-4.1-0", "xdg-utils"]) put(root, `${name}/copyright`, `Copyright of ${name}\n`);
  return root;
}

const OWNERS = new Map([
  ["libicuuc.so.70", [{ package: "libicu70:amd64", path: "/usr/lib/x86_64-linux-gnu/libicuuc.so.70" }]],
  ["libxml2.so.2", [{ package: "libxml2:amd64", path: "/usr/lib/x86_64-linux-gnu/libxml2.so.2" }, { package: "libxml2:i386", path: "/usr/lib/i386-linux-gnu/libxml2.so.2" }]],
  ["WebKitNetworkProcess", [{ package: "libwebkit2gtk-4.1-0:amd64", path: "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitNetworkProcess" }]],
  ["xdg-open", [{ package: "xdg-utils", path: "/usr/bin/xdg-open" }]],
]);

const PACKAGES = new Map([
  ["libicu70:amd64", { version: "70.1-2", source: "icu", sourceVersion: "70.1-2" }],
  ["libxml2:amd64", { version: "2.9.13+dfsg-1ubuntu0.4", source: "libxml2", sourceVersion: "2.9.13+dfsg-1ubuntu0.4" }],
  ["libwebkit2gtk-4.1-0:amd64", { version: "2.46.5-0ubuntu0.22.04.1", source: "webkit2gtk", sourceVersion: "2.46.5-0ubuntu0.22.04.1" }],
  ["xdg-utils", { version: "1.1.3-4.1ubuntu3~22.04.1", source: "xdg-utils", sourceVersion: "1.1.3-4.1ubuntu3~22.04.1" }],
]);

const fakeQuery = (owners = OWNERS) => ({
  owners: (name) => owners.get(name) ?? [],
  packages: (names) => new Map(names.map((name) => [name, PACKAGES.get(name)])),
});

test("the carried files are the ELF files and the commands outside the app's own files", () =>
  withWork((work) => {
    assert.deepEqual(carriedFiles(appDir(work), SKIP), [
      "AppRun",
      "usr/bin/xdg-open",
      "usr/lib/libicuuc.so.70",
      "usr/lib/libxml2.so.2",
      "usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitNetworkProcess",
    ]);
  }));

test("collect copies the copyright file of every package the AppImage takes a file from and lists each file", () =>
  withWork((work) => {
    const out = join(work, "appimage-libraries");
    collect({ appDir: appDir(work), out, skip: SKIP, query: fakeQuery(), docRoot: docs(work), system: "Ubuntu 22.04.5 LTS" });
    assert.deepEqual(readdirSync(join(out, "packages")).sort(), ["libicu70", "libwebkit2gtk-4.1-0", "libxml2", "xdg-utils"]);
    assert.equal(readFileSync(join(out, "packages", "libxml2", "copyright"), "utf8"), "Copyright of libxml2\n");
    const manifest = readFileSync(join(out, MANIFEST), "utf8");
    assert.match(manifest, /built this AppImage on Ubuntu 22\.04\.5 LTS/);
    assert.match(manifest, /^\| `usr\/lib\/libicuuc\.so\.70` \| libicu70:amd64 \| 70\.1-2 \| icu 70\.1-2 \| `packages\/libicu70\/copyright` \|$/m);
    assert.match(manifest, /^\| `usr\/lib\/libxml2\.so\.2` \| libxml2:amd64 \|/m);
    assert.doesNotMatch(manifest, /i386/);
    assert.match(manifest, /^- `AppRun`$/m);
    assert.doesNotMatch(manifest, /astroburst|AstroBurst\/|gschemas/);
  }));

test("collect stops when a package has no copyright file, or when the folder carries no system file at all", () =>
  withWork((work) => {
    const doc = docs(work);
    rmSync(join(doc, "libicu70"), { recursive: true });
    assert.throws(
      () => collect({ appDir: appDir(work), out: join(work, "a"), skip: SKIP, query: fakeQuery(), docRoot: doc, system: "Ubuntu" }),
      /libicu70:amd64 has no copyright file/,
    );
    const empty = join(work, "empty");
    put(empty, "usr/bin/astroburst", ELF);
    assert.throws(
      () => collect({ appDir: empty, out: join(work, "b"), skip: SKIP, query: fakeQuery(), docRoot: doc, system: "Ubuntu" }),
      /carries no file outside the app's own/,
    );
  }));

test("check accepts the AppImage that carries the collected folder and refuses one whose files differ from the list", () =>
  withWork((work) => {
    const root = appDir(work);
    const relative = "usr/lib/AstroBurst/licenses/appimage-libraries";
    collect({ appDir: root, out: join(root, relative), skip: SKIP, query: fakeQuery(), docRoot: docs(work), system: "Ubuntu" });
    assert.match(check({ appDir: root, folder: relative, skip: SKIP }), /lists all 5 files/);
    put(root, "usr/lib/libpng16.so.16", ELF);
    assert.throws(() => check({ appDir: root, folder: relative, skip: SKIP }), /not listed: usr\/lib\/libpng16\.so\.16/);
    rmSync(join(root, "usr/lib/libpng16.so.16"));
    rmSync(join(root, "usr/lib/libicuuc.so.70"));
    assert.throws(() => check({ appDir: root, folder: relative, skip: SKIP }), /listed but not carried: usr\/lib\/libicuuc\.so\.70/);
    assert.throws(() => check({ appDir: root, folder: "usr/lib/AstroBurst/licenses/missing", skip: SKIP }), /does not carry/);
  }));

test("a file is attributed to the package that installs it where the AppImage tooling copies it from", () => {
  const xdgOpen = [
    { package: "apparmor", path: "/etc/apparmor.d/abstractions/xdg-open" },
    { package: "xdg-utils", path: "/usr/bin/xdg-open" },
  ];
  assert.deepEqual(ownersOf("usr/bin/xdg-open", xdgOpen), [{ package: "xdg-utils", path: "/usr/bin/xdg-open" }]);
  const zlib = [
    { package: "zlib1g:amd64", path: "/lib/x86_64-linux-gnu/libz.so.1" },
    { package: "zlib1g:i386", path: "/lib/i386-linux-gnu/libz.so.1" },
    { package: "zlib-doc", path: "/usr/share/doc/zlib-doc/examples/libz.so.1" },
  ];
  assert.deepEqual(ownersOf("usr/lib/libz.so.1", zlib), [{ package: "zlib1g:amd64", path: "/lib/x86_64-linux-gnu/libz.so.1" }]);
  const helper = [
    { package: "libwebkit2gtk-4.1-0:amd64", path: "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitNetworkProcess" },
    { package: "other:amd64", path: "/usr/lib/x86_64-linux-gnu/other/WebKitNetworkProcess" },
  ];
  assert.deepEqual(ownersOf("usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitNetworkProcess", helper), [helper[0]]);
  assert.deepEqual(ownersOf("AppRun", []), []);
});

test("the dpkg-query outputs are read as Ubuntu prints them", () => {
  assert.deepEqual(
    parseOwners(
      [
        "libc6:amd64: /usr/lib64/ld-linux-x86-64.so.2",
        "libc6:amd64: /usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
        "diversion by libc6 from: /lib64/ld-linux-x86-64.so.2",
        "diversion by libc6 to: /lib64/ld-linux-x86-64.so.2.usr-is-merged",
        "libfoo1:amd64, libfoo1:i386: /usr/lib/x86_64-linux-gnu/other/ld-linux-x86-64.so.2.bak",
        "libbar:amd64, libbar-dev:amd64: /usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
      ].join("\n"),
      "ld-linux-x86-64.so.2",
    ),
    [
      { package: "libc6:amd64", path: "/usr/lib64/ld-linux-x86-64.so.2" },
      { package: "libc6:amd64", path: "/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2" },
      { package: "libbar:amd64", path: "/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2" },
      { package: "libbar-dev:amd64", path: "/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2" },
    ],
  );
  assert.deepEqual(
    parsePackages("zlib1g:amd64\t1:1.3.dfsg-3.1ubuntu2.1\tzlib\t1:1.3.dfsg-3.1ubuntu2.1\nxdg-utils\t1.1.3\txdg-utils\t1.1.3\n"),
    new Map([
      ["zlib1g:amd64", { version: "1:1.3.dfsg-3.1ubuntu2.1", source: "zlib", sourceVersion: "1:1.3.dfsg-3.1ubuntu2.1" }],
      ["xdg-utils", { version: "1.1.3", source: "xdg-utils", sourceVersion: "1.1.3" }],
    ]),
  );
});
