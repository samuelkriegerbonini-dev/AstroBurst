let _outputDir: string | null = null;
let _outputDirTiles: string | null = null;
let _exportDir: string | null = null;
let _historyDir: string | null = null;
let _resolving: Promise<string> | null = null;
let _resolvingExport: Promise<string> | null = null;

const OUTPUT_DIR_NAME = "output";
const HISTORY_DIR_NAME = "history";
const FALLBACK = `./${OUTPUT_DIR_NAME}`;
const HISTORY_FALLBACK = `./${HISTORY_DIR_NAME}`;
const EXPORT_FALLBACK = ".";

async function resolveAppDataChild(name: string, fallback: string): Promise<string> {
  try {
    const { appDataDir, join } = await import("@tauri-apps/api/path");
    return await join(await appDataDir(), name);
  } catch {
    return fallback;
  }
}

function resolveTauriOutputDir(): Promise<string> {
  return resolveAppDataChild(OUTPUT_DIR_NAME, FALLBACK);
}

export async function firstResolvedDir(
  resolvers: readonly (() => Promise<string>)[],
  fallback: string,
): Promise<string> {
  for (const resolve of resolvers) {
    try {
      const dir = await resolve();
      if (dir) return dir;
    } catch {
      continue;
    }
  }
  return fallback;
}

async function resolveTauriExportDir(): Promise<string> {
  const path = await import("@tauri-apps/api/path").catch(() => null);
  if (!path) return EXPORT_FALLBACK;
  return firstResolvedDir([path.downloadDir, path.homeDir, path.documentDir, path.desktopDir], EXPORT_FALLBACK);
}

export async function getOutputDir(): Promise<string> {
  if (_outputDir) return _outputDir;
  if (!_resolving) {
    _resolving = resolveTauriOutputDir().then((dir) => {
      _outputDir = dir;
      _resolving = null;
      return dir;
    });
  }
  return _resolving;
}

export async function getExportDir(): Promise<string> {
  if (_exportDir) return _exportDir;
  if (!_resolvingExport) {
    _resolvingExport = resolveTauriExportDir().then((dir) => {
      _exportDir = dir;
      _resolvingExport = null;
      return dir;
    });
  }
  return _resolvingExport;
}

export async function getOutputDirTiles(): Promise<string> {
  if (_outputDirTiles) return _outputDirTiles;
  const base = await getOutputDir();
  _outputDirTiles = `${base}/tiles`;
  return _outputDirTiles;
}

export async function getHistoryDir(): Promise<string> {
  if (_historyDir) return _historyDir;
  _historyDir = await resolveAppDataChild(HISTORY_DIR_NAME, HISTORY_FALLBACK);
  return _historyDir;
}
