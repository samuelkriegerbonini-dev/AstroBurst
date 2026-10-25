import { existsSync, readFileSync } from "node:fs";
import { posix } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it, expect } from "vitest";

const repoPath = (path: string) => fileURLToPath(new URL(`../../${path}`, import.meta.url));

const CREDITS_HEADING = "Credits and acknowledgements";

const CREDITED: Record<string, string[]> = {
  people: [
    "[Jae-Joon Lee](https://github.com/leejjoon)",
    "[jing2uo](https://github.com/jing2uo)",
    "J. Hester",
    "P. Scowen",
    "Arizona State University",
  ],
  "organisations and archives": [
    "NASA",
    "ESA",
    "STScI",
    "MAST",
    "[FlatPark](https://github.com/flatpark/flatpark)",
    "CDS",
    "VizieR",
  ],
  "screenshot data sets": ["jw02739", "jw06565", "jw02016", "jw01266", "jw01476", "Roman WFI L2", "HST/WFPC2", "M16"],
  services: ["Gaia DR3", "VizieR", "nova.astrometry.net", "MAST"],
  "software and methods": [
    "[wcs-rs](https://github.com/cds-astro/wcs-rs) by CDS",
    "docs/adr/0001-wcs-rs-for-wcs-engine.md",
    "gwcs 1.0.3",
    "fitsio-pure",
    "matplotlib",
    "IRAF",
    "zscale",
    "astropy",
    "wcslib",
    "Greisen et al. 2006",
    "Kasten-Young",
    "Fritsch-Carlson",
    "BT.709",
    "Foraxx",
    "Hubble Legacy",
    "MaxIm DL",
    "N.I.N.A.",
    "SharpCap",
    "GHS",
    "Winsorized",
    "BWMV",
    "RANSAC",
    "Lanczos3",
    "Tikhonov",
    "Richardson-Lucy",
    "CLAHE",
    "MLT",
    "SCNR",
    "Petrosian",
    "tweakreg",
    "ZWO",
    "QHY",
    "ASCOM",
    "Bayer",
    "X-Trans",
    "Pillow",
    "DS9",
    "jdaviz",
    "Siril",
    "PixInsight",
    "Tauri",
    "React",
    "WebGPU",
  ],
  documentation: [
    "CHANGELOG.md",
    "docs/astroburst_technical_document.pdf",
    "docs/features.md",
    "docs/server-api.md",
    "agent/README.md",
    "docs/quick-slope.md",
  ],
};

const CREDITED_VERBATIM = [
  "NASA, ESA, J. Hester and P. Scowen (Arizona State University), public domain",
  "a simulated Roman WFI L2 image",
  "VizieR at CDS",
];

const MENTIONED: Record<string, string[]> = {
  "author and archives": ["Samuel Krieger", "ESA Hubble Science Archive"],
  "project links": [
    "CONTRIBUTING.md",
    "CLA.md",
    "CODE_OF_CONDUCT.md",
    "SECURITY.md",
    "https://ko-fi.com/astroburst",
    "<!-- SUPPORTERS:START -->",
    "<!-- SUPPORTERS:END -->",
    "(LICENSE)",
    "src-tauri/licenses/THIRD-PARTY-NOTICES.md",
  ],
};

const VERBATIM = [
  "The headless server and the v2 API were contributed by [Jae-Joon Lee](https://github.com/leejjoon).",
  "The headless server, the v2 API, the Python client, the WCS engine migration, FITS tile compression, zscale and the first colormaps (gray and viridis) were contributed by [Jae-Joon Lee](https://github.com/leejjoon).",
  "This package is maintained by [jing2uo](https://github.com/jing2uo), not by the AstroBurst project.",
  "The network clause (section 13) covers the optional headless REST API: if you run a modified AstroBurst as a network service, you must offer the modified source to its users.",
  "AGPLv3. See [LICENSE](LICENSE).",
];

const read = (document: string) => readFileSync(repoPath(document), "utf8").replace(/\r\n/g, "\n");

const normalised = (markdown: string) => markdown.replace(/\s+/g, " ");

function section(markdown: string, heading: string): string {
  const start = markdown.indexOf(`\n## ${heading}\n`);
  if (start < 0) return "";
  const end = markdown.indexOf("\n## ", start + 1);
  return markdown.slice(start, end < 0 ? undefined : end);
}

const readme = () => normalised(read("README.md"));

const credits = () => normalised(section(read("README.md"), CREDITS_HEADING));

const escaped = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const mentions = (text: string, mention: string) =>
  new RegExp(`(?<![\\p{L}\\p{N}])${escaped(mention)}(?![\\p{L}\\p{N}])`, "u").test(text);

const missing = (text: string, mentionList: string[]) => mentionList.filter((mention) => !mentions(text, mention));

const slug = (heading: string) =>
  heading
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .replace(/\s/g, "-");

const withoutFencedBlocks = (markdown: string) => markdown.replace(/^```[\s\S]*?^```/gm, "");

const headingSlugs = (document: string) =>
  new Set([...withoutFencedBlocks(read(document)).matchAll(/^#{1,6}\s+(.+)$/gm)].map((match) => slug(match[1])));

function relativeLinks(markdown: string): string[] {
  return [...markdown.matchAll(/\]\(([^)\s]+)\)|(?:href|src)="([^"]+)"/g)]
    .map((match) => match[1] ?? match[2])
    .filter((target) => !/^[a-z][a-z0-9+.-]*:/i.test(target));
}

function brokenLinks(document: string): string[] {
  return relativeLinks(read(document)).filter((link) => {
    const [path, anchor] = link.split("#");
    const target = path === "" ? document : posix.join(posix.dirname(document), path);
    if (!existsSync(repoPath(target))) return true;
    return anchor !== undefined && target.endsWith(".md") && !headingSlugs(target).has(anchor);
  });
}

describe("README credits", () => {
  it.each(Object.entries(CREDITED))(`credits every mention of %s under "${CREDITS_HEADING}"`, (_, mentionList) => {
    expect(missing(credits(), mentionList)).toEqual([]);
  });

  it(`keeps the data and service credits verbatim under "${CREDITS_HEADING}"`, () => {
    const text = credits();
    expect(CREDITED_VERBATIM.filter((phrase) => !text.includes(phrase))).toEqual([]);
  });

  it.each(Object.entries(MENTIONED))("keeps every mention of %s", (_, mentionList) => {
    expect(missing(readme(), mentionList)).toEqual([]);
  });

  it("keeps the contributor, packaging and licence sentences verbatim", () => {
    const text = readme();
    expect(VERBATIM.filter((sentence) => !text.includes(sentence))).toEqual([]);
  });
});

describe("README and feature tour links", () => {
  it.each(["README.md", "docs/features.md"])("resolves every relative link in %s", (document) => {
    expect(brokenLinks(document)).toEqual([]);
  });
});
