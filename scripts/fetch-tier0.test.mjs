import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, symlinkSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), "fetch-tier0.mjs");

function verifyMissing(folder, work) {
  return spawnSync(process.execPath, [join(folder, "fetch-tier0.mjs"), "--verify", join(work, "missing.abcat")], {
    encoding: "utf8",
  });
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
      const run = verifyMissing(folder, work);
      assert.equal(run.status, 1, `${folder}: exit ${run.status}, stdout ${JSON.stringify(run.stdout)}, stderr ${JSON.stringify(run.stderr)}`);
      assert.match(run.stderr, /missing\.abcat does not exist/);
    }
  } finally {
    unlinkSync(linked);
    assert.ok(existsSync(join(real, "fetch-tier0.mjs")));
    rmSync(work, { recursive: true, force: true });
  }
});
