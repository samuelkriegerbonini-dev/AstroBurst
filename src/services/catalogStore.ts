import { typedInvoke } from "../infrastructure/tauri";
import { cancelProgress } from "./progress";

export type CatalogKind = "tier0_bundled" | "hosted" | "external" | "cache" | "registered";

export type CatalogStatus =
  | { state: "missing" }
  | { state: "partial"; received: number; total: number | null }
  | { state: "installed"; path: string; verified: boolean }
  | { state: "external"; path: string; valid: boolean; problem: string | null }
  | { state: "cache"; bytes: number; files: number; cap: number }
  | { state: "registered"; bytes: number; files: number; folder: string | null; folder_present: boolean };

export interface CatalogDatasetReport {
  id: string;
  title: string;
  kind: CatalogKind;
  status: CatalogStatus;
  size_bytes: number | null;
  installable: boolean;
  compressed: "none" | "bzip2";
  licence: string;
  credit: string;
  url_host: string | null;
  zenodo_record?: string | null;
  folder: string;
  folder_bytes: number;
  free_space_bytes: number | null;
}

export interface CatalogInstallOutcome {
  id: string;
  path: string;
  bytes: number;
  resumed_from: number;
  sha256_hex: string;
  sha256_verified: boolean;
  already_installed: boolean;
}

export interface ZenodoFile {
  key: string;
  size: number;
  checksum: string | null;
}

export interface ZenodoOffer {
  id: string;
  record_url: string;
  record_page: string | null;
  doi: string | null;
  title: string | null;
  record_licence: string | null;
  gaia_terms: string;
  host: string;
  files: ZenodoFile[];
  archive_key: string;
  archive_url: string;
  archive_bytes: number;
  archive_sha256: string | null;
  decoded_sha256: string | null;
  decoded_max_bytes: number;
  resume_from: number;
  need_bytes: number;
  free_space_bytes: number | null;
  folder: string;
}

export interface ZenodoConsent {
  archive_bytes: number;
  archive_sha256: string | null;
  licence_acknowledged: boolean;
}

export interface CatalogRemoved {
  id: string;
  freed_bytes: number;
  forgot_external: boolean;
}

export interface CatalogChoice {
  path: string;
  id?: string;
  dir?: string;
}

export interface CatalogInstallProgress {
  id: string;
  current: number;
  total: number;
  percent: number;
  stage: string;
  message?: string;
}

export const CATALOG_COMMANDS = [
  "catalog_status_cmd",
  "catalog_install_cmd",
  "catalog_use_file_cmd",
  "catalog_use_siril_dir_cmd",
  "catalog_remove_cmd",
  "catalog_zenodo_offer_cmd",
] as const;

export const CATALOG_INSTALL_PROGRESS_EVENT = "catalog-install-progress";

export function catalogStatus(): Promise<CatalogDatasetReport[]> {
  return typedInvoke<CatalogDatasetReport[]>("catalog_status_cmd", {});
}

export function installCatalog(id: string, consent?: ZenodoConsent): Promise<CatalogInstallOutcome> {
  return typedInvoke<CatalogInstallOutcome>("catalog_install_cmd", consent ? { id, consent } : { id });
}

export function zenodoOffer(): Promise<ZenodoOffer> {
  return typedInvoke<ZenodoOffer>("catalog_zenodo_offer_cmd", {});
}

export function zenodoConsent(offer: ZenodoOffer): ZenodoConsent {
  return { archive_bytes: offer.archive_bytes, archive_sha256: offer.archive_sha256, licence_acknowledged: true };
}

export function chooseCatalogFile(id: string, path: string): Promise<CatalogChoice> {
  return typedInvoke<CatalogChoice>("catalog_use_file_cmd", { id, path });
}

export function chooseSirilCatalogDir(dir: string): Promise<CatalogChoice> {
  return typedInvoke<CatalogChoice>("catalog_use_siril_dir_cmd", { dir });
}

export function removeCatalog(id: string): Promise<CatalogRemoved> {
  return typedInvoke<CatalogRemoved>("catalog_remove_cmd", { id });
}

export function cancelCatalogInstall(id: string): Promise<boolean> {
  return cancelProgress(`${CATALOG_INSTALL_PROGRESS_EVENT}:${id}`);
}

export async function onCatalogInstallProgress(cb: (progress: CatalogInstallProgress) => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<CatalogInstallProgress>(CATALOG_INSTALL_PROGRESS_EVENT, (event) => cb(event.payload));
}
