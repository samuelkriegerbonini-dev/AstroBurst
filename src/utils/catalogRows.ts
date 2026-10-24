import type {
  CatalogDatasetReport,
  CatalogInstallOutcome,
  CatalogInstallProgress,
  CatalogKind,
  CatalogRemoved,
} from "../services/catalogStore";
import { formatDecimalBytes } from "./format";

export type CatalogTier = 0 | 1 | 2;

export type InstallPhase =
  | { phase: "idle"; note?: string; returnFocus?: boolean }
  | { phase: "consent" }
  | { phase: "running"; stage: string; current: number; total: number; percent: number; cancelling: boolean; adopted?: boolean }
  | { phase: "failed"; message: string };

export type InstallAction =
  | { type: "ask"; id: string }
  | { type: "decline"; id: string }
  | { type: "start"; id: string }
  | { type: "progress"; progress: CatalogInstallProgress }
  | { type: "cancel"; id: string }
  | { type: "finished"; id: string; note: string }
  | { type: "failed"; id: string; message: string };

export type InstallMap = Readonly<Record<string, InstallPhase>>;

export const IDLE_INSTALL: InstallPhase = { phase: "idle" };

export const TIER2_ONLINE_TEXT =
  "Gaia DR3 online through VizieR, used when no offline tier is enough: only the star list of the field is downloaded and kept in a disk cache; the image never leaves this computer.";

export const INSTALL_COMPLETE_STAGE = "complete";

const CANCELLED_NOTE = "Cancelled; what was downloaded is kept and the next download resumes from there.";
const NO_VALUE = "--";
const VERSION_SUFFIX = /-(v\d+)$/;
const SIRIL_FILE = "siril_cat_healpix8_astro.dat";

const TIER_OF_KIND: Readonly<Record<CatalogKind, CatalogTier | null>> = {
  tier0_bundled: 0,
  hosted: 1,
  external: 1,
  cache: 2,
  registered: null,
};

const KIND_ORDER: readonly CatalogKind[] = ["tier0_bundled", "hosted", "external", "cache", "registered"];

const REMOVE_LABELS: Readonly<Record<CatalogKind, string>> = {
  tier0_bundled: "Remove",
  hosted: "Remove",
  external: "Forget",
  cache: "Clear",
  registered: "Remove",
};

export function catalogVersion(id: string): string | null {
  return VERSION_SUFFIX.exec(id)?.[1] ?? null;
}

function bytesText(bytes: number | null | undefined): string {
  return typeof bytes === "number" && Number.isFinite(bytes) && bytes >= 0 ? formatDecimalBytes(bytes) : NO_VALUE;
}

function megabytes(bytes: number): string {
  return (bytes / 1e6).toFixed(1);
}

export interface CatalogRowActions {
  install: boolean;
  cancel: boolean;
  remove: boolean;
  chooseFolder: boolean;
  chooseFile: boolean;
}

export type CatalogTone = "ok" | "muted" | "warn" | "error";

export interface CatalogRowView {
  id: string;
  tier: CatalogTier | null;
  tierLabel: string;
  title: string;
  version: string | null;
  sizeText: string;
  stateText: string;
  tone: CatalogTone;
  percent: number | null;
  installLabel: string;
  removeLabel: string;
  actions: CatalogRowActions;
  licence: string;
  credit: string;
}

const NO_ACTIONS: CatalogRowActions = { install: false, cancel: false, remove: false, chooseFolder: false, chooseFile: false };

interface StateView {
  text: string;
  tone: CatalogTone;
  actions: Partial<CatalogRowActions>;
  resume?: boolean;
}

function runningView(phase: Extract<InstallPhase, { phase: "running" }>): StateView {
  const verb = phase.stage === "decode" ? "Decoding" : "Downloading";
  const amount = phase.total > 0 ? ` (${megabytes(phase.current)} of ${megabytes(phase.total)} MB)` : "";
  const text = phase.cancelling ? "Cancelling..." : `${verb} ${phase.percent} %${amount}`;
  return { text, tone: "muted", actions: { cancel: !phase.cancelling } };
}

function bundledView(report: CatalogDatasetReport): StateView {
  if (report.status.state !== "installed") {
    return { text: "Not in this build: solving offline needs tier 1 or a Siril extract", tone: "warn", actions: {} };
  }
  return report.status.verified
    ? { text: "Bundled with AstroBurst, SHA-256 verified", tone: "ok", actions: {} }
    : { text: "Bundled with AstroBurst; its SHA-256 is checked on first use", tone: "ok", actions: {} };
}

function downloadView(report: CatalogDatasetReport): StateView {
  const status = report.status;
  if (status.state === "installed") {
    return { text: status.verified ? "Installed, SHA-256 verified" : "Installed; its SHA-256 is checked on first use", tone: "ok", actions: { remove: true } };
  }
  if (status.state === "partial") {
    const total = status.total ?? report.size_bytes;
    const amount = total ? `${megabytes(status.received)} of ${megabytes(total)} MB` : `${megabytes(status.received)} MB`;
    return { text: `Partial download: ${amount}; Download resumes it`, tone: "muted", actions: { install: report.installable, remove: true }, resume: true };
  }
  if (!report.installable && report.kind === "hosted") {
    return { text: "Not published yet: the download opens with the catalogs-v1 release", tone: "muted", actions: {} };
  }
  return { text: "Not installed", tone: "muted", actions: { install: report.installable } };
}

function externalView(report: CatalogDatasetReport): StateView {
  const status = report.status;
  const choose = { chooseFolder: true, chooseFile: true };
  if (status.state === "external") {
    return status.valid
      ? { text: `Using ${status.path}`, tone: "ok", actions: { ...choose, remove: true } }
      : { text: `Cannot use ${status.path}: ${status.problem ?? "the file is not a usable catalogue"}`, tone: "error", actions: { ...choose, remove: true } };
  }
  if (status.state === "installed" || status.state === "partial") {
    const view = downloadView(report);
    return { ...view, actions: { ...view.actions, ...choose } };
  }
  return {
    text: `Not set: choose your Siril catalogue folder or its ${SIRIL_FILE} file`,
    tone: "muted",
    actions: { ...choose, install: report.installable },
  };
}

function cacheView(report: CatalogDatasetReport): StateView {
  const status = report.status;
  if (status.state !== "cache") return { text: NO_VALUE, tone: "muted", actions: {} };
  return {
    text: `Cache ${bytesText(status.bytes)} in ${status.files} file${status.files === 1 ? "" : "s"} (cap ${bytesText(status.cap)})`,
    tone: "muted",
    actions: { remove: status.bytes > 0 || status.files > 0 },
  };
}

function stateView(report: CatalogDatasetReport): StateView {
  switch (report.kind) {
    case "tier0_bundled":
      return bundledView(report);
    case "external":
      return externalView(report);
    case "cache":
      return cacheView(report);
    default:
      return downloadView(report);
  }
}

export function catalogRowView(report: CatalogDatasetReport, install: InstallPhase = IDLE_INSTALL): CatalogRowView {
  const tier = TIER_OF_KIND[report.kind] ?? null;
  const view = install.phase === "running" ? runningView(install) : stateView(report);
  return {
    id: report.id,
    tier,
    tierLabel: tier === null ? "Catalogue" : `Tier ${tier}`,
    title: report.title,
    version: catalogVersion(report.id),
    sizeText: report.kind === "cache" ? NO_VALUE : bytesText(report.size_bytes),
    stateText: view.text,
    tone: view.tone,
    percent: install.phase === "running" ? install.percent : null,
    installLabel: view.resume ? "Resume" : "Download",
    removeLabel: REMOVE_LABELS[report.kind],
    actions: { ...NO_ACTIONS, ...view.actions },
    licence: report.licence,
    credit: report.credit,
  };
}

export function consentText(report: CatalogDatasetReport): string {
  const host = report.url_host ? `from ${report.url_host}` : "from an unknown host";
  const size = report.size_bytes === null ? "size unknown" : bytesText(report.size_bytes);
  const decoded = report.compressed === "bzip2" ? " (bzip2, unpacked after the download)" : "";
  const free = report.free_space_bytes === null ? "unknown" : bytesText(report.free_space_bytes);
  return (
    `Download ${report.title} ${host}: ${size}${decoded}. Licence: ${report.licence}. Credit: ${report.credit}. ` +
    `Free space: ${free} in ${report.folder}. Only the catalogue is downloaded; no image leaves this computer.`
  );
}

export function consentBlocker(report: CatalogDatasetReport): string | null {
  if (report.size_bytes === null || report.free_space_bytes === null) return null;
  if (report.free_space_bytes >= report.size_bytes) return null;
  return `Needs ${bytesText(report.size_bytes)} free; ${bytesText(report.free_space_bytes)} available`;
}

function set(installs: InstallMap, id: string, phase: InstallPhase): InstallMap {
  return { ...installs, [id]: phase };
}

export function installReducer(installs: InstallMap, action: InstallAction): InstallMap {
  const id = action.type === "progress" ? action.progress.id : action.id;
  const current = installs[id] ?? IDLE_INSTALL;
  switch (action.type) {
    case "ask":
      return current.phase === "idle" || current.phase === "failed" ? set(installs, id, { phase: "consent" }) : installs;
    case "decline":
      return current.phase === "consent" ? set(installs, id, { phase: "idle", returnFocus: true }) : installs;
    case "start":
      return current.phase === "consent"
        ? set(installs, id, { phase: "running", stage: "download", current: 0, total: 0, percent: 0, cancelling: false })
        : installs;
    case "progress": {
      const { current: done, total, percent, stage } = action.progress;
      if (stage === INSTALL_COMPLETE_STAGE) return current.phase === "running" ? set(installs, id, IDLE_INSTALL) : installs;
      if (current.phase === "running") return set(installs, id, { ...current, current: done, total, percent, stage });
      if (current.phase === "consent") return installs;
      return set(installs, id, { phase: "running", stage, current: done, total, percent, cancelling: false, adopted: true });
    }
    case "cancel":
      return current.phase === "running" ? set(installs, id, { ...current, cancelling: true }) : installs;
    case "finished":
      return set(installs, id, { phase: "idle", note: action.note });
    case "failed":
      return current.phase === "running" && current.cancelling
        ? set(installs, id, { phase: "idle", note: CANCELLED_NOTE })
        : set(installs, id, { phase: "failed", message: action.message });
  }
}

function removableBytes(report: CatalogDatasetReport): number | null {
  const status = report.status;
  switch (status.state) {
    case "partial":
      return status.received;
    case "cache":
    case "registered":
      return status.bytes;
    default:
      return report.size_bytes;
  }
}

export function removeConfirmText(report: CatalogDatasetReport): string | null {
  if (report.status.state === "external") return null;
  const verb = report.kind === "cache" ? "clears" : "deletes";
  const bytes = removableBytes(report);
  const what = typeof bytes === "number" && Number.isFinite(bytes) && bytes >= 0 ? bytesText(bytes) : "the downloaded files";
  return `Confirm: ${verb} ${what}`;
}

export function sortedCatalogRows(reports: readonly CatalogDatasetReport[]): CatalogDatasetReport[] {
  return [...reports].sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind));
}

export function installedNote(outcome: CatalogInstallOutcome): string {
  if (outcome.already_installed) return "Already installed and verified.";
  const resumed = outcome.resumed_from > 0 ? `; resumed after ${bytesText(outcome.resumed_from)}` : "";
  return `Installed ${bytesText(outcome.bytes)}, SHA-256 verified${resumed}.`;
}

export function removedNote(removed: CatalogRemoved): string {
  if (removed.forgot_external) return "Forgotten; the file itself is kept.";
  return removed.freed_bytes > 0 ? `Removed ${bytesText(removed.freed_bytes)}.` : "Nothing to remove.";
}
