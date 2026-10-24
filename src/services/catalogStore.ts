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
  folder: string;
  free_space_bytes: number | null;
}

export interface CatalogInstallOutcome {
  id: string;
  path: string;
  bytes: number;
  resumed_from: number;
  sha256_hex: string;
  already_installed: boolean;
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
}

export const CATALOG_COMMANDS = [
  "catalog_status_cmd",
  "catalog_install_cmd",
  "catalog_use_file_cmd",
  "catalog_use_siril_dir_cmd",
  "catalog_remove_cmd",
] as const;

export const CATALOG_INSTALL_PROGRESS_EVENT = "catalog-install-progress";

export function catalogStatus(): Promise<CatalogDatasetReport[]> {
  return typedInvoke<CatalogDatasetReport[]>("catalog_status_cmd", {});
}

export function installCatalog(id: string): Promise<CatalogInstallOutcome> {
  return typedInvoke<CatalogInstallOutcome>("catalog_install_cmd", { id });
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
