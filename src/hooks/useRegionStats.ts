import { useEffect, useRef, useState } from "react";
import type { Region, RegionStatsEntry } from "../shared/types/regions";
import { regionStats, type RegionStatsRequest } from "../services/regions";

export const STATS_DEBOUNCE_MS = 250;

const EMPTY_STATS: Map<string, RegionStatsEntry> = new Map();

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
        const res = await regionStats(filePath, buildStatsRequests(regions), { excludeDq });
        if (seqRef.current !== seq) return;
        setStats(new Map(res.regions.map((e) => [e.id, e])));
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
