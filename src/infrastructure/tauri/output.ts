let _outputDir: string | null = null;
let _outputDirTiles: string | null = null;
let _exportDir: string | null = null;
let _resolving: Promise<string> | null = null;
let _resolvingExport: Promise<string> | null = null;

const FALLBACK = "./output";

async function resolveTauriOutputDir(): Promise<string> {
  try {
    const { appDataDir } = await import("@tauri-apps/api/path");
    const base = await appDataDir();
    return `${base}output`;
  } catch {
    return FALLBACK;
  }
}

async function resolveTauriExportDir(): Promise<string> {
  try {
    const { downloadDir } = await import("@tauri-apps/api/path");
    return await downloadDir();
  } catch {
    return resolveTauriOutputDir();
  }
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
