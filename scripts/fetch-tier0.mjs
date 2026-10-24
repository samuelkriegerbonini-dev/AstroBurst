import { createHash } from "node:crypto";
import {
  createReadStream,
  createWriteStream,
  existsSync,
  mkdirSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
} from "node:fs";
import { dirname, resolve } from "node:path";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

export const TIER0_FILE = "tycho2-l6-v1.abcat";
export const TIER0_BYTES = 15541742;
export const TIER0_SHA256 = "bf01b058e7c9babe03d38d367c98d6a0dbb0e9785741a9c842d053574fa17076";
export const DEFAULT_BASE_URL = "https://github.com/samuelkriegerbonini-dev/AstroBurst/releases/download/catalogs-v1/";
export const ENV_BASE_URL = "ASTROBURST_CATALOG_BASE_URL";
export const ENV_SOURCE = "ASTROBURST_TIER0_SOURCE";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const DEFAULT_OUT = resolve(REPO_ROOT, "src-tauri", "resources", "catalogs", TIER0_FILE);
const PART_SUFFIX = ".part";
const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]"]);

class Refused extends Error {}

function argValue(args, name) {
  const at = args.indexOf(name);
  if (at < 0) return null;
  const value = args[at + 1];
  if (value === undefined || value.startsWith("--")) throw new Refused(`${name} needs a path`);
  return value;
}

function hashingCounter(hash, counter) {
  return new Transform({
    transform(chunk, _encoding, done) {
      counter.bytes += chunk.length;
      if (counter.bytes > TIER0_BYTES) {
        done(new Refused(`the download is larger than the pinned ${TIER0_BYTES} bytes`));
        return;
      }
      hash.update(chunk);
      done(null, chunk);
    },
  });
}

async function digestOf(path) {
  const hash = createHash("sha256");
  await pipeline(createReadStream(path), new Transform({
    transform(chunk, _encoding, done) {
      hash.update(chunk);
      done();
    },
  }));
  return hash.digest("hex");
}

export async function verify(path) {
  if (!existsSync(path)) return { ok: false, reason: `${path} does not exist` };
  const bytes = statSync(path).size;
  if (bytes !== TIER0_BYTES) return { ok: false, reason: `${path} holds ${bytes} bytes, ${TIER0_BYTES} are pinned` };
  const actual = await digestOf(path);
  if (actual !== TIER0_SHA256) {
    return { ok: false, reason: `${path} failed its sha-256 check: expected ${TIER0_SHA256}, received ${actual}` };
  }
  return { ok: true, reason: `${path} matches the pinned size and sha-256` };
}

export function downloadUrl(base) {
  const text = base && base.trim() !== "" ? base.trim() : DEFAULT_BASE_URL;
  const url = new URL(text.endsWith("/") ? `${text}${TIER0_FILE}` : `${text}/${TIER0_FILE}`);
  const loopback = LOOPBACK_HOSTS.has(url.hostname) || LOOPBACK_HOSTS.has(`[${url.hostname}]`);
  if (url.protocol !== "https:" && !(url.protocol === "http:" && loopback)) {
    throw new Refused(`the download address ${url} is refused: only https://, or http:// to this computer, is used`);
  }
  return url;
}

async function finish(part, out, counter, hash) {
  const actual = hash.digest("hex");
  if (counter.bytes !== TIER0_BYTES || actual !== TIER0_SHA256) {
    rmSync(part, { force: true });
    throw new Refused(
      `refused ${TIER0_FILE}: ${counter.bytes} bytes with sha-256 ${actual}; pinned ${TIER0_BYTES} bytes with sha-256 ${TIER0_SHA256}`,
    );
  }
  renameSync(part, out);
}

async function copyFrom(source, out) {
  const part = `${out}${PART_SUFFIX}`;
  const hash = createHash("sha256");
  const counter = { bytes: 0 };
  try {
    await pipeline(createReadStream(source), hashingCounter(hash, counter), createWriteStream(part));
  } catch (error) {
    rmSync(part, { force: true });
    throw error;
  }
  await finish(part, out, counter, hash);
}

async function download(url, out) {
  const response = await fetch(url, { redirect: "follow" });
  if (response.status === 404) {
    throw new Refused(
      `${TIER0_FILE} is not published yet: ${url} answered HTTP 404; publish the catalogs-v1 release, or set ${ENV_SOURCE} to a local copy`,
    );
  }
  if (!response.ok || response.body === null) {
    throw new Refused(`${url} answered HTTP ${response.status}`);
  }
  const final = new URL(response.url || url);
  if (final.protocol !== "https:" && final.protocol !== url.protocol) {
    throw new Refused(`the download was redirected to ${final}, which is not https`);
  }
  const part = `${out}${PART_SUFFIX}`;
  const hash = createHash("sha256");
  const counter = { bytes: 0 };
  try {
    await pipeline(Readable.fromWeb(response.body), hashingCounter(hash, counter), createWriteStream(part));
  } catch (error) {
    rmSync(part, { force: true });
    throw error;
  }
  await finish(part, out, counter, hash);
}

export async function fetchTier0({ out = DEFAULT_OUT, source = null, base = null } = {}) {
  mkdirSync(dirname(out), { recursive: true });
  if (existsSync(out)) {
    const present = await verify(out);
    if (present.ok) return `already present: ${present.reason}`;
    rmSync(out, { force: true });
    console.warn(`replacing ${out}: ${present.reason}`);
  }
  if (source) {
    await copyFrom(source, out);
    return `copied ${source} to ${out}; it matches the pinned size and sha-256`;
  }
  const url = downloadUrl(base);
  await download(url, out);
  return `downloaded ${url} to ${out}; it matches the pinned size and sha-256`;
}

async function main(args) {
  const checked = argValue(args, "--verify");
  if (checked) {
    const result = await verify(resolve(checked));
    if (!result.ok) throw new Refused(result.reason);
    return result.reason;
  }
  const out = resolve(argValue(args, "--out") ?? DEFAULT_OUT);
  const from = argValue(args, "--from") ?? process.env[ENV_SOURCE] ?? null;
  return fetchTier0({ out, source: from ? resolve(from) : null, base: process.env[ENV_BASE_URL] ?? null });
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  main(process.argv.slice(2))
    .then((message) => console.log(`tier 0: ${message}`))
    .catch((error) => {
      console.error(`tier 0: ${error instanceof Refused ? error.message : error.stack ?? error}`);
      process.exit(1);
    });
}
