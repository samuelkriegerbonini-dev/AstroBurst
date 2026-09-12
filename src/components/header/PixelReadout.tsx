import { useState, useEffect, memo } from "react";
import { Crosshair } from "lucide-react";
import { probePixel } from "../../services/display";
import type { PixelProbeResult } from "../../shared/types/analysis";

interface PixelReadoutProps {
  filePath: string | null;
  mouseX: number | null;
  mouseY: number | null;
}

const HOVER_DEBOUNCE_MS = 40;
const BOX_SIZE = 5;

function fmt(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return "—";
  const abs = Math.abs(v);
  if (abs !== 0 && (abs < 1e-3 || abs >= 1e6)) return v.toExponential(3);
  if (Number.isInteger(v)) return String(v);
  return v.toFixed(abs >= 100 ? 2 : 4);
}

function PixelReadoutInner({ filePath, mouseX, mouseY }: PixelReadoutProps) {
  const [probe, setProbe] = useState<PixelProbeResult | null>(null);

  useEffect(() => {
    if (!filePath || mouseX === null || mouseY === null) {
      setProbe(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      probePixel(filePath, mouseX, mouseY, BOX_SIZE)
        .then((res) => {
          if (cancelled) return;
          setProbe(res);
        })
        .catch(() => {
          if (cancelled) return;
          setProbe(null);
        });
    }, HOVER_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [filePath, mouseX, mouseY]);

  if (!probe) return null;

  const n = probe.neighborhood;
  const err = probe.err && probe.err.value !== null ? probe.err : null;
  return (
    <div className="flex flex-col gap-0.5 text-[10px] font-mono" style={{ color: "rgba(251,191,36,0.7)" }}>
      <div className="flex items-center gap-2 flex-wrap">
        <Crosshair size={10} />
        <span>
          {fmt(probe.value)}
          {probe.unit ? ` ${probe.unit}` : ""}
          {err ? ` ± ${fmt(err.value)}${err.unit ? ` ${err.unit}` : ""}` : ""}
        </span>
        <span className="text-zinc-600">
          px({probe.x},{probe.y})
        </span>
      </div>
      {probe.dq && (
        <div className="flex items-center gap-2 flex-wrap" style={{ color: "rgba(248,113,113,0.8)" }} title={probe.dq.table}>
          <span>DQ = {probe.dq.text}</span>
        </div>
      )}
      <div className="flex items-center gap-2 flex-wrap text-zinc-500">
        <span>box {probe.box}×{probe.box}:</span>
        <span>min {fmt(n.min)}</span>
        <span>max {fmt(n.max)}</span>
        <span>mean {fmt(n.mean)}</span>
        <span>median {fmt(n.median)}</span>
        <span className="text-zinc-600">({n.n_pixels}, nan {n.n_nan})</span>
      </div>
    </div>
  );
}

export default memo(PixelReadoutInner);
