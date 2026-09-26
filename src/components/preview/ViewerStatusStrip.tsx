import { memo, useEffect, useState } from "react";
import { useMousePixel } from "../../hooks/useMousePixelStore";
import { useAnalysisTarget } from "../../hooks/useAnalysisTarget";
import { probePixel } from "../../services/display";
import { getWcsInfo, pixelToWorld } from "../../services/astrometry";
import { statusStripIdleText, statusStripParts, viewerPublishesPixel, type PixelValueAt, type SkyAt } from "../../utils/previewShell";
import { ZERO_BASED_PIXEL_TITLE } from "../../utils/regionGeometry";

const HOVER_DEBOUNCE_MS = 40;
const PROBE_BOX = 1;

function ViewerStatusStrip() {
  const mousePixel = useMousePixel();
  const { path, composite, fileRgbView } = useAnalysisTarget();
  const publishesPixel = viewerPublishesPixel({ composite, fileRgbView });
  const pixel = publishesPixel ? mousePixel : null;
  const [valueAt, setValueAt] = useState<(PixelValueAt & { path: string }) | null>(null);
  const [skyAt, setSkyAt] = useState<(SkyAt & { path: string }) | null>(null);
  const [wcsPath, setWcsPath] = useState<string | null>(null);
  const x = pixel?.x ?? null;
  const y = pixel?.y ?? null;
  const hasWcs = path !== null && wcsPath === path;

  useEffect(() => {
    if (!path) return;
    let cancelled = false;
    getWcsInfo(path)
      .then(() => {
        if (!cancelled) setWcsPath(path);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [path]);

  useEffect(() => {
    if (!path || x === null || y === null) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      probePixel(path, x, y, PROBE_BOX)
        .then((res) => {
          if (!cancelled) setValueAt({ path, x, y, value: res.value, unit: res.unit });
        })
        .catch(() => {
          if (!cancelled) setValueAt({ path, x, y, value: null, unit: null });
        });
    }, HOVER_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [path, x, y]);

  useEffect(() => {
    if (!path || !hasWcs || x === null || y === null) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      pixelToWorld(path, [[x, y]], "icrs")
        .then((res) => {
          const radec = res.points[0];
          if (!cancelled && radec) setSkyAt({ path, x, y, radec });
        })
        .catch(() => {});
    }, HOVER_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [path, hasWcs, x, y]);

  const parts = statusStripParts(
    pixel,
    valueAt !== null && valueAt.path === path ? valueAt : null,
    hasWcs && skyAt !== null && skyAt.path === path ? skyAt : null,
  );

  return (
    <div
      className="flex items-center gap-4 px-3 shrink-0 overflow-hidden whitespace-nowrap text-[10px] font-mono text-zinc-500"
      style={{ height: 18, background: "rgba(5,5,16,0.6)", borderTop: "1px solid var(--ab-border)" }}
    >
      {parts ? (
        <>
          <span className="text-zinc-400" title={ZERO_BASED_PIXEL_TITLE}>{parts.position}</span>
          <span style={{ color: "rgba(251,191,36,0.8)" }} title="Pixel value of the image being measured">{parts.value}</span>
          {parts.sky && <span style={{ color: "rgba(52,211,153,0.7)" }}>{parts.sky}</span>}
        </>
      ) : (
        <span className="text-zinc-600">{statusStripIdleText(publishesPixel)}</span>
      )}
    </div>
  );
}

export default memo(ViewerStatusStrip);
