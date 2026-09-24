import type { ProcessResult } from "../shared/types";
import type { FileRenderState } from "../shared/types/preview";
import type { WizardState } from "./wizard";
import { assetUrlToPath } from "./exportSources";
import { parseImageRef } from "./imageRef";
import { CHAIN_ORDER, normalizeOutputPath } from "./processingChain";

const URL_SCHEME = /^(asset|https?|data|blob):/i;

export function urlToLocalPath(url: string | null | undefined): string | null {
  if (!url) return null;
  if (URL_SCHEME.test(url)) return assetUrlToPath(url);
  const end = url.search(/[?#]/);
  return end >= 0 ? url.slice(0, end) : url;
}

export interface KeepFile {
  path: string;
  sourcePath: string;
  result: Pick<ProcessResult, "png_path" | "previewUrl" | "resampled" | "resampledPath"> | null;
}

export type KeepWizard = Pick<
  WizardState,
  "stackedPaths" | "backgroundPaths" | "channelResults" | "starMaskPath" | "segmPath" | "resultPng" | "resultFits"
>;

export interface KeepInput {
  files: readonly KeepFile[];
  records: readonly FileRenderState[];
  wizard?: KeepWizard | null;
  previewUrls?: readonly (string | null | undefined)[];
}

export function outputKeepList(input: KeepInput): string[] {
  const kept = new Map<string, string>();
  const add = (value: string | null | undefined) => {
    if (!value || value.startsWith("__")) return;
    const path = parseImageRef(value).path;
    const key = normalizeOutputPath(path);
    if (!kept.has(key)) kept.set(key, path);
  };
  const addUrl = (url: string | null | undefined) => add(urlToLocalPath(url));

  for (const file of input.files) {
    add(file.sourcePath);
    add(file.path);
    const r = file.result;
    if (!r) continue;
    add(r.png_path);
    addUrl(r.previewUrl);
    add(r.resampledPath);
    add(r.resampled?.fits_path);
    add(r.resampled?.png_path);
    addUrl(r.resampled?.previewUrl);
  }

  for (const record of input.records) {
    const p = record.processed;
    if (p) {
      add(p.fitsPath);
      addUrl(p.previewUrl);
      add(p.inputPath);
    }
    for (const step of CHAIN_ORDER) {
      const entry = record.chain.steps[step];
      if (!entry) continue;
      add(entry.fitsPath);
      addUrl(entry.previewUrl);
    }
  }

  const w = input.wizard;
  if (w) {
    Object.values(w.stackedPaths).forEach(add);
    Object.values(w.backgroundPaths).forEach(add);
    for (const result of Object.values(w.channelResults)) {
      add(result.starless?.path);
      add(result.stretched?.path);
    }
    add(w.starMaskPath);
    add(w.segmPath);
    add(w.resultFits);
    addUrl(w.resultPng);
  }

  for (const url of input.previewUrls ?? []) addUrl(url);

  return [...kept.values()];
}
