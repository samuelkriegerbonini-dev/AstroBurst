import type { MastPreset, MastProduct, MastProductClass, MastProductList } from "../../../shared/types/mast";
import { presetProducts } from "./presetView";

export type ProductMode = "opens" | "everything";
export type SectionKey = "preset" | "opens" | "catalog" | "preview" | "association" | "auxiliary" | "other" | "hidden";

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
const NOUNS: Record<string, [string, string]> = {
  image: ["image", "images"],
  cube: ["cube", "cubes"],
  spectrum: ["spectrum", "spectra"],
  ramp: ["ramp", "ramps"],
};

const SECTIONS: { key: Exclude<SectionKey, "opens" | "hidden">; title: string; classes: MastProductClass[]; note: string | null; suffix: string }[] = [
  { key: "catalog", title: "Catalogues", classes: ["catalog"], note: "overlay arrives with C18", suffix: "" },
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
  const opens = rest.filter((p) => OPENS.includes(p.cls));
  if (opens.length) {
    sections.push({ key: "opens", title: "Opens in the app", detail: opensDetail(opens), note: null, classes: [...OPENS], products: opens, count: opens.length, bytes: sum(opens), expanded: everything || !focused });
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

export function shortFilename(filename: string, datasetId: string): string {
  const prefix = `${datasetId.toLowerCase()}_`;
  if (!filename.toLowerCase().startsWith(prefix) || filename.length <= prefix.length) return filename;
  return `…_${filename.slice(prefix.length)}`;
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
