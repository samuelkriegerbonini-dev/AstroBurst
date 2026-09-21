import { useEffect, useRef, useState } from "react";
import type { Region, RegionStatsEntry } from "../shared/types/regions";
import { regionStats, type RegionStatsRequest } from "../services/regions";

export const STATS_DEBOUNCE_MS = 250;
export const MAX_REGIONS_PER_STATS_CALL = 512;

const EMPTY_STATS: Map<string, RegionStatsEntry> = new Map();

export function chunkStatsRequests(requests: RegionStatsRequest[]): RegionStatsRequest[][] {
  if (requests.length <= MAX_REGIONS_PER_STATS_CALL) return requests.length === 0 ? [] : [requests];
  const chunks: RegionStatsRequest[][] = [];
  for (let start = 0; start < requests.length; start += MAX_REGIONS_PER_STATS_CALL) {
    chunks.push(requests.slice(start, start + MAX_REGIONS_PER_STATS_CALL));
  }
  return chunks;
}

export interface RegionStatsState {
  stats: Map<string, RegionStatsEntry>;
  loading: boolean;
  error: string | null;
}

export function buildStatsRequests(regions: Region[]): RegionStatsRequest[] {
  const byId = new Map(regions.map((r) => [r.id, r]));
  return regions.map((r) => ({
    id: r.id,
    shape: r.shape,
    background: r.backgroundId ? (byId.get(r.backgroundId)?.shape ?? null) : null,
  }));
}

export function useRegionStats(filePath: string | null, regions: Region[], excludeDq: boolean): RegionStatsState {
  const [stats, setStats] = useState<Map<string, RegionStatsEntry>>(EMPTY_STATS);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seqRef = useRef(0);

  useEffect(() => {
    if (!filePath || regions.length === 0) {
      seqRef.current += 1;
      setStats(EMPTY_STATS);
      setLoading(false);
      setError(null);
      return;
    }
    const seq = ++seqRef.current;
    const timer = setTimeout(async () => {
      setLoading(true);
      try {
        const merged = new Map<string, RegionStatsEntry>();
        for (const chunk of chunkStatsRequests(buildStatsRequests(regions))) {
          const res = await regionStats(filePath, chunk, { excludeDq });
          if (seqRef.current !== seq) return;
          for (const entry of res.regions) merged.set(entry.id, entry);
        }
        setStats(merged);
        setError(null);
      } catch (e) {
        if (seqRef.current !== seq) return;
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (seqRef.current === seq) setLoading(false);
      }
    }, STATS_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [filePath, regions, excludeDq]);

  return { stats, loading, error };
}
