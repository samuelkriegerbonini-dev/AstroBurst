import { useEffect, useRef } from "react";
import { useDqContext } from "../../context/PreviewContext";
import { paintDqMask, DQ_OVERLAY_RGB, DQ_OVERLAY_ALPHA } from "../../utils/dqOverlay";
import type { DqMaskData } from "../../shared/types/dq";

interface DqOverlayCanvasProps {
  canvasRef: React.RefObject<HTMLCanvasElement | null>;
}

export default function DqOverlayCanvas({ canvasRef }: DqOverlayCanvasProps) {
  const { dqMask, overlay } = useDqContext();
  const lastPaintRef = useRef<{ canvas: HTMLCanvasElement; mask: DqMaskData } | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    if (!overlay.enabled || !dqMask) {
      canvas.style.display = "none";
      return;
    }
    const last = lastPaintRef.current;
    if (!last || last.canvas !== canvas || last.mask !== dqMask) {
      canvas.width = dqMask.width;
      canvas.height = dqMask.height;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      const img = ctx.createImageData(dqMask.width, dqMask.height);
      paintDqMask(dqMask.cells, dqMask.width, dqMask.height, DQ_OVERLAY_RGB, DQ_OVERLAY_ALPHA, img.data);
      ctx.putImageData(img, 0, 0);
      lastPaintRef.current = { canvas, mask: dqMask };
    }
    canvas.style.display = "block";
  });

  return null;
}
