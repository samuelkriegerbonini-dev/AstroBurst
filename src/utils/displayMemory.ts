import type { StfParams } from "../shared/types";
import {
  COLORMAP_NAMES,
  DEFAULT_DISPLAY_SETTINGS,
  DISPLAY_LOOK_KEYS,
  GRID_FRAMES,
  LIMIT_MODES,
  STRETCH_MODES,
  type DisplayLook,
  type DisplaySettings,
  type LimitMode,
  type StretchMode,
} from "../shared/types/display";
import { displayFilterValue, type ChannelSource } from "./channelMapping";
import { reconcileDisplayPatch } from "./displayTransfer";
import { clampGridDensity } from "./gridSteps";

export const DISPLAY_MEMORY_KEY = "astroburst.display.files.v1";
export const LEGACY_DISPLAY_KEY = "astroburst.display.v1";
export const DISPLAY_MEMORY_FILE_CAP = 200;
export const DISPLAY_MEMORY_GROUP_CAP = 64;
export const IDENTITY_STF: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };

export const LINEAR_OFF_TITLE =
  "Linear: show the data linearly from its minimum to its maximum (stretch linear, limits minmax, STF reset). Click again to go back";
export const LINEAR_HINT_TITLE = "This result sets its own display; Linear is off while it is shown";
export const RGB_LINEAR_OFF_TITLE =
  "Linear: show each channel linearly from its own minimum to its maximum (per-channel STF reset). Click again to go back";
export const RGB_LINEAR_ON_TITLE = "Back to the previous channel STF";
export const RGB_LINEAR_BAKED = "STF is baked in: the composite on screen is display-referred";

export interface LinearUndo {
  stretch: StretchMode;
  limits: LimitMode;
}

export interface StfSlot {
  source: string;
  params: StfParams;
}

export interface RgbStf {
  r: StfParams;
  g: StfParams;
  b: StfParams;
  linked: boolean;
}

export interface RgbAutoStf {
  r: StfParams;
  g: StfParams;
  b: StfParams;
}

export interface FileDisplayEntry {
  look?: DisplayLook;
  linear?: LinearUndo;
  stf?: StfSlot;
  stfProcessed?: StfSlot;
  rgb?: RgbStf;
  rgbLinear?: RgbStf;
}

export interface GroupDisplayEntry {
  look: DisplayLook;
  linear: LinearUndo | null;
}

export interface DisplayMemoryDoc {
  v: 1;
  defaults: DisplayLook;
  sameGroup: boolean;
  files: [string, FileDisplayEntry][];
  groups: [string, GroupDisplayEntry][];
}

export type LookSource = "file" | "group" | "default";

export interface ResolvedDisplay {
  look: DisplayLook;
  linear: LinearUndo | null;
  source: LookSource;
}

export interface DisplayEdit {
  patch: Partial<DisplaySettings>;
  linear: LinearUndo | null;
  restoreStf: boolean;
}

export type StfSlotName = "stf" | "stfProcessed";

export interface StfSourceInput {
  filePath: string | null;
  processedPath: string | null;
  version: number;
  excludeDq: boolean;
}

export interface RgbLinearToggle {
  next: RgbStf;
  rgbLinear: RgbStf | null;
}

function finiteOr(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function finiteOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function sanitizeDisplaySettings(raw: unknown): DisplaySettings {
  const d = DEFAULT_DISPLAY_SETTINGS;
  if (!raw || typeof raw !== "object") return d;
  const r = raw as Record<string, unknown>;
  const stretch = STRETCH_MODES.find((s) => s === r.stretch) ?? d.stretch;
  const limits = LIMIT_MODES.find((l) => l === r.limits) ?? d.limits;
  const colormap = COLORMAP_NAMES.find((c) => c === r.colormap) ?? d.colormap;
  const gridFrame = GRID_FRAMES.find((f) => f === r.gridFrame) ?? d.gridFrame;
  const candidate: DisplaySettings = {
    stretch,
    limits,
    percentileLow: finiteOr(r.percentileLow, d.percentileLow),
    percentileHigh: finiteOr(r.percentileHigh, d.percentileHigh),
    userLo: finiteOrNull(r.userLo),
    userHi: finiteOrNull(r.userHi),
    zscaleContrast: finiteOr(r.zscaleContrast, d.zscaleContrast),
    asinhA: finiteOr(r.asinhA, d.asinhA),
    power: finiteOr(r.power, d.power),
    colormap,
    invert: r.invert === true,
    grid: r.grid === true,
    gridFrame,
    gridDensity: clampGridDensity(finiteOr(r.gridDensity, d.gridDensity)),
    compass: r.compass === true,
    colorbar: r.colorbar !== false,
    symmetric: r.symmetric === true,
    centre: finiteOr(r.centre, d.centre),
  };
  return reconcileDisplayPatch(candidate, {});
}

export function pickLook(d: DisplayLook): DisplayLook {
  return {
    stretch: d.stretch,
    limits: d.limits,
    percentileLow: d.percentileLow,
    percentileHigh: d.percentileHigh,
    userLo: d.userLo,
    userHi: d.userHi,
    zscaleContrast: d.zscaleContrast,
    asinhA: d.asinhA,
    power: d.power,
    colormap: d.colormap,
    invert: d.invert,
    symmetric: d.symmetric,
    centre: d.centre,
  };
}


export function withLook(d: DisplaySettings, look: DisplayLook): DisplaySettings {
  return reconcileDisplayPatch(d, look);
}

export function patchTouchesLook(patch: Partial<DisplaySettings>): boolean {
  return DISPLAY_LOOK_KEYS.some((key) => key in patch);
}

export function defaultsLook(look: DisplayLook): DisplayLook {
  return { ...pickLook(look), limits: look.limits === "user" ? "minmax" : look.limits, userLo: null, userHi: null };
}

export function displayGroupKey(file: ChannelSource | null): string | null {
  if (!file) return null;
  const raw = file.result?.header?.INSTRUME;
  const instrument = raw == null ? "" : String(raw).trim().toUpperCase();
  const filter = (displayFilterValue(file) ?? "").trim().toUpperCase();
  return instrument && filter ? `${instrument}|${filter}` : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sanitizeLook(raw: unknown): DisplayLook {
  return pickLook(sanitizeDisplaySettings(raw));
}

function legacyDisplay(legacyText: string | null): DisplaySettings {
  if (!legacyText) return DEFAULT_DISPLAY_SETTINGS;
  try {
    return sanitizeDisplaySettings(JSON.parse(legacyText));
  } catch {
    return DEFAULT_DISPLAY_SETTINGS;
  }
}

export function emptyDisplayMemory(legacyText: string | null): DisplayMemoryDoc {
  return { v: 1, defaults: defaultsLook(pickLook(legacyDisplay(legacyText))), sameGroup: true, files: [], groups: [] };
}

function finiteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function copyStf(s: StfParams): StfParams {
  return { shadow: s.shadow, midtone: s.midtone, highlight: s.highlight };
}

function parseStfParams(raw: unknown): StfParams | null {
  if (!isRecord(raw) || !finiteNumber(raw.shadow) || !finiteNumber(raw.midtone) || !finiteNumber(raw.highlight)) return null;
  return { shadow: raw.shadow, midtone: raw.midtone, highlight: raw.highlight };
}

function parseLinear(raw: unknown): LinearUndo | null {
  if (!isRecord(raw)) return null;
  const stretch = STRETCH_MODES.find((s) => s === raw.stretch);
  const limits = LIMIT_MODES.find((l) => l === raw.limits);
  return stretch && limits ? { stretch, limits } : null;
}

const MALFORMED = Symbol("malformed");

function parseSlot(raw: unknown): StfSlot | undefined | typeof MALFORMED {
  if (raw === undefined) return undefined;
  if (!isRecord(raw) || typeof raw.source !== "string") return MALFORMED;
  const params = parseStfParams(raw.params);
  return params ? { source: raw.source, params } : MALFORMED;
}

function parseRgb(raw: unknown): RgbStf | undefined | typeof MALFORMED {
  if (raw === undefined) return undefined;
  if (!isRecord(raw) || typeof raw.linked !== "boolean") return MALFORMED;
  const r = parseStfParams(raw.r);
  const g = parseStfParams(raw.g);
  const b = parseStfParams(raw.b);
  return r && g && b ? { r, g, b, linked: raw.linked } : MALFORMED;
}

function makeEntry(e: FileDisplayEntry): FileDisplayEntry {
  const out: FileDisplayEntry = {};
  if (e.look) out.look = e.look;
  if (e.linear) out.linear = e.linear;
  if (e.stf) out.stf = e.stf;
  if (e.stfProcessed) out.stfProcessed = e.stfProcessed;
  if (e.rgb) out.rgb = e.rgb;
  if (e.rgbLinear) out.rgbLinear = e.rgbLinear;
  return out;
}

function isEmptyEntry(e: FileDisplayEntry): boolean {
  return !e.look && !e.linear && !e.stf && !e.stfProcessed && !e.rgb && !e.rgbLinear;
}

function parseFileRow(row: unknown): [string, FileDisplayEntry] | null {
  if (!Array.isArray(row) || row.length !== 2 || typeof row[0] !== "string" || !isRecord(row[1])) return null;
  const raw = row[1];
  const stf = parseSlot(raw.stf);
  const stfProcessed = parseSlot(raw.stfProcessed);
  const rgb = parseRgb(raw.rgb);
  const rgbLinear = parseRgb(raw.rgbLinear);
  if (stf === MALFORMED || stfProcessed === MALFORMED || rgb === MALFORMED || rgbLinear === MALFORMED) return null;
  const entry = makeEntry({
    look: isRecord(raw.look) ? sanitizeLook(raw.look) : undefined,
    linear: parseLinear(raw.linear) ?? undefined,
    stf,
    stfProcessed,
    rgb,
    rgbLinear,
  });
  return isEmptyEntry(entry) ? null : [row[0], entry];
}

function parseGroupRow(row: unknown): [string, GroupDisplayEntry] | null {
  if (!Array.isArray(row) || row.length !== 2 || typeof row[0] !== "string" || !isRecord(row[1]) || !isRecord(row[1].look)) return null;
  return [row[0], { look: sanitizeLook(row[1].look), linear: parseLinear(row[1].linear) }];
}

function rowsOf<T>(raw: unknown, parse: (row: unknown) => [string, T] | null, cap: number): [string, T][] {
  if (!Array.isArray(raw)) return [];
  const byKey = new Map<string, T>();
  for (const row of raw) {
    const parsed = parse(row);
    if (!parsed) continue;
    byKey.delete(parsed[0]);
    byKey.set(parsed[0], parsed[1]);
  }
  return capList([...byKey], cap);
}

export function parseDisplayMemory(text: string | null, legacyText: string | null): DisplayMemoryDoc {
  if (!text) return emptyDisplayMemory(legacyText);
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return emptyDisplayMemory(legacyText);
  }
  if (!isRecord(raw) || raw.v !== 1) return emptyDisplayMemory(legacyText);
  return {
    v: 1,
    defaults: sanitizeLook(raw.defaults),
    sameGroup: raw.sameGroup !== false,
    files: rowsOf(raw.files, parseFileRow, DISPLAY_MEMORY_FILE_CAP),
    groups: rowsOf(raw.groups, parseGroupRow, DISPLAY_MEMORY_GROUP_CAP),
  };
}

export function serializeDisplayMemory(doc: DisplayMemoryDoc): string {
  return JSON.stringify(doc);
}

function capList<T>(list: [string, T][], cap: number): [string, T][] {
  return list.length > cap ? list.slice(list.length - cap) : list;
}

function putLast<T>(list: [string, T][], key: string, value: T, cap: number): [string, T][] {
  return capList([...list.filter(([k]) => k !== key), [key, value]], cap);
}

function isLast<T>(list: [string, T][], key: string): boolean {
  return list.length > 0 && list[list.length - 1][0] === key;
}

function rowOf<T>(list: [string, T][], key: string | null): T | null {
  if (key === null) return null;
  const row = list.find(([k]) => k === key);
  return row ? row[1] : null;
}

export function fileEntry(doc: DisplayMemoryDoc, path: string | null): FileDisplayEntry | null {
  return rowOf(doc.files, path);
}

function putEntry(doc: DisplayMemoryDoc, path: string, entry: FileDisplayEntry): DisplayMemoryDoc {
  const next = makeEntry(entry);
  if (isEmptyEntry(next)) {
    return fileEntry(doc, path) ? { ...doc, files: doc.files.filter(([k]) => k !== path) } : doc;
  }
  return { ...doc, files: putLast(doc.files, path, next, DISPLAY_MEMORY_FILE_CAP) };
}

function sameLook(a: DisplayLook, b: DisplayLook): boolean {
  return DISPLAY_LOOK_KEYS.every((key) => a[key] === b[key]);
}

function sameLinear(a: LinearUndo | null | undefined, b: LinearUndo | null | undefined): boolean {
  if (!a || !b) return !a && !b;
  return a.stretch === b.stretch && a.limits === b.limits;
}

function sameStf(a: StfParams, b: StfParams): boolean {
  return a.shadow === b.shadow && a.midtone === b.midtone && a.highlight === b.highlight;
}

function sameSlot(a: StfSlot | undefined, b: StfSlot | null): boolean {
  if (!a || !b) return !a && !b;
  return a.source === b.source && sameStf(a.params, b.params);
}

export function sameRgbStf(a: RgbStf, b: RgbStf): boolean {
  return a.linked === b.linked && sameStf(a.r, b.r) && sameStf(a.g, b.g) && sameStf(a.b, b.b);
}

function copyRgb(s: RgbStf): RgbStf {
  return { r: copyStf(s.r), g: copyStf(s.g), b: copyStf(s.b), linked: s.linked };
}

function identityUnlinked(): RgbStf {
  return { r: copyStf(IDENTITY_STF), g: copyStf(IDENTITY_STF), b: copyStf(IDENTITY_STF), linked: false };
}

export function resolveFileDisplay(doc: DisplayMemoryDoc, path: string | null, groupKey: string | null): ResolvedDisplay {
  const entry = fileEntry(doc, path);
  if (entry?.look) return { look: entry.look, linear: entry.linear ?? null, source: "file" };
  const group = doc.sameGroup ? rowOf(doc.groups, groupKey) : null;
  if (group) return { look: group.look, linear: group.linear, source: "group" };
  return { look: doc.defaults, linear: null, source: "default" };
}

export function rememberLook(
  doc: DisplayMemoryDoc,
  path: string,
  groupKey: string | null,
  look: DisplayLook,
  linear: LinearUndo | null,
): DisplayMemoryDoc {
  const prev = fileEntry(doc, path);
  const group = rowOf(doc.groups, groupKey);
  const fileSame = prev?.look !== undefined && isLast(doc.files, path) && sameLook(prev.look, look) && sameLinear(prev.linear, linear);
  const groupSame = groupKey === null || (group !== null && isLast(doc.groups, groupKey) && sameLook(group.look, look) && sameLinear(group.linear, linear));
  if (fileSame && groupSame) return doc;
  const stored = pickLook(look);
  const withFile = putEntry(doc, path, { ...prev, look: stored, linear: linear ?? undefined });
  if (groupKey === null) return withFile;
  return { ...withFile, groups: putLast(withFile.groups, groupKey, { look: stored, linear }, DISPLAY_MEMORY_GROUP_CAP) };
}

export function rememberStf(doc: DisplayMemoryDoc, path: string, slot: StfSlotName, value: StfSlot | null): DisplayMemoryDoc {
  const prev = fileEntry(doc, path);
  if (sameSlot(prev?.[slot], value) && (value === null || isLast(doc.files, path))) return doc;
  const stored = value ? { source: value.source, params: copyStf(value.params) } : undefined;
  return putEntry(doc, path, slot === "stf" ? { ...prev, stf: stored } : { ...prev, stfProcessed: stored });
}

export function forgetStf(doc: DisplayMemoryDoc, path: string): DisplayMemoryDoc {
  const prev = fileEntry(doc, path);
  if (!prev || (!prev.stf && !prev.stfProcessed)) return doc;
  const next = makeEntry({ ...prev, stf: undefined, stfProcessed: undefined });
  if (isEmptyEntry(next)) return { ...doc, files: doc.files.filter(([k]) => k !== path) };
  return { ...doc, files: doc.files.map((row): [string, FileDisplayEntry] => (row[0] === path ? [path, next] : row)) };
}

export function rememberRgbEdit(doc: DisplayMemoryDoc, path: string, current: RgbStf, auto: RgbAutoStf): DisplayMemoryDoc {
  const prev = fileEntry(doc, path);
  if (prev?.rgbLinear && sameRgbStf(current, identityUnlinked())) return doc;
  if (!prev?.rgb && !prev?.rgbLinear && sameRgbStf(current, { ...auto, linked: false })) return doc;
  if (prev?.rgb && !prev.rgbLinear && sameRgbStf(prev.rgb, current) && isLast(doc.files, path)) return doc;
  return putEntry(doc, path, { ...prev, rgb: copyRgb(current), rgbLinear: undefined });
}

export function setRgbLinear(doc: DisplayMemoryDoc, path: string, saved: RgbStf | null): DisplayMemoryDoc {
  const prev = fileEntry(doc, path);
  if (!saved && !prev?.rgbLinear) return doc;
  return putEntry(doc, path, { ...prev, rgbLinear: saved ? copyRgb(saved) : undefined });
}

export function touchFile(doc: DisplayMemoryDoc, path: string): DisplayMemoryDoc {
  const entry = fileEntry(doc, path);
  if (!entry || isLast(doc.files, path)) return doc;
  return { ...doc, files: putLast(doc.files, path, entry, DISPLAY_MEMORY_FILE_CAP) };
}

export function setDefaults(doc: DisplayMemoryDoc, look: DisplayLook): DisplayMemoryDoc {
  return sameLook(doc.defaults, look) ? doc : { ...doc, defaults: pickLook(look) };
}

export function setSameGroup(doc: DisplayMemoryDoc, on: boolean): DisplayMemoryDoc {
  return doc.sameGroup === on ? doc : { ...doc, sameGroup: on };
}

export function stfSourceKey({ filePath, processedPath, version, excludeDq }: StfSourceInput): string {
  return `${processedPath ?? filePath}|${processedPath ? version : 0}|${excludeDq ? 1 : 0}`;
}

export function resolveStf(
  entry: FileDisplayEntry | null,
  source: string,
  isProcessed: boolean,
  auto: StfParams | null,
  linearOn: boolean,
): StfParams | null {
  if (linearOn) return IDENTITY_STF;
  const slot = isProcessed ? entry?.stfProcessed : entry?.stf;
  if (slot && slot.source === source) return slot.params;
  return auto;
}

export function linearToggle(display: DisplaySettings, linear: LinearUndo | null): DisplayEdit {
  if (!linear) {
    return { patch: { stretch: "linear", limits: "minmax" }, linear: { stretch: display.stretch, limits: display.limits }, restoreStf: false };
  }
  return { patch: { stretch: linear.stretch, limits: linear.limits }, linear: null, restoreStf: true };
}

export function editWithLinear(patch: Partial<DisplaySettings>, linear: LinearUndo | null): DisplayEdit {
  if (linear && ("stretch" in patch || "limits" in patch)) return { patch, linear: null, restoreStf: true };
  return { patch, linear, restoreStf: false };
}

export function rgbSeed(entry: FileDisplayEntry | null, auto: RgbAutoStf): RgbStf {
  if (entry?.rgbLinear) return identityUnlinked();
  return entry?.rgb ?? { ...auto, linked: false };
}

export function rgbLinearToggle(current: RgbStf, saved: RgbStf | null): RgbLinearToggle {
  return saved ? { next: saved, rgbLinear: null } : { next: identityUnlinked(), rgbLinear: current };
}

export function linearTitle(linear: LinearUndo | null): string {
  if (!linear) return LINEAR_OFF_TITLE;
  return linear.stretch === "mtf" ? "Back to mtf" : `Back to ${linear.stretch} with ${linear.limits} limits`;
}

export function rgbLinearTitle(on: boolean): string {
  return on ? RGB_LINEAR_ON_TITLE : RGB_LINEAR_OFF_TITLE;
}
