import { useState, useCallback, useRef, useEffect } from "react";

export interface Transform {
  scale: number;
  x: number;
  y: number;
}

const ZOOM_MIN = 0.1;
const ZOOM_MAX = 32;
const ZOOM_STEP = 1.15;

function clamp(t: Transform): Transform {
  return { scale: Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, t.scale)), x: t.x, y: t.y };
}

export interface UseViewerTransformOptions {
  containerRef: React.RefObject<HTMLDivElement | null>;
  renderW: number;
  renderH: number;
}

export function useViewerTransform({ containerRef, renderW, renderH }: UseViewerTransformOptions) {
  const [transform, setTransform] = useState<Transform>({ scale: 1, x: 0, y: 0 });
  const transformRef = useRef(transform);
  transformRef.current = transform;

  const hasRenderDims = renderW > 0 && renderH > 0;

  const fitToWindow = useCallback(() => {
    const container = containerRef.current;
    if (!container || !hasRenderDims) return;
    const cw = container.clientWidth;
    const ch = container.clientHeight;
    if (cw === 0 || ch === 0) return;
    const scale = Math.min(cw / renderW, ch / renderH, 1);
    setTransform({
      scale,
      x: (cw - renderW * scale) / 2,
      y: (ch - renderH * scale) / 2,
    });
  }, [containerRef, renderW, renderH, hasRenderDims]);

  const fitToWindowRef = useRef(fitToWindow);
  fitToWindowRef.current = fitToWindow;

  const zoomTo = useCallback((newScale: number, centerX?: number, centerY?: number) => {
    setTransform((prev) => {
      const container = containerRef.current;
      if (!container) return { ...prev, scale: Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, newScale)) };
      const rect = container.getBoundingClientRect();
      const cx = centerX ?? rect.width / 2;
      const cy = centerY ?? rect.height / 2;
      const ratio = newScale / prev.scale;
      return clamp({
        scale: newScale,
        x: cx - (cx - prev.x) * ratio,
        y: cy - (cy - prev.y) * ratio,
      });
    });
  }, [containerRef]);

  const zoomIn = useCallback(() => zoomTo(transformRef.current.scale * ZOOM_STEP), [zoomTo]);
  const zoomOut = useCallback(() => zoomTo(transformRef.current.scale / ZOOM_STEP), [zoomTo]);

  const setOneToOne = useCallback(() => {
    const container = containerRef.current;
    if (!container || !hasRenderDims) return;
    setTransform({
      scale: 1,
      x: (container.clientWidth - renderW) / 2,
      y: (container.clientHeight - renderH) / 2,
    });
  }, [containerRef, renderW, renderH, hasRenderDims]);

  const handleWheelNative = useCallback(
    (e: WheelEvent) => {
      e.preventDefault();
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return;
      const factor = e.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP;
      const cx = e.clientX - rect.left;
      const cy = e.clientY - rect.top;
      setTransform((prev) => {
        const newScale = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, prev.scale * factor));
        const ratio = newScale / prev.scale;
        return clamp({
          scale: newScale,
          x: cx - (cx - prev.x) * ratio,
          y: cy - (cy - prev.y) * ratio,
        });
      });
    },
    [containerRef],
  );

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    el.addEventListener("wheel", handleWheelNative, { passive: false });
    return () => el.removeEventListener("wheel", handleWheelNative);
  }, [handleWheelNative, containerRef]);

  useEffect(() => {
    if (hasRenderDims) {
      requestAnimationFrame(() => fitToWindowRef.current());
    }
  }, [hasRenderDims, renderW, renderH]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => fitToWindowRef.current());
    ro.observe(el);
    return () => ro.disconnect();
  }, [containerRef]);

  return {
    transform,
    transformRef,
    setTransform,
    fitToWindow,
    zoomTo,
    zoomIn,
    zoomOut,
    setOneToOne,
    hasRenderDims,
    zoomPct: `${Math.round(transform.scale * 100)}%`,
    ZOOM_STEP,
  };
}

export const ZOOM_PRESETS = [0.25, 0.5, 1, 2, 4, 8] as const;
