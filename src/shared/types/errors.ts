export interface TauriCommandError {
  command: string;
  message: string;
  code?: string;
  context?: Record<string, unknown>;
}

export function isTauriError(err: unknown): err is TauriCommandError {
  return (
    typeof err === "object" &&
    err !== null &&
    "command" in err &&
    "message" in err
  );
}

export function normalizeTauriError(command: string, raw: unknown): TauriCommandError {
  if (typeof raw === "string") {
    return { command, message: raw };
  }
  if (raw instanceof Error) {
    return { command, message: raw.message };
  }
  if (typeof raw === "object" && raw !== null) {
    const obj = raw as Record<string, unknown>;
    return {
      command,
      message: String(obj.message ?? obj.error ?? JSON.stringify(raw)),
      code: typeof obj.code === "string" ? obj.code : undefined,
    };
  }
  return { command, message: String(raw) };
}
