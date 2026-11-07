export class TauriCommandError extends Error {
  command: string;
  code?: string;
  context?: Record<string, unknown>;
  hint?: string;

  constructor(command: string, message: string, code?: string, context?: Record<string, unknown>, hint?: string) {
    super(message);
    this.name = "TauriCommandError";
    this.command = command;
    this.code = code;
    this.context = context;
    this.hint = hint;
  }
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null) return false;
  const proto = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

export function normalizeTauriError(command: string, raw: unknown): TauriCommandError {
  if (raw instanceof TauriCommandError) {
    return raw;
  }
  if (typeof raw === "string") {
    return new TauriCommandError(command, raw);
  }
  if (raw instanceof Error) {
    return new TauriCommandError(command, raw.message);
  }
  if (typeof raw === "object" && raw !== null) {
    const obj = raw as Record<string, unknown>;
    return new TauriCommandError(
      command,
      String(obj.message ?? obj.error ?? JSON.stringify(raw)),
      typeof obj.code === "string" ? obj.code : undefined,
      isPlainObject(obj.context) ? { ...obj.context } : undefined,
      typeof obj.hint === "string" && obj.hint.trim() ? obj.hint : undefined,
    );
  }
  return new TauriCommandError(command, String(raw));
}
