import { useCallback, useEffect, useRef } from "react";
import { applyStfRender } from "../services/analysis";
import { getOutputDir } from "../infrastructure/tauri";
import { cpuStfPreviewPlan } from "../utils/analysisTarget";
import type { StfParams } from "../shared/types";

export interface CpuStfPreviewInput {
  active: boolean;
  path: string | null;
  stf: StfParams;
  baseStf: StfParams | null;
  histReady: boolean;
  setStfPreviewUrl: (url: string | null) => void;
}

interface StfRenderRequest {
  params: StfParams;
  path: string;
}

const MAX_RENDER_FAILURES = 3;

function sameRequest(a: StfRenderRequest | null, b: StfRenderRequest): boolean {
  return (
    a !== null &&
    a.path === b.path &&
    a.params.shadow === b.params.shadow &&
    a.params.midtone === b.params.midtone &&
    a.params.highlight === b.params.highlight
  );
}

export function useCpuStfPreview({ active, path, stf, baseStf, histReady, setStfPreviewUrl }: CpuStfPreviewInput): void {
  const rafIdRef = useRef<number | null>(null);
  const pendingRef = useRef<StfRenderRequest | null>(null);
  const wantedRef = useRef<StfRenderRequest | null>(null);
  const busyRef = useRef(false);
  const failCountRef = useRef(0);
  const flushRef = useRef<() => void>(() => {});

  const flush = useCallback(async () => {
    const request = pendingRef.current;
    if (busyRef.current || !request) return;
    if (failCountRef.current >= MAX_RENDER_FAILURES) {
      pendingRef.current = null;
      failCountRef.current = 0;
      return;
    }
    pendingRef.current = null;
    if (!sameRequest(wantedRef.current, request)) return;
    busyRef.current = true;
    try {
      const { shadow, midtone, highlight } = request.params;
      const result = await applyStfRender(request.path, await getOutputDir(), shadow, midtone, highlight);
      failCountRef.current = 0;
      if (result.previewUrl && sameRequest(wantedRef.current, request)) {
        setStfPreviewUrl(`${result.previewUrl}${result.previewUrl.includes("?") ? "&" : "?"}t=${Date.now()}`);
      }
    } catch {
      failCountRef.current++;
    } finally {
      busyRef.current = false;
      if (pendingRef.current) queueMicrotask(() => flushRef.current());
    }
  }, [setStfPreviewUrl]);

  useEffect(() => {
    flushRef.current = flush;
  }, [flush]);

  useEffect(
    () => () => {
      if (rafIdRef.current !== null) cancelAnimationFrame(rafIdRef.current);
    },
    [],
  );

  const { shadow, midtone, highlight } = stf;
  const plan = cpuStfPreviewPlan({ active, histReady, stf, baseStf });

  useEffect(() => {
    if (plan !== "render" || !path) {
      wantedRef.current = null;
      if (plan === "base") setStfPreviewUrl(null);
      return;
    }
    const request = { params: { shadow, midtone, highlight }, path };
    wantedRef.current = request;
    pendingRef.current = request;
    if (rafIdRef.current !== null) cancelAnimationFrame(rafIdRef.current);
    rafIdRef.current = requestAnimationFrame(() => {
      rafIdRef.current = null;
      flushRef.current();
    });
  }, [plan, path, shadow, midtone, highlight, setStfPreviewUrl]);
}
