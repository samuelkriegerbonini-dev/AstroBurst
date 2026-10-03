import { memo, useEffect, useMemo, useRef, useState } from "react";
import { lutNodataRgb, type DisplayTransfer } from "../../utils/displayTransfer";
import { COLORBAR_HEIGHT_PX, colorbarPixels, colorbarTicks, valueAtFraction } from "../../utils/colorbar";

interface ColorbarProps {
  transfer: DisplayTransfer;
  lut: Uint8Array;
  unit: string | null;
  pending: boolean;
  centre: number | null;
}

const TICK_MARK_PX = 3;

function labelShift(index: number, last: number): string {
  if (index === 0) return "none";
  if (index === last) return "translateX(-100%)";
  return "translateX(-50%)";
}

function Colorbar({ transfer, lut, unit, pending, centre }: ColorbarProps) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(0);
  const [readback, setReadback] = useState<string | null>(null);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const measure = () => setWidth(Math.round(el.clientWidth));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || width < 1) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const row = colorbarPixels(lut, width, transfer.invert);
    const img = ctx.createImageData(width, COLORBAR_HEIGHT_PX);
    for (let y = 0; y < COLORBAR_HEIGHT_PX; y++) img.data.set(row, y * row.length);
    ctx.putImageData(img, 0, 0);
  }, [lut, width, transfer.invert]);

  const ticks = useMemo(() => colorbarTicks(transfer, width, centre), [transfer, width, centre]);
  const [nr, ng, nb] = lutNodataRgb(lut);
  const last = ticks.length - 1;

  const onMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    if (rect.width <= 0) return;
    const frac = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    setReadback(valueAtFraction(frac, transfer).toPrecision(4));
  };

  return (
    <div
      data-colorbar
      data-pending={pending ? "true" : "false"}
      className={`shrink-0 px-3 pt-0.5 pb-3 text-[9px] font-mono text-zinc-500 ${pending ? "opacity-50" : ""}`}
    >
      <div className="flex items-start gap-2">
        <span
          data-colorbar-nodata
          title="no data: NaN or exact-0 padding"
          className="shrink-0 rounded-sm border border-zinc-700/60"
          style={{ width: COLORBAR_HEIGHT_PX, height: COLORBAR_HEIGHT_PX, background: `rgb(${nr}, ${ng}, ${nb})` }}
        />
        <div ref={wrapRef} className="relative flex-1 min-w-0">
          <canvas
            data-colorbar-strip
            ref={canvasRef}
            width={Math.max(width, 1)}
            height={COLORBAR_HEIGHT_PX}
            className="block w-full"
            style={{ height: COLORBAR_HEIGHT_PX }}
            onMouseMove={onMove}
            onMouseLeave={() => setReadback(null)}
          />
          {ticks.map((tick, i) => (
            <span
              key={i}
              data-tick={tick.kind}
              data-value={tick.value}
              className="absolute pointer-events-none"
              style={{ left: `${tick.frac * 100}%`, top: COLORBAR_HEIGHT_PX }}
            >
              <span
                className={`absolute left-0 top-0 w-px ${tick.kind === "centre" ? "bg-amber-400/80" : "bg-zinc-500"}`}
                style={{ height: TICK_MARK_PX }}
              />
              <span
                className={`absolute left-0 whitespace-nowrap leading-none ${tick.kind === "centre" ? "text-amber-400/80" : ""}`}
                style={{ top: TICK_MARK_PX, transform: labelShift(i, last) }}
              >
                {tick.label}
              </span>
            </span>
          ))}
        </div>
        <span className="shrink-0 w-14 text-right text-zinc-300" style={{ lineHeight: `${COLORBAR_HEIGHT_PX}px` }}>
          {readback !== null && <span data-colorbar-readback>{readback}</span>}
        </span>
        {unit !== null && (
          <span data-colorbar-unit className="shrink-0" style={{ lineHeight: `${COLORBAR_HEIGHT_PX}px` }}>
            {unit}
          </span>
        )}
      </div>
    </div>
  );
}

export default memo(Colorbar);
