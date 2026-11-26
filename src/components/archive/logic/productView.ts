import type { MastCalibration, MastPreset, MastProduct, MastProductClass, MastProductList } from "../../../shared/types/mast";
import { presetProducts } from "./presetView";

export type ProductMode = "opens" | "everything";
export type SectionKey = "preset" | "opens" | "intermediate" | "catalog" | "preview" | "association" | "auxiliary" | "other" | "hidden";

export interface ProductSection {
  key: SectionKey;
  title: string;
  detail: string;
  note: string | null;
  classes: MastProductClass[];
  products: MastProduct[];
  count: number;
  bytes: number;
  expanded: boolean;
}

export interface OpensPart {
  cls: MastProductClass;
  count: number;
  text: string;
}

export type SectionOverrides = Partial<Record<SectionKey, boolean>>;

export interface HiddenSummary {
  count: number;
  bytes: number;
  reference: number;
  guidestar: number;
  text: string;
}

export interface ProductView {
  summary: string;
  sections: ProductSection[];
  hidden: HiddenSummary;
  visible: MastProduct[];
  presetNote: string | null;
}

const UNITS: [number, string][] = [
  [1e12, "TB"],
  [1e9, "GB"],
  [1e6, "MB"],
  [1e3, "KB"],
];

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 1000) return `${Math.max(0, Math.round(bytes || 0))} B`;
  for (const [size, unit] of UNITS) {
    if (bytes < size) continue;
    const value = bytes / size;
    if (size >= 1e9) return `${value.toFixed(2)} ${unit}`;
    return value < 10 ? `${value.toFixed(1)} ${unit}` : `${Math.round(value)} ${unit}`;
  }
  return `${bytes} B`;
}

export function exactBytes(bytes: number): string {
  return `${Math.round(bytes).toLocaleString("en-US")} ${bytes === 1 ? "byte" : "bytes"}`;
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

const OPENS: readonly MastProductClass[] = ["image", "cube", "spectrum", "ramp"];
const INTERMEDIATE_SUFFIXES: ReadonlySet<string> = new Set(["uncal", "rate", "rateints", "ramp", "trapsfilled", "dark", "ima", "raw", "spt"]);

export const SUFFIX_MEANINGS: Readonly<Record<string, string>> = {
  uncal: "uncalibrated ramp",
  rate: "count-rate image of one exposure",
  rateints: "count-rate image per integration",
  cal: "calibrated exposure",
  calints: "calibrated exposure per integration",
  crf: "calibrated exposure with outliers flagged",
  i2d: "resampled and combined image",
  s2d: "rectified 2-D spectrum",
  s3d: "spectral cube",
  x1d: "extracted 1-D spectrum",
  x1dints: "1-D spectra per integration",
  c1d: "combined 1-D spectrum",
  flt: "calibrated exposure",
  flc: "calibrated exposure with CTE correction",
  drz: "drizzled image",
  drc: "drizzled image with CTE correction",
  raw: "raw exposure",
  ima: "intermediate exposure",
};

export function isIntermediate(product: Pick<MastProduct, "cls" | "suffix">): boolean {
  return product.cls === "ramp" || INTERMEDIATE_SUFFIXES.has(product.suffix.toLowerCase());
}

export function isScience(product: Pick<MastProduct, "cls" | "suffix">): boolean {
  return OPENS.includes(product.cls) && !isIntermediate(product);
}
const NOUNS: Record<string, [string, string]> = {
  image: ["image", "images"],
  cube: ["cube", "cubes"],
  spectrum: ["spectrum", "spectra"],
  ramp: ["ramp", "ramps"],
};

const SECTIONS: { key: Exclude<SectionKey, "opens" | "hidden">; title: string; classes: MastProductClass[]; note: string | null; suffix: string }[] = [
  { key: "catalog", title: "Catalogues and tables", classes: ["catalog"], note: "ECSV tables open in Files as table rows", suffix: "" },
  { key: "preview", title: "Previews", classes: ["preview"], note: null, suffix: " (see Previews)" },
  { key: "association", title: "Association", classes: ["association"], note: null, suffix: "" },
  { key: "auxiliary", title: "Auxiliary", classes: ["auxiliary"], note: null, suffix: "" },
  { key: "other", title: "Other", classes: ["other"], note: null, suffix: "" },
];

function sum(products: readonly MastProduct[]): number {
  return products.reduce((s, p) => s + p.sizeBytes, 0);
}

function opensDetail(products: readonly MastProduct[]): string {
  const parts = OPENS.map((cls) => [cls, products.filter((p) => p.cls === cls).length] as const)
    .filter(([, n]) => n > 0)
    .map(([cls, n]) => plural(n, NOUNS[cls][0], NOUNS[cls][1]));
  return `${parts.join(", ")}, ${formatBytes(sum(products))}`;
}

function cachedText(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())} UTC`;
}

export function productSummary(list: MastProductList): string {
  const head = `${plural(list.totalCount, "file", "files")}, ${formatBytes(list.totalBytes)} at MAST`;
  if (list.fromCache) return `${head} · from the cache`;
  const listed = `listed in ${(list.elapsedMs / 1000).toFixed(1)} s`;
  return list.cachedUntil ? `${head} · ${listed} · cached until ${cachedText(list.cachedUntil)}` : `${head} · ${listed}`;
}

function presetSection(preset: MastPreset, products: MastProduct[]): ProductSection {
  const bytes = sum(products);
  return {
    key: "preset",
    title: `${preset.label} files`,
    detail: `${plural(products.length, "file", "files")}, ${formatBytes(bytes)}`,
    note: null,
    classes: [...new Set(products.map((p) => p.cls))],
    products,
    count: products.length,
    bytes,
    expanded: true,
  };
}

export function productView(list: MastProductList, mode: ProductMode, showHidden: boolean, preset: MastPreset | null = null): ProductView {
  const everything = mode === "everything";
  const sections: ProductSection[] = [];
  const picked = preset ? presetProducts(preset, list.products) : [];
  const focused = picked.length > 0;
  if (focused && preset) sections.push(presetSection(preset, picked));
  const rest = list.products.filter((p) => !picked.includes(p));
  const opens = rest.filter(isScience);
  if (opens.length) {
    sections.push({ key: "opens", title: "Science files", detail: opensDetail(opens), note: null, classes: [...OPENS], products: opens, count: opens.length, bytes: sum(opens), expanded: everything || !focused });
  }
  const intermediate = rest.filter((p) => OPENS.includes(p.cls) && isIntermediate(p));
  if (intermediate.length) {
    sections.push({
      key: "intermediate",
      title: "Intermediate files",
      detail: opensDetail(intermediate),
      note: "earlier pipeline stages, such as rate and uncal",
      classes: [...OPENS],
      products: intermediate,
      count: intermediate.length,
      bytes: sum(intermediate),
      expanded: everything,
    });
  }
  for (const spec of SECTIONS) {
    const products = rest.filter((p) => spec.classes.includes(p.cls));
    if (!products.length) continue;
    const bytes = sum(products);
    sections.push({
      key: spec.key,
      title: spec.title,
      detail: `${plural(products.length, "file", "files")}, ${formatBytes(bytes)}${spec.suffix}`,
      note: spec.note,
      classes: [...spec.classes],
      products,
      count: products.length,
      bytes,
      expanded: everything || (!focused && spec.key === "catalog"),
    });
  }
  const hiddenProducts = list.products.filter((p) => p.cls === "reference" || p.cls === "guidestar");
  const reference = hiddenProducts.filter((p) => p.cls === "reference").length;
  const guidestar = hiddenProducts.length - reference;
  const hiddenBytes = sum(hiddenProducts);
  const hidden: HiddenSummary = {
    count: hiddenProducts.length,
    bytes: hiddenBytes,
    reference,
    guidestar,
    text: `${plural(hiddenProducts.length, "file", "files")}, ${formatBytes(hiddenBytes)}: ${plural(reference, "CRDS reference file", "CRDS reference files")}, ${plural(guidestar, "guide-star file", "guide-star files")}`,
  };
  if (showHidden && hiddenProducts.length) {
    sections.push({ key: "hidden", title: "Hidden", detail: hidden.text, note: null, classes: ["reference", "guidestar"], products: hiddenProducts, count: hiddenProducts.length, bytes: hiddenBytes, expanded: true });
  }
  const visible = sections.filter((s) => s.expanded).flatMap((s) => s.products);
  const presetNote = preset && !focused ? `No ${preset.label} files (${preset.products.join(", ")}) in this dataset` : null;
  return { summary: productSummary(list), sections, hidden, visible, presetNote };
}

export const CLASS_LABELS: Readonly<Record<MastProductClass, string>> = {
  image: "Image",
  cube: "Cube",
  spectrum: "Spectrum",
  ramp: "Ramp",
  catalog: "Catalogue",
  preview: "Preview",
  association: "Association",
  auxiliary: "Auxiliary",
  reference: "CRDS reference",
  guidestar: "Guide star",
  other: "Other",
};

export function productLabel(product: Pick<MastProduct, "cls" | "suffix" | "filename">): string {
  const suffix = product.suffix.trim().toLowerCase();
  if (OPENS.includes(product.cls) && suffix) return suffix;
  if (product.cls !== "catalog") return CLASS_LABELS[product.cls];
  if (product.suffix === "whtlt" || product.filename.toLowerCase().endsWith("_whtlt.ecsv")) return "Light curve";
  if (product.suffix === "segm") return "Segmentation map";
  return CLASS_LABELS.catalog;
}

export function productLabelTitle(product: Pick<MastProduct, "cls" | "suffix">): string {
  const meaning = SUFFIX_MEANINGS[product.suffix.trim().toLowerCase()];
  return meaning ? `${CLASS_LABELS[product.cls]}: ${meaning}` : CLASS_LABELS[product.cls];
}

export function scienceSelection(view: ProductView, refusalOf: (product: MastProduct) => string | null): MastProduct[] {
  const preset = view.sections.find((s) => s.key === "preset");
  const source = preset ? preset.products : (view.sections.find((s) => s.key === "opens")?.products ?? []);
  return source.filter((p) => refusalOf(p) === null);
}

export interface ProductExtras {
  jdavizUri: string | null;
  calibrationVersion: string | null;
  crdsVersion: string | null;
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

export function calibrationOf(product: MastProduct): MastCalibration | null {
  return product.calibration ?? null;
}

export function productExtras(product: MastProduct): ProductExtras {
  const raw = product as MastProduct & Record<string, unknown>;
  const calibration = calibrationOf(product);
  return {
    jdavizUri: text(raw.jdavizUri),
    calibrationVersion: text(calibration?.calibrationSoftwareVersion),
    crdsVersion: text(calibration?.crdsContext),
  };
}

export function versionText(extras: ProductExtras): string | null {
  const parts = [extras.calibrationVersion ? `calibration ${extras.calibrationVersion}` : null, extras.crdsVersion ? `CRDS ${extras.crdsVersion}` : null].filter(Boolean);
  return parts.length ? parts.join(", ") : null;
}

export function copyJdavizUris(products: readonly MastProduct[]): string {
  return products
    .map((p) => productExtras(p).jdavizUri)
    .filter((uri): uri is string => uri !== null)
    .join("\n");
}

export function shortFilename(filename: string, datasetId: string): string {
  const prefix = `${datasetId.toLowerCase()}_`;
  if (!filename.toLowerCase().startsWith(prefix) || filename.length <= prefix.length) return filename;
  return `…_${filename.slice(prefix.length)}`;
}

export function filenameParts(filename: string, datasetId: string): { head: string; tail: string } {
  const shown = shortFilename(filename, datasetId);
  const cut = shown.lastIndexOf("_");
  if (cut <= 0) return { head: shown, tail: "" };
  return { head: shown.slice(0, cut), tail: shown.slice(cut) };
}

export function sectionExpanded(section: ProductSection, overrides: SectionOverrides): boolean {
  return overrides[section.key] ?? section.expanded;
}

export function visibleProducts(view: ProductView, overrides: SectionOverrides): MastProduct[] {
  return view.sections.filter((s) => sectionExpanded(s, overrides)).flatMap((s) => s.products);
}

export function opensParts(section: ProductSection): OpensPart[] {
  return OPENS.map((cls) => ({ cls, count: section.products.filter((p) => p.cls === cls).length }))
    .filter((p) => p.count > 0)
    .map((p) => ({ ...p, text: plural(p.count, NOUNS[p.cls][0], NOUNS[p.cls][1]) }));
}

export function copyMastUris(products: readonly MastProduct[]): string {
  return products.map((p) => p.mastUri).join("\n");
}

export function copyDownloadUrls(products: readonly MastProduct[]): string {
  return products.map((p) => p.downloadUrl).join("\n");
}
