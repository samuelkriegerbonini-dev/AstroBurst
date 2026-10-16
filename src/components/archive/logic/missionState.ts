import type { MastError, MastMission, MastMissionId, MastTokenStatus } from "../../../shared/types/mast";
import { errorView, operationName } from "./errorCopy";

export type RomanLock = "locked" | "checking" | "unlocked" | "expired";
export type MissionAccent = "amber" | "sky" | "violet" | "muted";
export type ResultsGate = "open" | "locked" | "unavailable";

export interface MissionEntry {
  id: MastMissionId;
  label: string;
  placement: "tab" | "more";
  enabled: boolean;
  reason: string | null;
  lock: RomanLock | null;
  accent: MissionAccent;
}

export interface TokenView {
  status: MastTokenStatus | null;
  checking: boolean;
  expired: boolean;
}

export interface ViewAvailability {
  gallery: boolean;
  galleryReason: string | null;
  previews: boolean;
  previewsReason: string | null;
  products: boolean;
  productsReason: string | null;
}

export const TAB_MISSIONS: readonly MastMissionId[] = ["jwst", "hst", "roman"];
export const MORE_ORDER: readonly MastMissionId[] = ["iue", "classy", "ullyses", "roman_cgi", "roman_spectra", "tess", "multi_mission"];
export const ROMAN_SEARCH_DISABLED = "Roman needs an authorized MAST token";

const OFFER_NOTES: Partial<Record<MastMissionId, string>> = {
  iue: "search and metadata; no preview images at MAST",
  classy: "search and metadata (galaxy sample); no previews",
  ullyses: "search and metadata (targets); no previews",
};

const ROMAN_CGI_REASON = "needs Roman authorization; coronagraphy is not processed by AstroBurst";

const ACCENTS: Partial<Record<MastMissionId, MissionAccent>> = { jwst: "amber", hst: "sky", roman: "violet" };

export function romanLock(token: TokenView): RomanLock {
  if (token.checking) return "checking";
  const status = token.status;
  if (!status || !status.present) return "locked";
  if (token.expired) return "expired";
  return status.authorized === true ? "unlocked" : "locked";
}

function notDeployedReason(m: MastMission): string {
  return `not deployed by MAST (HTTP 404, checked ${m.checkedOn})`;
}

function entryFor(m: MastMission, roman: RomanLock): MissionEntry {
  const placement = TAB_MISSIONS.includes(m.id) ? "tab" : "more";
  const accent = ACCENTS[m.id] ?? "muted";
  if (m.id === "roman") return { id: m.id, label: m.label, placement, enabled: true, reason: null, lock: roman, accent };
  if (m.id === "roman_cgi") return { id: m.id, label: m.label, placement, enabled: false, reason: ROMAN_CGI_REASON, lock: null, accent };
  if (m.availability === "not_deployed") return { id: m.id, label: m.label, placement, enabled: false, reason: notDeployedReason(m), lock: null, accent };
  if (m.availability === "unsupported" || !m.capabilities.search) {
    return { id: m.id, label: m.label, placement, enabled: false, reason: m.note ?? "not supported by AstroBurst yet", lock: null, accent };
  }
  if (m.availability === "token") return { id: m.id, label: m.label, placement, enabled: false, reason: ROMAN_CGI_REASON, lock: null, accent };
  const reason = placement === "more" && m.id in OFFER_NOTES ? OFFER_NOTES[m.id] ?? null : null;
  return { id: m.id, label: m.label, placement, enabled: true, reason, lock: null, accent };
}

export function missionEntries(missions: readonly MastMission[], roman: RomanLock): MissionEntry[] {
  return missions.map((m) => entryFor(m, roman));
}

export function tabMissions(entries: readonly MissionEntry[], current: MastMissionId): MissionEntry[] {
  const tabs = TAB_MISSIONS.map((id) => entries.find((e) => e.id === id)).filter((e): e is MissionEntry => !!e);
  const extra = entries.find((e) => e.id === current && e.placement === "more");
  return extra ? [...tabs, extra] : tabs;
}

export function moreMenu(entries: readonly MissionEntry[]): { enabled: MissionEntry[]; disabled: MissionEntry[] } {
  const ordered = MORE_ORDER.map((id) => entries.find((e) => e.id === id)).filter((e): e is MissionEntry => !!e && e.placement === "more");
  return { enabled: ordered.filter((e) => e.enabled), disabled: ordered.filter((e) => !e.enabled) };
}

export function viewAvailability(m: MastMission): ViewAvailability {
  const noImages = `MAST's ${m.label} search API serves no thumbnails or previews.`;
  const images = m.capabilities.images;
  return {
    gallery: images && m.views.gallery,
    galleryReason: images && m.views.gallery ? null : noImages,
    previews: images,
    previewsReason: images ? null : noImages,
    products: m.capabilities.products,
    productsReason: m.capabilities.products ? null : `Product lists for ${m.label} arrive with spectra support (MA15)`,
  };
}

export function resultsGate(m: MastMission, roman: RomanLock): ResultsGate {
  if (m.id === "roman") return roman === "unlocked" ? "open" : "locked";
  if (m.availability === "not_deployed" || m.availability === "unsupported" || !m.capabilities.search) return "unavailable";
  if (m.availability === "token") return "locked";
  return "open";
}

export interface MissionProbe {
  stage: "checking" | "done" | "failed";
  at: number;
  error: MastError | null;
}

export type MissionProbes = Partial<Record<MastMissionId, MissionProbe>>;

export interface MoreEntry extends MissionEntry {
  current: boolean;
  check: "idle" | "checking" | null;
  note: string | null;
}

export const NOT_DEPLOYED_STATUS = 404;
export const NOT_DEPLOYED_FOOTER = "AstroBurst will show it once MAST deploys it.";

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

function dateText(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function clockText(at: number): string {
  const d = new Date(at);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function checkedText(at: number, now: number): string {
  const day = dateText(new Date(at));
  return day === dateText(new Date(now)) ? `${clockText(at)} today` : `${day} ${clockText(at)}`;
}

function moreEntry(entry: MissionEntry, mission: MastMission | undefined, probe: MissionProbe | undefined, current: MastMissionId, now: number): MoreEntry {
  const notDeployed = mission?.availability === "not_deployed";
  const check = notDeployed ? (probe?.stage === "checking" ? "checking" : "idle") : null;
  const reason = notDeployed && probe?.stage === "done" ? `not deployed by MAST (HTTP ${NOT_DEPLOYED_STATUS}, checked ${checkedText(probe.at, now)})` : entry.reason;
  const note = probe?.stage === "failed" && probe.error ? `Could not check again (${errorView(probe.error, { surface: "search", hasResults: true }).short})` : null;
  return { ...entry, reason, current: entry.id === current, check, note };
}

export function moreMenuView(
  missions: readonly MastMission[],
  roman: RomanLock,
  probes: MissionProbes,
  current: MastMissionId,
  now: number,
): { enabled: MoreEntry[]; disabled: MoreEntry[] } {
  const { enabled, disabled } = moreMenu(missionEntries(missions, roman));
  const view = (e: MissionEntry) => moreEntry(e, missions.find((m) => m.id === e.id), probes[e.id], current, now);
  return { enabled: enabled.map(view), disabled: disabled.map(view) };
}

export function notDeployedView(input: { label: string; operation?: string; at: number | null; status: number | null; checkedOn: string; now: number }): { title: string; text: string } {
  const when = input.at === null ? input.checkedOn : checkedText(input.at, input.now);
  return {
    title: `MAST does not serve the ${input.label} ${operationName(input.operation)} right now`,
    text: `Checked ${when} (HTTP ${input.status ?? NOT_DEPLOYED_STATUS}). ${NOT_DEPLOYED_FOOTER}`,
  };
}
