import { useState, useCallback, useRef, useEffect } from "react";
import {
  clampViewerScale,
  fitScale,
  fitsPerRenderPx,
  isZoomPresetActive,
  renderScaleForFits,
  wheelZoomFactor,
  zoomPercentLabel,
} from "../utils/viewerZoom";

export interface Transform {
  scale: number;
  x: number;
  y: number;
}

const ZOOM_STEP = 1.15;

export interface UseViewerTransformOptions {
  containerRef: React.RefObject<HTMLDivElement | null>;
  renderW: number;
  renderH: number;
  fitsW?: number;
}

export function useViewerTransform({ containerRef, renderW, renderH, fitsW }: UseViewerTransformOptions) {
  const [containerEl, setContainerEl] = useState<HTMLDivElement | null>(null);
  const [transform, setTransformState] = useState<Transform>({ scale: 1, x: 0, y: 0 });
  const transformRef = useRef(transform);
  transformRef.current = transform;
  const userInteractedRef = useRef(false);

  const hasRenderDims = renderW > 0 && renderH > 0;
  const fitsPerRender = fitsPerRenderPx(fitsW, renderW);
  const fitsPerRenderRef = useRef(fitsPerRender);
  fitsPerRenderRef.current = fitsPerRender;

  const attachContainer = useCallback(
    (el: HTMLDivElement | null) => {
      containerRef.current = el;
      setContainerEl(el);
    },
    [containerRef],
  );

  const setTransform = useCallback((action: React.SetStateAction<Transform>) => {
    userInteractedRef.current = true;
    setTransformState(action);
  }, []);

  const fitToWindow = useCallback(() => {
    const container = containerRef.current;
    if (!container || !hasRenderDims) return;
    const cw = container.clientWidth;
    const ch = container.clientHeight;
    if (cw === 0 || ch === 0) return;
    const scale = fitScale(cw, ch, renderW, renderH, fitsPerRender);
    userInteractedRef.current = false;
    setTransformState({
      scale,
      x: (cw - renderW * scale) / 2,
      y: (ch - renderH * scale) / 2,
    });
  }, [containerRef, renderW, renderH, hasRenderDims, fitsPerRender]);

  const fitToWindowRef = useRef(fitToWindow);
  fitToWindowRef.current = fitToWindow;

  const zoomTo = useCallback((newScale: number, centerX?: number, centerY?: number) => {
    userInteractedRef.current = true;
    setTransformState((prev) => {
      const clamped = clampViewerScale(newScale, fitsPerRenderRef.current);
      const container = containerRef.current;
      if (!container) return { ...prev, scale: clamped };
      const rect = container.getBoundingClientRect();
      const cx = centerX ?? rect.width / 2;
      const cy = centerY ?? rect.height / 2;
      const ratio = clamped / prev.scale;
      return {
        scale: clamped,
        x: cx - (cx - prev.x) * ratio,
        y: cy - (cy - prev.y) * ratio,
      };
    });
  }, [containerRef]);

  const zoomIn = useCallback(() => zoomTo(transformRef.current.scale * ZOOM_STEP), [zoomTo]);
  const zoomOut = useCallback(() => zoomTo(transformRef.current.scale / ZOOM_STEP), [zoomTo]);
  const zoomToFits = useCallback(
    (fitsScale: number) => zoomTo(renderScaleForFits(fitsScale, fitsPerRenderRef.current)),
    [zoomTo],
  );

  const setOneToOne = useCallback(() => {
    const container = containerRef.current;
    if (!container || !hasRenderDims) return;
    userInteractedRef.current = true;
    const scale = clampViewerScale(renderScaleForFits(1, fitsPerRender), fitsPerRender);
    setTransformState({
      scale,
      x: (container.clientWidth - renderW * scale) / 2,
      y: (container.clientHeight - renderH * scale) / 2,
    });
  }, [containerRef, renderW, renderH, hasRenderDims, fitsPerRender]);

  const wheelRafRef = useRef<number | null>(null);
  const pendingWheelRef = useRef<{ factor: number; clientX: number; clientY: number } | null>(null);

  const handleWheelNative = useCallback(
    (e: WheelEvent) => {
      e.preventDefault();
      const factor = wheelZoomFactor(e.deltaY, e.deltaMode);
      const prev = pendingWheelRef.current;
      pendingWheelRef.current = {
        factor: (prev?.factor ?? 1) * factor,
        clientX: e.clientX,
        clientY: e.clientY,
      };
      if (wheelRafRef.current !== null) return;
      wheelRafRef.current = requestAnimationFrame(() => {
        wheelRafRef.current = null;
        const pending = pendingWheelRef.current;
        pendingWheelRef.current = null;
        const rect = containerRef.current?.getBoundingClientRect();
        if (!pending || !rect) return;
        const cx = pending.clientX - rect.left;
        const cy = pending.clientY - rect.top;
        userInteractedRef.current = true;
        setTransformState((t) => {
          const newScale = clampViewerScale(t.scale * pending.factor, fitsPerRenderRef.current);
          const ratio = newScale / t.scale;
          return {
            scale: newScale,
            x: cx - (cx - t.x) * ratio,
            y: cy - (cy - t.y) * ratio,
          };
        });
      });
    },
    [containerRef],
  );

  useEffect(() => {
    if (!containerEl) return;
    containerEl.addEventListener("wheel", handleWheelNative, { passive: false });
    return () => {
      containerEl.removeEventListener("wheel", handleWheelNative);
      if (wheelRafRef.current !== null) {
        cancelAnimationFrame(wheelRafRef.current);
        wheelRafRef.current = null;
        pendingWheelRef.current = null;
      }
    };
  }, [handleWheelNative, containerEl]);

  useEffect(() => {
    if (hasRenderDims) {
      userInteractedRef.current = false;
      requestAnimationFrame(() => fitToWindowRef.current());
    }
  }, [hasRenderDims, renderW, renderH]);

  useEffect(() => {
    if (!containerEl) return;
    let raf: number | null = null;
    const ro = new ResizeObserver(() => {
      if (raf !== null) return;
      raf = requestAnimationFrame(() => {
        raf = null;
        if (!userInteractedRef.current) fitToWindowRef.current();
      });
    });
    ro.observe(containerEl);
    return () => {
      ro.disconnect();
      if (raf !== null) cancelAnimationFrame(raf);
    };
  }, [containerEl]);

  return {
    attachContainer,
    transform,
    transformRef,
    setTransform,
    fitToWindow,
    zoomTo,
    zoomToFits,
    zoomIn,
    zoomOut,
    setOneToOne,
    hasRenderDims,
    fitsPerRender,
    isPresetActive: (preset: number) => isZoomPresetActive(transform.scale, preset, fitsPerRender),
    zoomPct: zoomPercentLabel(transform.scale, fitsPerRender),
    ZOOM_STEP,
  };
}

export const ZOOM_PRESETS = [0.25, 0.5, 1, 2, 4, 8] as const;
