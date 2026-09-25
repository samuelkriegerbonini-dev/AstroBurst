export interface RunTracker {
  seq: number;
  inFlight: number | null;
}

export function createRunTracker(): RunTracker {
  return { seq: 0, inFlight: null };
}

export function beginRun(t: RunTracker): number {
  t.seq += 1;
  t.inFlight = t.seq;
  return t.seq;
}

export function isCurrentRun(t: RunTracker, seq: number): boolean {
  return t.seq === seq;
}

export function settleRun(t: RunTracker, seq: number): void {
  if (t.inFlight === seq) t.inFlight = null;
}

export function abandonRun(t: RunTracker): boolean {
  t.seq += 1;
  const orphaned = t.inFlight !== null;
  t.inFlight = null;
  return orphaned;
}
