import type { HistoryScope, OpName, OpSpec } from "../shared/types/ops";

export type RerunSection =
  | "background"
  | "denoise"
  | "deconvolution"
  | "stretch"
  | "masked_stretch"
  | "local_contrast"
  | "hdr"
  | "curves"
  | "pixelmath";

export interface RerunRequest {
  section: RerunSection;
  op: OpSpec;
  node: string;
  downstream: boolean;
  scope: HistoryScope;
}

export interface PendingRerun extends RerunRequest {
  seq: number;
}

const SECTIONS: Record<OpName, RerunSection | null> = {
  background: "background",
  wavelet_denoise: "denoise",
  rl_deconv: "deconvolution",
  arcsinh: "stretch",
  ghs: null,
  masked_stretch: "masked_stretch",
  lhe: "local_contrast",
  hdrmt: "hdr",
  tone: "curves",
  pixelmath: "pixelmath",
};

export function sectionForOp(op: OpSpec): RerunSection | null {
  return SECTIONS[op.op];
}

let pending: PendingRerun | null = null;
let armed: PendingRerun | null = null;
let seq = 0;
const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of listeners) listener();
}

function sameScope(a: HistoryScope, b: HistoryScope): boolean {
  return a.imageKey === b.imageKey && a.basePath === b.basePath;
}

export const historyRerunStore = {
  request(request: RerunRequest): PendingRerun {
    seq += 1;
    pending = { ...request, seq };
    armed = request.downstream ? pending : null;
    notify();
    return pending;
  },
  get: (): PendingRerun | null => pending,
  peek(section: RerunSection): PendingRerun | null {
    return pending?.section === section ? pending : null;
  },
  consume(at: number): void {
    if (pending?.seq !== at) return;
    pending = null;
    notify();
  },
  armed: (): PendingRerun | null => armed,
  takeArmed(scope: HistoryScope, op: OpName): PendingRerun | null {
    if (!armed || armed.op.op !== op || !sameScope(armed.scope, scope)) return null;
    const taken = armed;
    armed = null;
    notify();
    return taken;
  },
  disarm(): void {
    if (!armed) return;
    armed = null;
    notify();
  },
  clear(): void {
    pending = null;
    armed = null;
    notify();
  },
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },
};
