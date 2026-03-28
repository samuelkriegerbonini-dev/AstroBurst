import { useState, useCallback, useRef, useEffect, memo } from "react";
import { ZoomIn, ZoomOut, Home } from "lucide-react";

interface ZoomPanViewProps {
  src: string;
  alt?: string;
  className?: string;
}

const ZOOM_MIN = 0.25;
const ZOOM_MAX = 16;
const ZOOM_STEP = 1.15;

function ZoomPanView({ src, alt = "", className = "" }: ZoomPanViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(1);
  const [translate, setTranslate] = useState({ x: 0, y: 0 });
  const dragRef = useRef<{ dragging: boolean; startX: number; startY: number; origTx: number; origTy: number }>({
    dragging: false, startX: 0, startY: 0, origTx: 0, origTy: 0,
  });

  const resetView = useCallback(() => {
    setScale(1);
    setTranslate({ x: 0, y: 0 });
  }, []);

  useEffect(() => {
    resetView();
  }, [src, resetView]);

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    const container = containerRef.current;
    if (!container) return;

    const rect = container.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;

    setScale((prev) => {
      const factor = e.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP;
      const next = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, prev * factor));
      const ratio = next / prev;

      setTranslate((t) => ({
        x: mx - ratio * (mx - t.x),
        y: my - ratio * (my - t.y),
      }));

      return next;
    });
  }, []);

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    dragRef.current = {
      dragging: true,
      startX: e.clientX,
      startY: e.clientY,
      origTx: translate.x,
      origTy: translate.y,
    };
  }, [translate]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    const d = dragRef.current;
    if (!d.dragging) return;
    setTranslate({
      x: d.origTx + (e.clientX - d.startX),
      y: d.origTy + (e.clientY - d.startY),
    });
  }, []);

  const handlePointerUp = useCallback(() => {
    dragRef.current.dragging = false;
  }, []);

  const handleDoubleClick = useCallback((e: React.MouseEvent) => {
    const container = containerRef.current;
    if (!container) return;
    const rect = container.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;

    if (scale > 1.05) {
      resetView();
    } else {
      const next = 3;
      const ratio = next / scale;
      setTranslate((t) => ({
        x: mx - ratio * (mx - t.x),
        y: my - ratio * (my - t.y),
      }));
      setScale(next);
    }
  }, [scale, resetView]);

  const zoomIn = useCallback(() => {
    setScale((s) => Math.min(ZOOM_MAX, s * 1.5));
  }, []);

  const zoomOut = useCallback(() => {
    setScale((s) => Math.max(ZOOM_MIN, s / 1.5));
  }, []);

  const zoomPct = `${Math.round(scale * 100)}%`;

  return (
    <div
      ref={containerRef}
      className={`relative overflow-hidden ${className}`}
      style={{ cursor: dragRef.current.dragging ? "grabbing" : "grab" }}
      onWheel={handleWheel}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerCancel={handlePointerUp}
      onDoubleClick={handleDoubleClick}
    >
      <div
        className="absolute origin-top-left will-change-transform"
        style={{
          transform: `translate(${translate.x}px, ${translate.y}px) scale(${scale})`,
        }}
      >
        <img
          src={src}
          alt={alt}
          className="block max-w-none"
          draggable={false}
          style={{ imageRendering: scale >= 2 ? "pixelated" : "auto" }}
        />
      </div>

      <div className="absolute top-2 right-2 flex flex-col gap-1 z-10">
        {[
          { icon: ZoomIn, action: zoomIn, title: "Zoom in" },
          { icon: ZoomOut, action: zoomOut, title: "Zoom out" },
          { icon: Home, action: resetView, title: "Reset view" },
        ].map(({ icon: Icon, action, title }) => (
          <button
            key={title}
            onClick={(e) => { e.stopPropagation(); action(); }}
            title={title}
            className="w-7 h-7 flex items-center justify-center rounded-md
              bg-zinc-900/80 backdrop-blur-sm border border-zinc-700/50
              text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800/90
              transition-all duration-150 active:scale-95"
          >
            <Icon size={13} strokeWidth={1.8} />
          </button>
        ))}
      </div>

      <div className="absolute bottom-2 left-2 z-10
        text-[10px] font-mono text-zinc-600
        bg-zinc-950/70 backdrop-blur-sm rounded px-2 py-0.5
        border border-zinc-800/30 select-none pointer-events-none"
      >
        {zoomPct}
      </div>
    </div>
  );
}

export default memo(ZoomPanView);
