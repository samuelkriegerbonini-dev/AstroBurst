let _convertFileSrc: ((path: string) => string) | null = null;
let _invoke: ((cmd: string, args?: Record<string, any>) => Promise<any>) | null = null;

export const isTauri = (): boolean => !!(window as any).__TAURI_INTERNALS__;

async function ensureConvertFileSrc(): Promise<(path: string) => string> {
  if (_convertFileSrc) return _convertFileSrc;
  const { convertFileSrc } = await import("@tauri-apps/api/core");
  _convertFileSrc = convertFileSrc;
  return convertFileSrc;
}

async function ensureInvoke(): Promise<(cmd: string, args?: Record<string, any>) => Promise<any>> {
  if (_invoke) return _invoke;
  const { invoke } = await import("@tauri-apps/api/core");
  _invoke = invoke;
  return invoke;
}

export async function getPreviewUrl(path: string): Promise<string> {
  if (!path) return "";
  if (isTauri()) {
    const convert = await ensureConvertFileSrc();
    const cleanPath = path.startsWith("\\\\?\\") ? path.slice(4) : path;
    return convert(cleanPath);
  }
  return path;
}

export const safeInvoke = async (command: string, args: Record<string, any> = {}): Promise<any> => {
  if (isTauri()) {
    const invoke = await ensureInvoke();
    return invoke(command, args);
  }
  throw new Error(`Command "${command}" requires Tauri desktop environment.`);
};
