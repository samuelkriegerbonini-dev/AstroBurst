import { normalizeTauriError, type TauriCommandError } from "../../shared/types/errors";

let _convertFileSrc: ((path: string) => string) | null = null;
let _invoke: ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | null = null;

export const isTauri = (): boolean => !!(window as any).__TAURI_INTERNALS__;

async function ensureConvertFileSrc(): Promise<(path: string) => string> {
  if (_convertFileSrc) return _convertFileSrc;
  const { convertFileSrc } = await import("@tauri-apps/api/core");
  _convertFileSrc = convertFileSrc;
  return convertFileSrc;
}

async function ensureInvoke(): Promise<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>> {
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

export async function typedInvoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri()) {
    throw normalizeTauriError(command, `Command "${command}" requires Tauri desktop environment.`);
  }
  const invoke = await ensureInvoke();
  try {
    return (await invoke(command, args)) as T;
  } catch (err: unknown) {
    throw normalizeTauriError(command, err);
  }
}
