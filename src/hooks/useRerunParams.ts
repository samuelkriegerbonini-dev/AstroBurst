import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { OpSpec } from "../shared/types/ops";
import { historyRerunStore, type PendingRerun, type RerunSection } from "../utils/historyRerunStore";

export type RerunDecoder<T> = (op: OpSpec) => T | null;

export interface RerunInitial<T> {
  seq: number;
  params: T;
}

export function rerunUpdate<T>(pending: PendingRerun, mountedSeq: number | null, decode: RerunDecoder<T>): T | null {
  return pending.seq === mountedSeq ? null : decode(pending.op);
}

export function useRerunInitial<T>(section: RerunSection, decode: RerunDecoder<T>): RerunInitial<T> | null {
  const [initial] = useState<RerunInitial<T> | null>(() => {
    const pending = historyRerunStore.peek(section);
    const params = pending ? decode(pending.op) : null;
    return pending && params !== null ? { seq: pending.seq, params } : null;
  });
  return initial;
}

export function useRerunUpdates<T>(section: RerunSection, initial: RerunInitial<T> | null, decode: RerunDecoder<T>, apply: (params: T) => void): void {
  const pending = useSyncExternalStore(
    historyRerunStore.subscribe,
    () => historyRerunStore.peek(section),
    () => historyRerunStore.peek(section),
  );
  const decodeRef = useRef(decode);
  decodeRef.current = decode;
  const applyRef = useRef(apply);
  applyRef.current = apply;
  const mountedSeq = initial?.seq ?? null;
  useEffect(() => {
    if (!pending) return;
    const params = rerunUpdate(pending, mountedSeq, decodeRef.current);
    if (params !== null) applyRef.current(params);
    historyRerunStore.consume(pending.seq);
  }, [pending, mountedSeq]);
}
