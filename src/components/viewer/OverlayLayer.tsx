import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { useDisplayContext, useDisplayedImage } from "../../context/PreviewContext";
import { useOverlayDoc } from "../../hooks/useOverlayStore";
import { useRegionKey } from "../../hooks/useRegionKey";
import { getWcsInfo, gridLines, isWcsGridError } from "../../services/astrometry";
import type { GridFrame } from "../../shared/types/display";
import { horizontalPixelScaleArcsec, overlayWcsPath } from "../../utils/compass";
import { overlayStore, type OverlayPaintContext } from "../../utils/overlayStore";
import { wcsOverlayStatus } from "../../utils/wcsOverlayStatus";
import { screenPxPerImagePx, type ViewerTransform } from "../../utils/pixelMapping";
import { isRegionMappingUsable, regionPointToScreen, resolveRegionHost, type RegionMapping } from "../../utils/regionCoords";
import type { Pt } from "../../utils/regionGeometry";
import { COMPASS_LAYER_ID, COMPASS_LAYER_KIND, createCompassPainter } from "./painters/compassPainter";
import { createGridPainter, GRID_LAYER_ID, GRID_LAYER_KIND } from "./painters/gridPainter";

interface OverlayLayerProps {
  containerRef: RefObject<HTMLDivElement | null>;
  transform: ViewerTransform;
  renderW: number;
  renderH: number;
  fitsW: number;
  fitsH: number;
  enabled: boolean;
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function useWcsGridLayer(
  fileKey: string | null,
  wcsPath: string | null,
  grid: boolean,
  frame: GridFrame,
  density: number,
): void {
  useEffect(() => {
    if (!fileKey || !wcsPath || !grid) return;
    let cancelled = false;
    gridLines(wcsPath, frame, density)
      .then((result) => {
        if (cancelled) return;
        if (isWcsGridError(result)) {
          console.warn("[AstroBurst] WCS grid unavailable:", result.error);
          wcsOverlayStatus.set("grid", result.error);
          return;
        }
        wcsOverlayStatus.set("grid", null);
        overlayStore.add(fileKey, {
          id: GRID_LAYER_ID,
          kind: GRID_LAYER_KIND,
          visible: true,
          paint: createGridPainter(result),
        });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        console.error("[AstroBurst] WCS grid failed:", err);
        wcsOverlayStatus.set("grid", errorText(err));
      });
    return () => {
      cancelled = true;
      wcsOverlayStatus.set("grid", null);
      overlayStore.remove(fileKey, GRID_LAYER_ID);
    };
  }, [fileKey, wcsPath, grid, frame, density]);
}

function useCompassLayer(fileKey: string | null, wcsPath: string | null, compass: boolean): void {
  useEffect(() => {
    if (!fileKey || !wcsPath || !compass) return;
    let cancelled = false;
    getWcsInfo(wcsPath)
      .then((info) => {
        if (cancelled) return;
        const scaleX = horizontalPixelScaleArcsec(info);
        if (!info.north_vec || !info.east_vec || !Number.isFinite(scaleX)) {
          console.warn("[AstroBurst] compass unavailable: the WCS has no usable orientation");
          wcsOverlayStatus.set("compass", "the WCS has no usable orientation");
          return;
        }
        wcsOverlayStatus.set("compass", null);
        overlayStore.add(fileKey, {
          id: COMPASS_LAYER_ID,
          kind: COMPASS_LAYER_KIND,
          visible: true,
          paint: createCompassPainter({
            northVec: info.north_vec,
            eastVec: info.east_vec,
            pixelScaleArcsec: scaleX,
            imageCentre: { x: (info.naxis1 - 1) / 2, y: (info.naxis2 - 1) / 2 },
          }),
        });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        console.warn("[AstroBurst] compass unavailable:", err);
        wcsOverlayStatus.set("compass", errorText(err));
      });
    return () => {
      cancelled = true;
      wcsOverlayStatus.set("compass", null);
      overlayStore.remove(fileKey, COMPASS_LAYER_ID);
    };
  }, [fileKey, wcsPath, compass]);
}

function OverlayLayer({ containerRef, transform, renderW, renderH, fitsW, fitsH, enabled }: OverlayLayerProps) {
  const regionKey = useRegionKey();
  const fileKey = enabled ? regionKey : null;
  const { display } = useDisplayContext();
  const displayed = useDisplayedImage();
  const wcsPath = overlayWcsPath(fileKey, displayed.path);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [hostEl, setHostEl] = useState<HTMLElement | null>(null);

  const mapping: RegionMapping = { transform, renderW, renderH, fitsW, fitsH };
  const active = enabled && fileKey !== null && isRegionMappingUsable(mapping);
  const mappingRef = useRef(mapping);
  mappingRef.current = mapping;

  useWcsGridLayer(active ? fileKey : null, wcsPath, display.grid, display.gridFrame, display.gridDensity);
  useCompassLayer(active ? fileKey : null, wcsPath, display.compass);
  const doc = useOverlayDoc(fileKey);
  const hasLayers = doc.layers.some((l) => l.visible);

  const attachCanvas = useCallback(
    (el: HTMLCanvasElement | null) => {
      canvasRef.current = el;
      setHostEl(resolveRegionHost(containerRef.current, el));
    },
    [containerRef],
  );

  useLayoutEffect(() => {
    if (!active || !hostEl) return;
    const measure = () => setSize({ w: hostEl.clientWidth, h: hostEl.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(hostEl);
    return () => ro.disconnect();
  }, [hostEl, active]);

  const toScreen = useCallback((p: Pt): Pt => regionPointToScreen(p, mappingRef.current), []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !active) return;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(size.w * dpr));
    const h = Math.max(1, Math.round(size.h * dpr));
    if (canvas.width !== w) canvas.width = w;
    if (canvas.height !== h) canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size.w, size.h);
    const paint: OverlayPaintContext = {
      ctx,
      toScreen,
      width: size.w,
      height: size.h,
      screenPxPerImagePx: screenPxPerImagePx(transform, renderW, fitsW),
    };
    for (const layer of doc.layers) {
      if (layer.visible) layer.paint(paint);
    }
  }, [active, size, transform, renderW, renderH, fitsW, fitsH, doc, toScreen]);

  if (!active || !hasLayers) return null;

  return (
    <canvas
      ref={attachCanvas}
      style={{
        position: "absolute",
        inset: 0,
        width: "100%",
        height: "100%",
        pointerEvents: "none",
        zIndex: 4,
      }}
    />
  );
}

export default memo(OverlayLayer);
