import { memo, useCallback, useEffect, useId, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import {
  arrowDelta,
  CROP_BODY_ROLE_DESCRIPTION,
  CROP_EDGE_HANDLES,
  CROP_HANDLE_ROLE_DESCRIPTION,
  CROP_HANDLES,
  CROP_TARGET_CURSORS,
  CROP_TARGET_LABELS,
  cropHitLayout,
  cropSizeLabel,
  editorBoxHeight,
  editorBoxSize,
  editorNaturalWidth,
  nudgeTarget,
  pointerDragStep,
  rectFromMargins,
  sameMargins,
  suppressTextSelection,
  unitsPerScreenPx,
  type CropHitSizes,
  type CropMargins,
  type CropTarget,
  type GridSize,
  type PointerDrag,
  type ScreenSize,
} from "../../utils/cropRect";

interface CropEditorProps {
  grid: GridSize | null;
  availableHeight: number;
  margins: CropMargins;
  revision: number;
  imageUrl: string | null;
  previewSize: GridSize | null;
  loading: boolean;
  previewError: string;
  onChange: (margins: CropMargins, basedOnRevision: number) => void;
}

interface DragState extends PointerDrag {
  pointerId: number;
}

const EDITOR_BASIS_PX = 320;
const MIN_WIDTH_PX = 120;
const PLACEHOLDER_WIDTH_PX = 240;
const LABEL_LINE_PX = 14;
const LABEL_GAP_PX = 4;
const HIT_SIZES: CropHitSizes = { handlePx: 9, stripPx: 8, bodyPx: 10 };
const FRAME_COLOR = "#22d3ee";
const FOCUS_COLOR = "#ffffff";
const HANDLE_FILL = "#09090b";
const DIM_FILL = "rgba(9, 9, 11, 0.62)";

function CropEditor({
  grid,
  availableHeight,
  margins,
  revision,
  imageUrl,
  previewSize,
  loading,
  previewError,
  onChange,
}: CropEditorProps) {
  const hintId = useId();
  const svgRef = useRef<SVGSVGElement>(null);
  const dragRef = useRef<DragState | null>(null);
  const releaseDragRef = useRef<(() => void) | null>(null);
  const [availableWidth, setAvailableWidth] = useState(0);
  const [activeTarget, setActiveTarget] = useState<CropTarget | null>(null);
  const [focusedTarget, setFocusedTarget] = useState<CropTarget | null>(null);
  const [failedUrl, setFailedUrl] = useState<string | null>(null);

  const aspect = previewSize ?? grid;

  const observeWrapper = useCallback((element: HTMLDivElement | null) => {
    if (!element) return;
    const update = () => setAvailableWidth(element.clientWidth);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const stopDrag = useCallback(() => {
    const release = releaseDragRef.current;
    releaseDragRef.current = null;
    dragRef.current = null;
    release?.();
    setActiveTarget(null);
  }, []);

  useEffect(() => stopDrag, [stopDrag]);

  const beginDrag = useCallback(
    (e: React.PointerEvent<SVGElement>, target: CropTarget) => {
      if (!grid || e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      stopDrag();
      const element = e.currentTarget;
      const pointerId = e.pointerId;
      element.setPointerCapture(pointerId);
      if (element.tabIndex >= 0) element.focus({ preventScroll: true });
      const restoreSelection = suppressTextSelection(document.body.style);
      const handleEnd = (ev: PointerEvent) => {
        if (ev.pointerId === pointerId) stopDrag();
      };
      window.addEventListener("pointerup", handleEnd);
      window.addEventListener("pointercancel", handleEnd);
      releaseDragRef.current = () => {
        window.removeEventListener("pointerup", handleEnd);
        window.removeEventListener("pointercancel", handleEnd);
        restoreSelection();
      };
      dragRef.current = {
        pointerId,
        revision,
        start: { target, margins, clientX: e.clientX, clientY: e.clientY },
      };
      setActiveTarget(target);
    },
    [grid, margins, revision, stopDrag],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent<SVGSVGElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== e.pointerId) return;
      if ((e.buttons & 1) === 0) {
        stopDrag();
        return;
      }
      const svg = svgRef.current;
      if (!svg || !grid) return;
      const step = pointerDragStep(drag, revision, margins, e.clientX, e.clientY, svg.getBoundingClientRect(), grid);
      dragRef.current = { pointerId: drag.pointerId, revision: step.drag.revision, start: step.drag.start };
      if (!sameMargins(step.margins, margins)) onChange(step.margins, step.drag.revision);
    },
    [grid, margins, revision, onChange, stopDrag],
  );

  const handleLostCapture = useCallback(
    (e: React.PointerEvent<SVGSVGElement>) => {
      if (dragRef.current?.pointerId === e.pointerId) stopDrag();
    },
    [stopDrag],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<SVGElement>, target: CropTarget) => {
      if (!grid) return;
      const delta = arrowDelta(e.key, e.shiftKey);
      if (!delta) return;
      e.preventDefault();
      const next = nudgeTarget(margins, target, delta.dx, delta.dy, grid);
      if (!sameMargins(next, margins)) onChange(next, revision);
    },
    [grid, margins, revision, onChange],
  );

  const area: ScreenSize = {
    width: availableWidth,
    height: availableHeight - (grid ? LABEL_LINE_PX + LABEL_GAP_PX : 0),
  };
  const naturalWidth = aspect ? editorNaturalWidth(aspect, area.height) : PLACEHOLDER_WIDTH_PX;
  const boxSize = aspect
    ? editorBoxSize(aspect, area)
    : {
        width: area.width > 0 ? Math.min(area.width, PLACEHOLDER_WIDTH_PX) : PLACEHOLDER_WIDTH_PX,
        height: editorBoxHeight(area.height),
      };

  const wrapperStyle: React.CSSProperties = {
    flex: `1 1 ${EDITOR_BASIS_PX}px`,
    minWidth: Math.min(MIN_WIDTH_PX, naturalWidth),
    maxWidth: naturalWidth,
    rowGap: LABEL_GAP_PX,
  };

  const hint = (
    <span id={hintId} className="sr-only">
      Drag the box or a handle. With a handle or the box focused, the arrow keys move it by 1 pixel and Shift with an
      arrow key by 10 pixels.
    </span>
  );

  if (!grid) {
    return (
      <div ref={observeWrapper} style={wrapperStyle} className="flex flex-col">
        <div
          style={{ width: boxSize.width, height: boxSize.height }}
          className="flex items-center justify-center gap-1.5 rounded border border-zinc-800/60 bg-zinc-950/80 px-3 text-center text-[10px] text-zinc-500"
        >
          {loading ? (
            <>
              <Loader2 size={12} className="animate-spin" />
              Loading preview...
            </>
          ) : (
            <span className={previewError ? "text-red-400" : undefined}>
              {previewError ? `Preview unavailable: ${previewError}` : "Preview unavailable"}
            </span>
          )}
        </div>
        {hint}
      </div>
    );
  }

  const rect = rectFromMargins(margins, grid);
  const hits = cropHitLayout(rect, unitsPerScreenPx(grid, boxSize), HIT_SIZES);
  const imageFailed = imageUrl !== null && failedUrl === imageUrl;
  const showImage = imageUrl !== null && !imageFailed;
  const upscaled = previewSize !== null && boxSize.width > previewSize.width;
  const dimPath = `M0 0H${grid.width}V${grid.height}H0Z M${rect.x0} ${rect.y0}H${rect.x1}V${rect.y1}H${rect.x0}Z`;
  const bodyHighlighted = focusedTarget === "body" || activeTarget === "body";
  const boxMessage = showImage
    ? ""
    : loading
      ? "Loading preview..."
      : imageFailed
        ? "Preview image failed to load"
        : previewError
          ? `Preview unavailable: ${previewError}`
          : "";

  return (
    <div ref={observeWrapper} style={wrapperStyle} className="flex flex-col">
      <div
        className="relative shrink-0 bg-zinc-950 ring-1 ring-zinc-800/70"
        style={{ width: boxSize.width, height: boxSize.height }}
      >
        {showImage && (
          <img
            src={imageUrl}
            alt="Overlay of the aligned channels"
            draggable={false}
            onError={() => setFailedUrl(imageUrl)}
            className="pointer-events-none absolute inset-0 h-full w-full select-none"
            style={{ imageRendering: upscaled ? "pixelated" : "auto" }}
          />
        )}
        {boxMessage && (
          <div
            className={`absolute inset-0 flex items-center justify-center overflow-hidden px-2 text-center text-[9px] ${
              loading ? "text-zinc-500" : "text-red-400"
            }`}
          >
            {boxMessage}
          </div>
        )}
        <svg
          ref={svgRef}
          viewBox={`0 0 ${grid.width} ${grid.height}`}
          preserveAspectRatio="none"
          className="absolute inset-0 h-full w-full select-none"
          style={{
            overflow: "visible",
            touchAction: "none",
            cursor: activeTarget ? CROP_TARGET_CURSORS[activeTarget] : undefined,
          }}
          onPointerMove={handlePointerMove}
          onLostPointerCapture={handleLostCapture}
        >
          <path d={dimPath} fill={DIM_FILL} fillRule="evenodd" pointerEvents="none" />
          <rect
            x={rect.x0}
            y={rect.y0}
            width={rect.x1 - rect.x0}
            height={rect.y1 - rect.y0}
            fill="none"
            stroke={focusedTarget === "body" ? FOCUS_COLOR : FRAME_COLOR}
            strokeWidth={bodyHighlighted ? 2 : 1.25}
            vectorEffect="non-scaling-stroke"
            pointerEvents="none"
          />
          <rect
            role="button"
            tabIndex={0}
            aria-roledescription={CROP_BODY_ROLE_DESCRIPTION}
            aria-label={CROP_TARGET_LABELS.body}
            aria-describedby={hintId}
            x={hits.body.x}
            y={hits.body.y}
            width={Math.max(0, hits.body.width)}
            height={Math.max(0, hits.body.height)}
            fill="transparent"
            style={{ cursor: CROP_TARGET_CURSORS.body, outline: "none" }}
            onPointerDown={(e) => beginDrag(e, "body")}
            onKeyDown={(e) => handleKeyDown(e, "body")}
            onFocus={() => setFocusedTarget("body")}
            onBlur={() => setFocusedTarget(null)}
          />
          {CROP_EDGE_HANDLES.map((edge) => {
            const strip = hits.strips[edge];
            return (
              <rect
                key={`strip-${edge}`}
                aria-hidden="true"
                x={strip.x}
                y={strip.y}
                width={Math.max(0, strip.width)}
                height={Math.max(0, strip.height)}
                fill="transparent"
                style={{ cursor: CROP_TARGET_CURSORS[edge] }}
                onPointerDown={(e) => beginDrag(e, edge)}
              />
            );
          })}
          {CROP_HANDLES.map((handle) => {
            const hit = hits.handles[handle];
            const highlighted = focusedTarget === handle || activeTarget === handle;
            return (
              <rect
                key={handle}
                role="button"
                tabIndex={0}
                aria-roledescription={CROP_HANDLE_ROLE_DESCRIPTION}
                aria-label={CROP_TARGET_LABELS[handle]}
                aria-describedby={hintId}
                x={hit.x}
                y={hit.y}
                width={hit.width}
                height={hit.height}
                fill={highlighted ? (focusedTarget === handle ? FOCUS_COLOR : FRAME_COLOR) : HANDLE_FILL}
                stroke={FRAME_COLOR}
                strokeWidth={1.25}
                vectorEffect="non-scaling-stroke"
                style={{ cursor: CROP_TARGET_CURSORS[handle], outline: "none" }}
                onPointerDown={(e) => beginDrag(e, handle)}
                onKeyDown={(e) => handleKeyDown(e, handle)}
                onFocus={() => setFocusedTarget(handle)}
                onBlur={() => setFocusedTarget(null)}
              />
            );
          })}
        </svg>
      </div>
      <div
        className="shrink-0 whitespace-nowrap font-mono text-[9px] text-zinc-500"
        style={{ lineHeight: `${LABEL_LINE_PX}px` }}
      >
        {cropSizeLabel(margins, grid)}
      </div>
      {hint}
    </div>
  );
}

export default memo(CropEditor);
