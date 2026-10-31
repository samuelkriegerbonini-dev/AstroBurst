import { useEffect, useMemo, useState } from "react";
import { computeHistogram } from "../../services/analysis";
import { frameMapping, sharedHistogramFrame, type HistogramFrame } from "../../utils/histogramFrame";

interface ChannelSpec {
  key: string;
  color: string;
}

interface StfMarkers {
  shadow: number;
  midtone: number;
  highlight: number;
  symmetryPoint?: undefined;
}

interface SymmetryPointMarker {
  symmetryPoint: number;
  shadow?: undefined;
  midtone?: undefined;
  highlight?: undefined;
}

type StfHistogramProps = (StfMarkers | SymmetryPointMarker) & {
  channels: ChannelSpec[];
  height?: number;
};

interface Curve extends HistogramFrame {
  color: string;
  points: string;
}

function sharedFrameTransform(curve: Curve, shared: HistogramFrame | null): string | undefined {
  const mapping = shared ? frameMapping(curve, shared) : null;
  return mapping ? `matrix(${mapping.scale} 0 0 1 ${mapping.offset} 0)` : undefined;
}

export default function StfHistogram(props: StfHistogramProps) {
  const { channels, height = 44 } = props;
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
          const frame = { min: res.data_min, max: res.data_max };
          if (n < 2) return { color: channels[i].color, points: "", ...frame };
          const maxLog = Math.log1p(Math.max(1, ...bins));
          const points = bins
            .map((v, j) => {
              const x = (j / (n - 1)) * 100;
              const y = 100 - (Math.log1p(Math.max(0, v)) / maxLog) * 96;
              return `${x.toFixed(2)},${y.toFixed(2)}`;
            })
            .join(" ");
          return { color: channels[i].color, points, ...frame };
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
  const sharedFrame = props.symmetryPoint === undefined ? null : sharedHistogramFrame(curves);
  const markers = props.symmetryPoint === undefined
    ? [
        { x: clamp01(props.shadow) * 100, color: "#a1a1aa" },
        { x: clamp01(props.shadow + props.midtone * (props.highlight - props.shadow)) * 100, color: "#f59e0b" },
        { x: clamp01(props.highlight) * 100, color: "#a1a1aa" },
      ]
    : [{ x: clamp01(props.symmetryPoint) * 100, color: "#f59e0b" }];

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
          transform={sharedFrameTransform(c, sharedFrame)}
          fill="none"
          stroke={c.color}
          strokeWidth={1}
          vectorEffect="non-scaling-stroke"
          opacity={0.85}
        />
      ))}
      {markers.map((m, i) => (
        <line key={i} x1={m.x} x2={m.x} y1={0} y2={100} stroke={m.color} strokeWidth={1}
              vectorEffect="non-scaling-stroke" strokeDasharray="3 2" opacity={0.9} />
      ))}
    </svg>
  );
}
