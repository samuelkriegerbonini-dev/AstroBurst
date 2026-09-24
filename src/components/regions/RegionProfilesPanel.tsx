import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Activity, Loader2 } from "lucide-react";
import type { RadialProfile, LineCut, RegionShape } from "../../shared/types";
import { radialProfile, lineCut } from "../../services/regions";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useDqContext } from "../../context/PreviewContext";
import { shapeSummary } from "../../utils/regionGeometry";
import ProfilePlot, { type ProfileSeries } from "./ProfilePlot";
import MeasurementBadge from "../analysis/MeasurementBadge";

interface RegionProfilesPanelProps {
  filePath: string | null;
  measurePath: string | null;
}

const PROFILE_DEBOUNCE_MS = 250;
const MEAN_COLOR = "#7dd3fc";
const MEDIAN_COLOR = "#fbbf24";
const CUT_COLOR = "#a78bfa";

type ProfileRequest =
  | { kind: "radial"; x: number; y: number; maxRadius: number; background: [number, number] | null }
  | { kind: "cut"; x1: number; y1: number; x2: number; y2: number };

type ProfileResult = { kind: "radial"; data: RadialProfile } | { kind: "cut"; data: LineCut };

function requestFor(shape: RegionShape | undefined, background: RegionShape | undefined): ProfileRequest | null {
  if (!shape) return null;
  const bg: [number, number] | null =
    background && background.shape === "annulus" ? [background.r_inner, background.r_outer] : null;
  if (shape.shape === "circle") return { kind: "radial", x: shape.x, y: shape.y, maxRadius: shape.r, background: bg };
  if (shape.shape === "annulus") {
    return { kind: "radial", x: shape.x, y: shape.y, maxRadius: shape.r_outer, background: bg };
  }
  if (shape.shape === "line") return { kind: "cut", x1: shape.x1, y1: shape.y1, x2: shape.x2, y2: shape.y2 };
  return null;
}

function RegionProfilesPanel({ filePath, measurePath }: RegionProfilesPanelProps) {
  const doc = useRegionDoc(filePath);
  const { excludeDq } = useDqContext();
  const [result, setResult] = useState<ProfileResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seqRef = useRef(0);

  const selected = doc.regions.find((r) => r.id === doc.selectedId);
  const background = selected?.backgroundId ? doc.regions.find((r) => r.id === selected.backgroundId) : undefined;
  const request = useMemo(() => requestFor(selected?.shape, background?.shape), [selected?.shape, background?.shape]);
  const requestKey = request ? JSON.stringify(request) : null;

  useEffect(() => {
    setResult(null);
  }, [measurePath]);

  useEffect(() => {
    const seq = ++seqRef.current;
    if (!measurePath || !requestKey) {
      setResult(null);
      setError(null);
      setLoading(false);
      return;
    }
    const req = JSON.parse(requestKey) as ProfileRequest;
    const timer = setTimeout(async () => {
      setLoading(true);
      try {
        const res: ProfileResult =
          req.kind === "radial"
            ? {
                kind: "radial",
                data: await radialProfile(measurePath, req.x, req.y, req.maxRadius, { background: req.background, excludeDq }),
              }
            : { kind: "cut", data: await lineCut(measurePath, req.x1, req.y1, req.x2, req.y2, excludeDq) };
        if (seqRef.current !== seq) return;
        setResult(res);
        setError(null);
      } catch (e) {
        if (seqRef.current !== seq) return;
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (seqRef.current === seq) setLoading(false);
      }
    }, PROFILE_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [measurePath, requestKey, excludeDq]);

  const series = useMemo<ProfileSeries[]>(() => {
    if (!result) return [];
    if (result.kind === "radial") {
      const bins = result.data.bins;
      const x = bins.map((b) => b.r + 0.5);
      return [
        { x, y: bins.map((b) => b.mean), color: MEAN_COLOR, label: "mean" },
        { x, y: bins.map((b) => b.median), color: MEDIAN_COLOR, label: "median" },
      ];
    }
    return [{ x: result.data.distance, y: result.data.values, color: CUT_COLOR, label: "value" }];
  }, [result]);

  if (!selected || !request) return null;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Activity size={12} className="text-sky-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            {request.kind === "radial" ? "Radial profile" : "Line cut"}
          </span>
          <MeasurementBadge />
        </div>
        <div className="flex items-center gap-2">
          {result?.data.masked && (
            <span className="text-[9px] px-1.5 py-0.5 rounded text-amber-300 bg-amber-900/30">DQ masked</span>
          )}
          {loading && <Loader2 size={12} className="animate-spin text-sky-400/70" />}
        </div>
      </div>
      <div className="px-3 py-2 space-y-2">
        <div className="text-[10px] font-mono text-zinc-500 truncate">{shapeSummary(selected.shape)}</div>
        {error && (
          <div className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">
            {error}
          </div>
        )}
        {result && (
          <ProfilePlot
            series={series}
            xLabel={request.kind === "radial" ? "radius (px)" : "distance (px)"}
            yLabel={result.kind === "radial" && result.data.background ? "value − bg" : "value"}
          />
        )}
        {result?.kind === "radial" && (
          <div className="flex flex-wrap gap-x-3 text-[9px] font-mono text-zinc-500">
            <span>{result.data.bins.length} bins</span>
            {result.data.background && (
              <span>
                bg {result.data.background.median.toExponential(3)} ± {result.data.background.sigma.toExponential(2)}
              </span>
            )}
            {result.data.bins.length > 0 && (
              <span>cum {result.data.bins[result.data.bins.length - 1].cumulative_sum.toExponential(3)}</span>
            )}
          </div>
        )}
        {result?.kind === "cut" && (
          <div className="flex flex-wrap gap-x-3 text-[9px] font-mono text-zinc-500">
            <span>length {result.data.length.toFixed(1)} px</span>
            <span>{result.data.n_samples} samples</span>
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(RegionProfilesPanel);
