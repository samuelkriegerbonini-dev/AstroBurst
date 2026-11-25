export function isCancelledRun(message: string): boolean {
  return /\boperation cancelled\b/i.test(message) || /^task join failed:.*\bcancel/i.test(message.trim());
}
