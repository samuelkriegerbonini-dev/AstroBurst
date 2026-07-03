import { useEffect, useMemo, useState } from "react";
import { computeHistogram } from "../../services/analysis";

interface ChannelSpec {
  key: string;
  color: string;
}

interface StfHistogramProps {
  channels: ChannelSpec[];
  shadow: number;
  midtone: number;
  highlight: number;
  height?: number;
}

interface Curve {
  color: string;
  points: string;
}

export default function StfHistogram({ channels, shadow, midtone, highlight, height = 44 }: StfHistogramProps) {
  const [curves, setCurves] = useState<Curve[] | null>(null);
  const channelSig = useMemo(() => channels.map((c) => c.key).join("|"), [channels]);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const results = await Promise.all(channels.map((c) => computeHistogram(c.key)));
        if (!alive) return;
        const built = results.map((res, i) => {
          const bins = res.bins ?? [];
          const n = bins.length;
          if (n < 2) return { color: channels[i].color, points: "" };
          const maxLog = Math.log1p(Math.max(1, ...bins));
          const points = bins
            .map((v, j) => {
              const x = (j / (n - 1)) * 100;
              const y = 100 - (Math.log1p(Math.max(0, v)) / maxLog) * 96;
              return `${x.toFixed(2)},${y.toFixed(2)}`;
            })
            .join(" ");
          return { color: channels[i].color, points };
        });
        setCurves(built.filter((c) => c.points.length > 0));
      } catch {
        if (alive) setCurves([]);
      }
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [channelSig]);

  if (!curves || curves.length === 0) return null;

  const clamp01 = (v: number) => Math.max(0, Math.min(1, v));
  const shadowX = clamp01(shadow) * 100;
  const highlightX = clamp01(highlight) * 100;
  const midX = clamp01(shadow + midtone * (highlight - shadow)) * 100;

  return (
    <svg
      viewBox="0 0 100 100"
      preserveAspectRatio="none"
      style={{ width: "100%", height }}
      className="rounded-md bg-zinc-900/70 border border-zinc-800/40"
    >
      {curves.map((c, i) => (
        <polyline
          key={i}
          points={c.points}
          fill="none"
          stroke={c.color}
          strokeWidth={1}
          vectorEffect="non-scaling-stroke"
          opacity={0.85}
        />
      ))}
      <line x1={shadowX} x2={shadowX} y1={0} y2={100} stroke="#a1a1aa" strokeWidth={1}
            vectorEffect="non-scaling-stroke" strokeDasharray="3 2" opacity={0.9} />
      <line x1={midX} x2={midX} y1={0} y2={100} stroke="#f59e0b" strokeWidth={1}
            vectorEffect="non-scaling-stroke" strokeDasharray="3 2" opacity={0.9} />
      <line x1={highlightX} x2={highlightX} y1={0} y2={100} stroke="#a1a1aa" strokeWidth={1}
            vectorEffect="non-scaling-stroke" strokeDasharray="3 2" opacity={0.9} />
    </svg>
  );
}
