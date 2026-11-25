export function sameRequest(stored: unknown, current: unknown): boolean {
  return current !== null && current !== undefined && JSON.stringify(stored) === JSON.stringify(current);
}
