export interface StackRunStart {
  binId: string;
  token: number;
  generation: number;
  files: string[];
}

export interface StackRunOutcome {
  store: boolean;
  notice: string | null;
}

export const STACK_INPUTS_CHANGED = "Inputs changed while stacking; result discarded";

function sameFiles(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((f, i) => f === b[i]);
}

export function stackRunFinish(
  started: StackRunStart,
  current: { generation: number; files: string[] },
): StackRunOutcome {
  if (current.generation !== started.generation || !sameFiles(started.files, current.files)) {
    return { store: false, notice: STACK_INPUTS_CHANGED };
  }
  return { store: true, notice: null };
}

export function stackStateAfterReset(generation: number, running: boolean): { generation: number; cancel: boolean } {
  return { generation: generation + 1, cancel: running };
}

export function nextDiscardNotices(
  notices: Readonly<Record<string, string>>,
  binId: string,
  outcome: StackRunOutcome,
): Record<string, string> {
  const next = { ...notices };
  if (outcome.store) delete next[binId];
  else if (outcome.notice) next[binId] = outcome.notice;
  return next;
}
