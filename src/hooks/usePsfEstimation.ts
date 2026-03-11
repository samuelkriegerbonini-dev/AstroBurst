import { useState, useCallback } from "react";

interface StarCandidate {
  x: number;
  y: number;
  peak: number;
  flux: number;
  fwhm: number;
  ellipticity: number;
  snr: number;
}

interface PsfResult {
  kernel: number[][];
  kernel_size: number;
  average_fwhm: number;
  average_ellipticity: number;
  stars_used: StarCandidate[];
  stars_rejected: number;
  spread_pixels: number;
}

interface PsfConfig {
  numStars?: number;
  cutoutRadius?: number;
  saturationThreshold?: number;
  maxEllipticity?: number;
}

const safeInvoke = async (cmd: string, args: Record<string, any> = {}) => {
  if (!(window as any).__TAURI_INTERNALS__) throw new Error("Requires Tauri");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke(cmd, args);
};

export function usePsfEstimation() {
  const [result, setResult] = useState<PsfResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const estimate = useCallback(async (path: string, config?: PsfConfig) => {
    setLoading(true);
    setError(null);
    try {
      const res = await safeInvoke("estimate_psf_cmd", {
        path,
        numStars: config?.numStars ?? 3,
        cutoutRadius: config?.cutoutRadius ?? 15,
        saturationThreshold: config?.saturationThreshold ?? 0.95,
        maxEllipticity: config?.maxEllipticity ?? 0.3,
      }) as PsfResult;
      setResult(res);
      return res;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      return null;
    } finally {
      setLoading(false);
    }
  }, []);

  return { result, loading, error, estimate };
}

export type { PsfResult, PsfConfig, StarCandidate };
