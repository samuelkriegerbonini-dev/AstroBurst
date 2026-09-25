import { useEffect, useRef, useState } from "react";
import type { Region, RegionStatsEntry, RegionStatsResult } from "../shared/types/regions";
import type { PhotCal } from "../services/analysis";
import { regionStats, type RegionStatsRequest } from "../services/regions";
import type { RegionStatsMeasured } from "../utils/measurementLog";

export const STATS_DEBOUNCE_MS = 250;
export const MAX_REGIONS_PER_STATS_CALL = 512;
export const MAX_CLIP_ITERS = 100;

const EMPTY_STATS: Map<string, RegionStatsEntry> = new Map();

export function chunkStatsRequests(requests: RegionStatsRequest[]): RegionStatsRequest[][] {
  if (requests.length <= MAX_REGIONS_PER_STATS_CALL) return requests.length === 0 ? [] : [requests];
  const chunks: RegionStatsRequest[][] = [];
  for (let start = 0; start < requests.length; start += MAX_REGIONS_PER_STATS_CALL) {
    chunks.push(requests.slice(start, start + MAX_REGIONS_PER_STATS_CALL));
  }
  return chunks;
}

export interface RegionStatsClip {
  sigma: number | null;
  maxiters: number | null;
}

export function parseClipIters(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const v = Number(trimmed);
  return Number.isInteger(v) && v >= 1 && v <= MAX_CLIP_ITERS ? v : null;
}

export function parseClipSigma(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const v = Number(trimmed);
  const asF32 = Math.fround(v);
  return Number.isFinite(asF32) && asF32 > 0 ? v : null;
}

export interface RegionCalibrationState {
  photcal: PhotCal | null;
  calibrationWarnings: string[];
  pixelAreaArcsec2: number | null;
}

export interface RegionStatsState extends RegionCalibrationState {
  stats: Map<string, RegionStatsEntry>;
  loading: boolean;
  error: string | null;
  measured: RegionStatsMeasured | null;
}

const NO_CALIBRATION: RegionCalibrationState = { photcal: null, calibrationWarnings: [], pixelAreaArcsec2: null };

export function calibrationOf(res: RegionStatsResult): RegionCalibrationState {
  return {
    photcal: res.photcal ?? null,
    calibrationWarnings: res.calibration_warnings ?? [],
    pixelAreaArcsec2: res.pixel_area_arcsec2 ?? null,
  };
}

export function measuredOfChunks(responses: readonly RegionStatsResult[]): { masked: boolean; dqExcluded: number | null; elapsedMs: number } {
  return {
    masked: responses.some((r) => r.masked),
    dqExcluded: responses.find((r) => typeof r.dq_excluded === "number")?.dq_excluded ?? null,
    elapsedMs: responses.reduce((sum, r) => sum + r.elapsed_ms, 0),
  };
}

export function buildStatsRequests(regions: Region[]): RegionStatsRequest[] {
  const byId = new Map(regions.map((r) => [r.id, r]));
  return regions.map((r) => ({
    id: r.id,
    shape: r.shape,
    background: r.backgroundId ? (byId.get(r.backgroundId)?.shape ?? null) : null,
  }));
}

export function useRegionStats(
  filePath: string | null,
  regions: Region[],
  excludeDq: boolean,
  clip?: RegionStatsClip,
): RegionStatsState {
  const [stats, setStats] = useState<Map<string, RegionStatsEntry>>(EMPTY_STATS);
  const [calibration, setCalibration] = useState<RegionCalibrationState>(NO_CALIBRATION);
  const [measured, setMeasured] = useState<RegionStatsMeasured | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seqRef = useRef(0);
  const sigma = clip?.sigma ?? null;
  const maxiters = clip?.maxiters ?? null;

  useEffect(() => {
    setStats(EMPTY_STATS);
    setCalibration(NO_CALIBRATION);
    setMeasured(null);
  }, [filePath]);

  useEffect(() => {
    if (!filePath || regions.length === 0) {
      seqRef.current += 1;
      setStats(EMPTY_STATS);
      setCalibration(NO_CALIBRATION);
      setMeasured(null);
      setLoading(false);
      setError(null);
      return;
    }
    const seq = ++seqRef.current;
    const timer = setTimeout(async () => {
      setLoading(true);
      try {
        const merged = new Map<string, RegionStatsEntry>();
        let last: RegionCalibrationState = NO_CALIBRATION;
        const responses: RegionStatsResult[] = [];
        for (const chunk of chunkStatsRequests(buildStatsRequests(regions))) {
          const res = await regionStats(filePath, chunk, {
            excludeDq,
            sigma: sigma ?? undefined,
            maxiters: maxiters ?? undefined,
          });
          if (seqRef.current !== seq) return;
          responses.push(res);
          for (const entry of res.regions) merged.set(entry.id, entry);
          last = calibrationOf(res);
        }
        setStats(merged);
        setCalibration(last);
        setMeasured({ regions, excludeDq, sigma, maxiters, ...measuredOfChunks(responses) });
        setError(null);
      } catch (e) {
        if (seqRef.current !== seq) return;
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (seqRef.current === seq) setLoading(false);
      }
    }, STATS_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [filePath, regions, excludeDq, sigma, maxiters]);

  return { stats, loading, error, measured, ...calibration };
}
