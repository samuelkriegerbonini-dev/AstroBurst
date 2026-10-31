export interface HistogramFrame {
  min: number;
  max: number;
}

export interface FrameMapping {
  scale: number;
  offset: number;
}

function finiteFrame(frame: HistogramFrame): boolean {
  return Number.isFinite(frame.min) && Number.isFinite(frame.max);
}

export function sharedHistogramFrame(frames: readonly HistogramFrame[]): HistogramFrame | null {
  if (frames.length === 0 || !frames.every(finiteFrame)) return null;
  const min = Math.min(...frames.map((f) => f.min));
  const max = Math.max(...frames.map((f) => f.max));
  return max > min ? { min, max } : null;
}

export function frameMapping(own: HistogramFrame, shared: HistogramFrame): FrameMapping | null {
  const range = shared.max - shared.min;
  if (!finiteFrame(own) || !finiteFrame(shared) || !(range > 0)) return null;
  return {
    scale: (own.max - own.min) / range,
    offset: ((own.min - shared.min) / range) * 100,
  };
}
