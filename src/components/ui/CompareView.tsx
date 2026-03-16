import { useState, useCallback, useRef, useEffect, memo } from "react";

interface CompareViewProps {
  originalUrl: string;
  resultUrl: string;
  originalLabel?: string;
  resultLabel?: string;
  accent?: string;
  height?: number | string;
}

function CompareView({
  originalUrl,
  resultUrl,
  originalLabel = "Original",
  resultLabel = "Result",
  accent = "teal",
  height,
}: CompareViewProps) {
  const [position, setPosition] = useState(50);
  const containerRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  const handleMouseDown = useCallback(() => {
    dragging.current = true;
  }, []);

  const handleMouseMove = useCallback((e: MouseEvent) => {
    if (!dragging.current || !containerRef.current) return;
    const rect = containerRef.current.getBoundingClientRect();
    const x = ((e.clientX - rect.left) / rect.width) * 100;
    setPosition(Math.max(0, Math.min(100, x)));
  }, []);

  const handleMouseUp = useCallback(() => {
    dragging.current = false;
  }, []);

  useEffect(() => {
    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };
  }, [handleMouseMove, handleMouseUp]);

  const style: React.CSSProperties = height ? { height } : {};

  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-xs text-zinc-400">{originalLabel} / {resultLabel}</span>
      <div
        ref={containerRef}
        className="ab-compare-view"
        style={style}
        onMouseDown={handleMouseDown}
      >
        <img
          src={resultUrl}
          alt={resultLabel}
          className="absolute inset-0 w-full h-full object-contain"
          draggable={false}
        />
        <div
          className="absolute inset-0 overflow-hidden"
          style={{ width: `${position}%` }}
        >
          <img
            src={originalUrl}
            alt={originalLabel}
            className="absolute inset-0 w-full h-full object-contain"
            style={{
              width: containerRef.current ? `${containerRef.current.offsetWidth}px` : "100%",
              maxWidth: "none",
            }}
            draggable={false}
          />
        </div>
        <div
          className="ab-compare-divider"
          style={{ left: `${position}%` }}
          data-accent={accent}
        >
          <div className="ab-compare-handle">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path d="M3 6H1M11 6H9M3 6L5 4M3 6L5 8M9 6L7 4M9 6L7 8" stroke="#333" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
          </div>
        </div>
        <div className="ab-compare-label left-2">{originalLabel}</div>
        <div className="ab-compare-label right-2">{resultLabel}</div>
      </div>
    </div>
  );
}

export default memo(CompareView);
