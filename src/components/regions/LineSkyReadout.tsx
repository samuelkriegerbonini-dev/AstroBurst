import { memo, useEffect, useRef, useState } from "react";
import { skySeparation } from "../../services/astrometry";
import type { SkySeparationResult } from "../../shared/types/astrometry";
import type { RegionShape } from "../../shared/types/regions";
import { formatPositionAngle, formatSeparation, pixelLength } from "../../utils/skyMeasure";

interface LineSkyReadoutProps {
  measurePath: string | null;
  shape: RegionShape;
}

const SKY_DEBOUNCE_MS = 250;
const LENGTH_DIGITS = 1;

function LineSkyReadout({ measurePath, shape }: LineSkyReadoutProps) {
  const line = shape.shape === "line" ? shape : null;
  const lineKey = line ? `${line.x1},${line.y1},${line.x2},${line.y2}` : null;
  const [sky, setSky] = useState<SkySeparationResult | null>(null);
  const seqRef = useRef(0);

  useEffect(() => {
    const seq = ++seqRef.current;
    setSky(null);
    if (!measurePath || !lineKey) return;
    const [x1, y1, x2, y2] = lineKey.split(",").map(Number);
    const timer = setTimeout(async () => {
      try {
        const res = await skySeparation(measurePath, [x1, y1], [x2, y2], true);
        if (seqRef.current !== seq) return;
        setSky(res);
      } catch {
        if (seqRef.current === seq) setSky(null);
      }
    }, SKY_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [measurePath, lineKey]);

  if (!line) return null;

  const length = sky?.pixel_length ?? pixelLength(line.x1, line.y1, line.x2, line.y2);
  const text = sky
    ? `length ${length.toFixed(LENGTH_DIGITS)} px - sep ${formatSeparation(sky.separation_arcsec)} - PA ${formatPositionAngle(sky.position_angle_deg)}`
    : `length ${length.toFixed(LENGTH_DIGITS)} px`;

  return <div className="font-mono text-[9px] text-zinc-500">{text}</div>;
}

export default memo(LineSkyReadout);
