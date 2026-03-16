import { useState, useCallback } from "react";
import { estimatePsf } from "../services/processing.service";

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

export function usePsfEstimation() {
  const [result, setResult] = useState<PsfResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const estimate = useCallback(async (path: string, config?: PsfConfig) => {
    setLoading(true);
    setError(null);
    try {
      const res = await estimatePsf(path, config) as PsfResult;
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
