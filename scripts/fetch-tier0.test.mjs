import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { DEFAULT_OUT, TIER0_BYTES, TIER0_FILE, TIER0_SHA256, downloadUrl, fetchTier0, verify } from "./fetch-tier0.mjs";

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), "fetch-tier0.mjs");
const PROVENANCE = join(dirname(DEFAULT_OUT), TIER0_FILE.replace(/\.abcat$/, ".PROVENANCE.md"));
const BODY = Buffer.from("a small catalogue body that the fetch tests pin by size and sha-256\n");
const PINS = { file: "small.abcat", bytes: BODY.length, sha256: createHash("sha256").update(BODY).digest("hex") };
const UNREACHABLE_BASE = "http://127.0.0.1:9/";

function sameLengthOtherBody() {
  const other = Buffer.from(BODY);
  other[0] ^= 0xff;
  return other;
}

function runScript(args, script = SCRIPT) {
  return spawnSync(process.execPath, [script, ...args], {
    encoding: "utf8",
    env: { ...process.env, ASTROBURST_CATALOG_BASE_URL: UNREACHABLE_BASE, ASTROBURST_TIER0_SOURCE: "" },
  });
}

function described(run) {
  return `exit ${run.status}, stdout ${JSON.stringify(run.stdout)}, stderr ${JSON.stringify(run.stderr)}`;
}

async function withWork(body) {
  const work = mkdtempSync(join(tmpdir(), "fetch-tier0-"));
  try {
    await body(work);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

async function serve(routes) {
  const hits = [];
  const server = createServer((request, response) => {
    hits.push(request.url);
    const route = routes[request.url] ?? { status: 404 };
    response.writeHead(route.status, route.location ? { location: route.location } : {});
    response.end(route.body ?? "");
  });
  await new Promise((listening) => server.listen(0, "127.0.0.1", listening));
  return {
    base: `http://127.0.0.1:${server.address().port}/`,
    hits,
    close: () => {
      server.closeAllConnections();
      return new Promise((closed) => server.close(closed));
    },
  };
}

async function withServer(routes, body) {
  const server = await serve(routes);
  try {
    await body(server);
  } finally {
    await server.close();
  }
}

test("the fetch script fails on a missing catalogue whether it is run from its folder or through a link", () => {
  const work = mkdtempSync(join(tmpdir(), "fetch-tier0-"));
  const real = join(work, "real");
  const linked = join(work, "linked");
  mkdirSync(real);
  copyFileSync(SCRIPT, join(real, "fetch-tier0.mjs"));
  symlinkSync(real, linked, "junction");
  try {
    for (const folder of [real, linked]) {
      const run = runScript(["--verify", join(work, "missing.abcat")], join(folder, "fetch-tier0.mjs"));
      assert.equal(run.status, 1, `${folder}: ${described(run)}`);
      assert.match(run.stderr, /missing\.abcat does not exist/);
    }
  } finally {
    unlinkSync(linked);
    assert.ok(existsSync(join(real, "fetch-tier0.mjs")));
    rmSync(work, { recursive: true, force: true });
  }
});

test("a download that matches its pins is kept and leaves no part file", () =>
  withWork((work) =>
    withServer({ "/small.abcat": { status: 200, body: BODY } }, async ({ base }) => {
      const out = join(work, PINS.file);
      assert.match(await fetchTier0({ out, base, pins: PINS }), /^downloaded /);
      assert.deepEqual(readFileSync(out), BODY);
      assert.deepEqual(readdirSync(work), [PINS.file]);
    }),
  ));

test("a download shorter than its pins is refused and its part file removed", () =>
  withWork((work) =>
    withServer({ "/small.abcat": { status: 200, body: BODY.subarray(0, BODY.length - 1) } }, async ({ base }) => {
      await assert.rejects(fetchTier0({ out: join(work, PINS.file), base, pins: PINS }), new RegExp(`refused small\\.abcat: ${BODY.length - 1} bytes`));
      assert.deepEqual(readdirSync(work), []);
    }),
  ));

test("a download longer than its pins is refused while it streams and its part file removed", () =>
  withWork((work) =>
    withServer({ "/small.abcat": { status: 200, body: Buffer.concat([BODY, Buffer.from("x")]) } }, async ({ base }) => {
      await assert.rejects(fetchTier0({ out: join(work, PINS.file), base, pins: PINS }), new RegExp(`larger than the pinned ${BODY.length} bytes`));
      assert.deepEqual(readdirSync(work), []);
    }),
  ));

test("a download of the pinned size with another sha-256 is refused and its part file removed", () =>
  withWork((work) =>
    withServer({ "/small.abcat": { status: 200, body: sameLengthOtherBody() } }, async ({ base }) => {
      await assert.rejects(fetchTier0({ out: join(work, PINS.file), base, pins: PINS }), new RegExp(`${BODY.length} bytes with sha-256 [0-9a-f]{64}; pinned`));
      assert.deepEqual(readdirSync(work), []);
    }),
  ));

test("a redirect on this computer is followed and the body behind it is still checked", () =>
  withWork((work) =>
    withServer(
      {
        "/moved/small.abcat": { status: 302, location: "/good/small.abcat" },
        "/good/small.abcat": { status: 200, body: BODY },
        "/bad/small.abcat": { status: 302, location: "/wrong/small.abcat" },
        "/wrong/small.abcat": { status: 200, body: sameLengthOtherBody() },
      },
      async ({ base, hits }) => {
        const out = join(work, PINS.file);
        await assert.rejects(fetchTier0({ out, base: `${base}bad/`, pins: PINS }), /refused small\.abcat/);
        assert.deepEqual(readdirSync(work), []);
        assert.match(await fetchTier0({ out, base: `${base}moved/`, pins: PINS }), /^downloaded /);
        assert.deepEqual(readFileSync(out), BODY);
        assert.deepEqual(hits, ["/bad/small.abcat", "/wrong/small.abcat", "/moved/small.abcat", "/good/small.abcat"]);
      },
    ),
  ));

test("a stale file is replaced, and a file that matches its pins is kept without a request", () =>
  withWork((work) =>
    withServer({ "/small.abcat": { status: 200, body: BODY } }, async ({ base, hits }) => {
      const out = join(work, PINS.file);
      writeFileSync(out, sameLengthOtherBody());
      assert.match(await fetchTier0({ out, base, pins: PINS }), /^downloaded /);
      assert.deepEqual(readFileSync(out), BODY);
      assert.match(await fetchTier0({ out, base, pins: PINS }), /^already present: /);
      assert.equal(hits.length, 1);
    }),
  ));

test("an unpublished catalogue names the variable that points the build at a local copy", () =>
  withWork((work) =>
    withServer({}, async ({ base }) => {
      await assert.rejects(fetchTier0({ out: join(work, PINS.file), base, pins: PINS }), /HTTP 404.*ASTROBURST_TIER0_SOURCE/);
      assert.deepEqual(readdirSync(work), []);
    }),
  ));

test("only https addresses, or http to this computer, are used", async () => {
  for (const refused of ["http://example.com/catalogs/", "ftp://127.0.0.1/catalogs/", "http://127.0.0.2/"]) {
    assert.throws(() => downloadUrl(refused, PINS.file), /is refused/, refused);
  }
  for (const [base, expected] of [
    ["https://example.com/catalogs", "https://example.com/catalogs/small.abcat"],
    ["http://127.0.0.1:8080/", "http://127.0.0.1:8080/small.abcat"],
    ["http://localhost/c/", "http://localhost/c/small.abcat"],
    ["http://[::1]/", "http://[::1]/small.abcat"],
  ]) {
    assert.equal(downloadUrl(base, PINS.file).href, expected);
  }
  await withWork(async (work) => {
    await assert.rejects(fetchTier0({ out: join(work, PINS.file), base: "http://example.com/", pins: PINS }), /is refused/);
    assert.deepEqual(readdirSync(work), []);
  });
});

test("a local copy is copied only when it matches its pins", () =>
  withWork(async (work) => {
    const out = join(work, "out", PINS.file);
    for (const [name, body, refusal] of [
      ["short.abcat", BODY.subarray(1), /refused small\.abcat/],
      ["long.abcat", Buffer.concat([BODY, BODY]), /larger than the pinned/],
      ["other.abcat", sameLengthOtherBody(), /refused small\.abcat/],
    ]) {
      writeFileSync(join(work, name), body);
      await assert.rejects(fetchTier0({ out, source: join(work, name), pins: PINS }), refusal, name);
      assert.deepEqual(readdirSync(dirname(out)), [], name);
    }
    writeFileSync(join(work, "good.abcat"), BODY);
    assert.match(await fetchTier0({ out, source: join(work, "good.abcat"), pins: PINS }), /^copied /);
    assert.deepEqual(readFileSync(out), BODY);
    assert.deepEqual(await verify(out, PINS), { ok: true, reason: `${out} matches the pinned size and sha-256` });
  }));

test("the command line refuses a local copy of the wrong size or sha-256, and --verify refuses the same files", () =>
  withWork((work) => {
    const short = join(work, "short.abcat");
    const zeros = join(work, "zeros.abcat");
    writeFileSync(short, BODY);
    writeFileSync(zeros, Buffer.alloc(TIER0_BYTES));
    for (const [source, copyRefusal, verifyRefusal] of [
      [short, new RegExp(`refused ${TIER0_FILE}: ${BODY.length} bytes`), new RegExp(`holds ${BODY.length} bytes, ${TIER0_BYTES} are pinned`)],
      [zeros, new RegExp(`${TIER0_BYTES} bytes with sha-256 [0-9a-f]{64}; pinned ${TIER0_BYTES} bytes with sha-256 ${TIER0_SHA256}`), /failed its sha-256 check/],
    ]) {
      const out = join(work, "out", TIER0_FILE);
      const copy = runScript(["--from", source, "--out", out]);
      assert.equal(copy.status, 1, described(copy));
      assert.match(copy.stderr, copyRefusal);
      assert.deepEqual(readdirSync(dirname(out)), []);
      const checked = runScript(["--verify", source]);
      assert.equal(checked.status, 1, described(checked));
      assert.match(checked.stderr, verifyRefusal);
    }
  }));

test("the command line refuses an unknown, malformed or incomplete option instead of fetching", () =>
  withWork((work) => {
    const missing = join(work, "missing.abcat");
    for (const [args, refusal] of [
      [["--verfy", missing], /unknown option '--verfy'/i],
      [["--verify"], /'--verify <value>' argument missing/],
      [["--verify", ""], /--verify needs a path/],
      [[missing], /unexpected argument/i],
      [["--verify", missing, "--from", missing], /--verify takes no other option/],
    ]) {
      const run = runScript(args);
      assert.equal(run.status, 1, `${args.join(" ")}: ${described(run)}`);
      assert.match(run.stderr, refusal, args.join(" "));
      assert.doesNotMatch(run.stdout, /already present|downloaded|copied/, args.join(" "));
    }
    const inline = runScript([`--verify=${missing}`]);
    assert.equal(inline.status, 1, described(inline));
    assert.match(inline.stderr, /missing\.abcat does not exist/);
  }));

test("--verify takes the folder that holds the catalogue as well as the file", async (t) => {
  await withWork((work) => {
    const run = runScript(["--verify", work]);
    assert.equal(run.status, 1, described(run));
    assert.ok(run.stderr.includes(`${join(work, TIER0_FILE)} does not exist`), described(run));
  });
  await t.test("on the fetched catalogue", { skip: !existsSync(DEFAULT_OUT) && "the catalogue has not been fetched" }, () => {
    const run = runScript(["--verify", dirname(DEFAULT_OUT)]);
    assert.equal(run.status, 0, described(run));
    assert.ok(run.stdout.includes(`${DEFAULT_OUT} matches the pinned size and sha-256`), described(run));
  });
});

test("the provenance file next to the catalogue states the pinned name, size and sha-256", () => {
  const provenance = readFileSync(PROVENANCE, "utf8").replace(/\r\n/g, "\n");
  assert.equal(/^- File: `([^`]+)`$/m.exec(provenance)?.[1], TIER0_FILE);
  assert.equal(/^- SHA-256: `([0-9a-f]{64})`$/m.exec(provenance)?.[1], TIER0_SHA256);
  assert.equal(Number(/^- Size: (\d+) bytes$/m.exec(provenance)?.[1]), TIER0_BYTES);
});
