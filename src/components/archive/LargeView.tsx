import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { ChevronLeft, ChevronRight, ImageOff, Info, Minus, Plus, X } from "lucide-react";
import { ProgressBar, SlotStage } from "./PreviewsTab";
import { imageSetKey, largeEntry, type ArchiveState, type PreviewState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { useNow } from "./state/useNow";
import {
  EXCLUSIVE_PREVIEWS_TEXT,
  datasetPositionText,
  entriesInGroup,
  entryFilename,
  entryTitle,
  groupsOf,
  largeNeeded,
  largeTitle,
  listFailureText,
  listingText,
  previewAlt,
  previewFailureText,
  previewKey,
  previewStatusText,
  previewsAllowed,
  progressFraction,
  progressText,
  railImageLines,
  railObservationLines,
  sourceWcs,
  stripPosition,
  waitedOf,
} from "./logic/detailView";
import { largeKeyCommand } from "./logic/detailKeys";
import { isPublic } from "./logic/cardLabel";
import { displayRows } from "./logic/resultsView";
import { scaleBar } from "./logic/quickWcs";
import { missionLabelOf } from "./logic/missionLabels";
import { ZOOM_STEP, arcsecPerScreenPx, clampView, fitView, isFit, maxScale, panBy, zoomAt, zoomTo, type Size, type ZoomView } from "./logic/zoom";
import { FOCUSABLE_SELECTOR, focusTrapTarget } from "../../utils/focusTrap";
import { slotError, slotTooltip } from "./logic/errorCopy";
import type { MastError } from "../../shared/types/mast";

interface LargeViewProps {
  state: ArchiveState;
  ctl: ArchiveController;
  onFocusQuery?: () => void;
}

const SCALE_BAR_MAX_PX = 140;

function readyOf(p: PreviewState | undefined): (PreviewState & { ref: NonNullable<PreviewState["ref"]>; url: string }) | null {
  return p && p.stage === "ready" && p.ref && p.url ? (p as PreviewState & { ref: NonNullable<PreviewState["ref"]>; url: string }) : null;
}

function LargeDialog({ state, ctl, onFocusQuery }: LargeViewProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number; view: ZoomView } | null>(null);
  const wheel = useRef<(e: WheelEvent) => void>(() => {});
  const [box, setBox] = useState<Size>({ w: 960, h: 600 });
  const [railOpen, setRailOpen] = useState(() => !state.detail.narrow);
  const [zoom, setZoom] = useState<{ key: string; view: ZoomView } | null>(null);
  const [loadedUrl, setLoadedUrl] = useState<string | null>(null);

  const results = state.results;
  const mission = results ? state.missions.find((m) => m.id === results.mission) : undefined;
  const datasetId = state.large.datasetId;
  const rows = useMemo(() => displayRows(results, state.sort), [results, state.sort]);
  const index = rows.findIndex((r) => r.datasetId === datasetId);
  const row = index >= 0 ? rows[index] : null;
  const setKey = mission && datasetId ? imageSetKey(mission.id, datasetId) : null;
  const set = setKey ? state.imageSets[setKey] : undefined;
  const list = setKey ? state.lists[setKey] : undefined;
  const entry = largeEntry(state);
  const filename = entry?.previewFilename ?? null;
  const paneState = mission && datasetId && filename ? state.previews[previewKey(mission.id, datasetId, filename, "pane")] : undefined;
  const largeState = mission && datasetId && filename ? state.previews[previewKey(mission.id, datasetId, filename, "large")] : undefined;
  const pane = readyOf(paneState);
  const large = readyOf(largeState);
  const wantsLarge = !!pane && largeNeeded(pane.ref);
  const awaitingLarge = wantsLarge && largeState?.stage !== "failed";
  const main = large ?? pane;
  const source: Size | null = main ? { w: main.ref.source.width, h: main.ref.source.height } : null;
  const max = source ? maxScale(source) : 1;
  const viewKey = `${datasetId}|${entry?.key ?? ""}`;
  const view = source ? (zoom && zoom.key === viewKey ? clampView(zoom.view, source, box, max) : fitView(source, box, max)) : null;
  const busy = (!!paneState && !pane && paneState.stage !== "failed") || (!!largeState && !large && largeState.stage !== "failed") || (!set && !!list && !list.error);
  const now = useNow(busy ? 250 : null);

  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const measure = () => setBox({ w: el.clientWidth || 960, h: el.clientHeight || 600 });
    measure();
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      wheel.current(e);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    if (typeof ResizeObserver === "undefined") return () => el.removeEventListener("wheel", onWheel);
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => {
      ro.disconnect();
      el.removeEventListener("wheel", onWheel);
    };
  }, []);

  const setView = (next: ZoomView | null) => setZoom(next ? { key: viewKey, view: next } : null);
  const updateView = (fn: (current: ZoomView, img: Size) => ZoomView) => {
    if (!source) return;
    setZoom((prev) => {
      const base = prev && prev.key === viewKey ? clampView(prev.view, source, box, max) : fitView(source, box, max);
      return { key: viewKey, view: fn(base, source) };
    });
  };
  const stepZoom = (factor: number) => updateView((v, img) => zoomAt(v, factor, { x: box.w / 2, y: box.h / 2 }, img, box, max));
  wheel.current = (e: WheelEvent) => {
    const el = stageRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const point = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    updateView((v, img) => zoomAt(v, e.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP, point, img, box, max));
  };

  const close = (returnFocus = true) => ctl.closeLarge(returnFocus);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Tab") {
      const root = dialogRef.current;
      if (!root) return;
      const focusables = Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
      const target = focusTrapTarget(focusables.length, focusables.indexOf(document.activeElement as HTMLElement), e.shiftKey);
      if (target === null) return;
      e.preventDefault();
      focusables[target].focus();
      return;
    }
    const cmd = largeKeyCommand(e);
    if (!cmd) return;
    e.preventDefault();
    e.stopPropagation();
    switch (cmd) {
      case "close":
        close();
        return;
      case "focus-query":
        close(false);
        onFocusQuery?.();
        return;
      case "prev-image":
      case "next-image":
        ctl.largeStepImage(cmd === "next-image" ? 1 : -1);
        return;
      case "prev-dataset":
      case "next-dataset":
        ctl.largeStepDataset(cmd === "next-dataset" ? 1 : -1);
        return;
      case "next-group":
        ctl.cycleGroup();
        return;
      case "zoom-in":
        stepZoom(ZOOM_STEP);
        return;
      case "zoom-out":
        stepZoom(1 / ZOOM_STEP);
        return;
      case "zoom-fit":
        setView(null);
        return;
      case "zoom-actual":
        updateView((v, img) => zoomTo(v, max, img, box, max));
        return;
      case "toggle-rail":
        setRailOpen((o) => !o);
        return;
      case "retry":
        ctl.retryLarge();
        return;
    }
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !view || !source) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { x: e.clientX, y: e.clientY, view };
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || !source) return;
    setView(panBy(d.view, e.clientX - d.x, e.clientY - d.y, source, box, max));
  };
  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    drag.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
  };

  const label = mission?.label ?? (results ? missionLabelOf(results.mission) : "MAST");
  const allowed = !!row && !!mission && previewsAllowed(row, mission);
  const imgStyle = view && source ? { left: view.x, top: view.y, width: source.w * view.scale, height: source.h * view.scale } : undefined;
  const wcs = main ? sourceWcs(main.ref) : null;
  const bar = wcs && view ? scaleBar(arcsecPerScreenPx(wcs.pixelScaleArcsec, view.scale), SCALE_BAR_MAX_PX) : null;
  const groups = set ? groupsOf(set) : [];
  const strip = set && entry ? entriesInGroup(set, entry.group) : [];
  const pos = set && entry ? stripPosition(set, entry) : null;
  const showBackdrop = !!pane && (!large || loadedUrl !== large.url);

  const rowPublic = row ? isPublic(row) : true;
  const failureView = (error: MastError, surface: "list" | "preview", text: string, waitedMs: number | null): ReactNode => {
    const view = slotError(error, { surface, rowPublic, waitedMs });
    return view ? (
      <SlotStage ctl={ctl} view={view} text={text} onRetry={() => ctl.retryLarge()} />
    ) : (
      <>
        <p>{text}</p>
        <RetryLarge ctl={ctl} />
      </>
    );
  };
  let status: ReactNode = null;
  let failure: MastError | null = null;
  if (!allowed) status = <p>{EXCLUSIVE_PREVIEWS_TEXT}</p>;
  else if (!set) {
    if (list?.error) {
      failure = list.error;
      status = failureView(list.error, "list", listFailureText(list.error), waitedOf(list));
    } else status = <p>{listingText(datasetId ?? "", list ? Math.max(0, now - list.startedAt) : 0)}</p>;
  } else if (!entry) status = <p>MAST lists no preview images for this dataset.</p>;
  else if (!entry.previewFilename) status = <p>MAST lists no preview for this image.</p>;
  else if (paneState?.stage === "failed" && paneState.error) {
    failure = paneState.error;
    status = failureView(paneState.error, "preview", previewFailureText(paneState.error, rowPublic), waitedOf(paneState));
  } else if (!pane) {
    const phase = paneState ? paneState.stage : "pending";
    status = (
      <>
        <p>{previewStatusText(phase, entry, "pane")}</p>
        {paneState && phase === "download" ? <p className="ab-mast-mono">{progressText(paneState, now)}</p> : null}
        {paneState ? <ProgressBar fraction={phase === "decode" ? 1 : progressFraction(paneState)} /> : null}
      </>
    );
  }

  let chip: ReactNode = null;
  let chipTitle: string | undefined;
  if (pane && wantsLarge && !large) {
    const largeView = largeState?.stage === "failed" && largeState.error ? slotError(largeState.error, { surface: "preview", rowPublic, waitedMs: waitedOf(largeState) }) : null;
    if (largeState?.stage === "failed" && largeState.error) {
      chipTitle = largeView ? slotTooltip(largeView) : undefined;
      const said = largeView ? [largeView.meta, largeView.detail].filter(Boolean).join(" · ") || largeView.title : previewFailureText(largeState.error, rowPublic);
      chip = (
        <>
          <span>{`4096 px copy failed: ${said}`}</span>
          <RetryLarge ctl={ctl} />
        </>
      );
    } else {
      const detail = largeState && largeState.stage === "download" && largeState.received > 0 ? ` · ${progressText(largeState, now)}` : "";
      chip = <span>{`Loading the 4096 px copy${largeState?.stage === "decode" ? " · preparing" : detail}`}</span>;
    }
  }

  return (
    <div
      ref={dialogRef}
      data-testid="mast-lightbox"
      data-dataset={datasetId ?? undefined}
      data-filename={entry ? entryFilename(entry) : undefined}
      role="dialog"
      aria-modal="true"
      aria-label={`Large view of ${entry ? entryTitle(entry) : datasetId ?? "the dataset"}`}
      tabIndex={-1}
      className="ab-mast-large"
      onKeyDown={onKeyDown}
    >
      <header className="ab-mast-large-head">
        <button type="button" className="ab-mast-link-btn" onClick={() => close()}>
          <ChevronLeft size={13} aria-hidden="true" />
          Results
        </button>
        <span className="ab-mast-large-title" title={datasetId ?? undefined}>
          {row ? largeTitle(row, label) : datasetId}
        </span>
        {index >= 0 ? <span className="ab-mast-large-pos ab-mast-mono">{`${datasetPositionText(index, rows.length)} (Up/Down)`}</span> : null}
        <button type="button" className="ab-mast-icon-btn" aria-label="Show or hide the info rail (I)" aria-pressed={railOpen} title="Info rail (I)" onClick={() => setRailOpen((o) => !o)}>
          <Info size={13} aria-hidden="true" />
        </button>
        <button type="button" className="ab-mast-icon-btn" aria-label="Close the Large view (Esc)" title="Close (Esc)" onClick={() => close()}>
          <X size={13} aria-hidden="true" />
        </button>
      </header>
      <div className="ab-mast-large-main">
        <div
          ref={stageRef}
          className="ab-mast-large-stage"
          data-zoomed={view && source && !isFit(view, source, box, max) ? "true" : "false"}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        >
          {large && entry ? (
            <img
              className="ab-mast-large-img"
              src={large.url}
              alt={previewAlt(entry, large.ref)}
              draggable={false}
              decoding="async"
              style={imgStyle}
              onLoad={() => setLoadedUrl(large.url)}
              onError={() => ctl.imageMissing(large.url)}
            />
          ) : null}
          {pane && entry && showBackdrop ? (
            <img
              className={awaitingLarge ? "ab-mast-large-img ab-mast-large-blur" : "ab-mast-large-img"}
              src={pane.url}
              alt={large ? "" : previewAlt(entry, pane.ref)}
              aria-hidden={large ? true : undefined}
              draggable={false}
              decoding="async"
              style={imgStyle}
              onError={() => ctl.imageMissing(pane.url)}
            />
          ) : null}
          {status ? (
            <div
              className="ab-mast-large-status"
              role={failure ? "alert" : "status"}
              data-error-code={failure?.code}
              data-error-status={failure?.context.status}
            >
              {!allowed || (paneState?.stage === "failed" && !failure) ? <ImageOff size={20} aria-hidden="true" /> : null}
              {status}
            </div>
          ) : null}
          {chip ? (
            <div className="ab-mast-large-chip" role="status" title={chipTitle}>
              {chip}
            </div>
          ) : null}
          {bar ? (
            <div className="ab-mast-scalebar" aria-label={`Scale bar ${bar.label}`}>
              <span className="ab-mast-scalebar-line" style={{ width: bar.px }} />
              <span className="ab-mast-mono">{bar.label}</span>
            </div>
          ) : null}
          <div className="ab-mast-zoom" role="group" aria-label="Zoom">
            <button type="button" className="ab-mast-zoom-btn" aria-label="Fit to the window" title="Fit (0)" disabled={!view} onClick={() => setView(null)}>
              fit
            </button>
            <button type="button" className="ab-mast-zoom-btn" aria-label="Zoom out" title="Zoom out (-)" disabled={!view} onClick={() => stepZoom(1 / ZOOM_STEP)}>
              <Minus size={12} aria-hidden="true" />
            </button>
            <button type="button" className="ab-mast-zoom-btn" aria-label="Zoom in" title="Zoom in (+)" disabled={!view} onClick={() => stepZoom(ZOOM_STEP)}>
              <Plus size={12} aria-hidden="true" />
            </button>
            <button
              type="button"
              className="ab-mast-zoom-btn"
              aria-label="Actual size of the 4096 px copy"
              title="1:1 (1)"
              disabled={!view || !source}
              onClick={() => updateView((v, img) => zoomTo(v, max, img, box, max))}
            >
              1:1
            </button>
          </div>
        </div>
        {railOpen ? (
          <aside className="ab-mast-large-rail" aria-label="Image information">
            <h3 className="ab-mast-meta-title">Image</h3>
            {entry && main ? railImageLines(entry, main.ref).map((line) => <p key={line}>{line}</p>) : entry ? <p>{entryTitle(entry)}</p> : null}
            <h3 className="ab-mast-meta-title">Observation</h3>
            {row ? railObservationLines(row).map((line) => <p key={line}>{line}</p>) : null}
          </aside>
        ) : null}
      </div>
      <footer className="ab-mast-large-foot">
        {set && entry && groups.length > 1 ? (
          <div className="ab-mast-segment" role="group" aria-label="Product group">
            {groups.map((g) => (
              <button
                key={g.key}
                type="button"
                className="ab-mast-seg"
                data-active={g.key === entry.group ? "true" : "false"}
                aria-pressed={g.key === entry.group}
                title="Next group (G)"
                onClick={() => {
                  const first = entriesInGroup(set, g.key)[0];
                  if (first) ctl.largeSelectImage(first.key);
                }}
              >
                {g.label}
              </button>
            ))}
          </div>
        ) : null}
        <div className="ab-mast-large-entries" role="group" aria-label="Images in this group">
          {strip.map((e) => (
            <button key={e.key} type="button" className="ab-mast-seg" data-active={entry && e.key === entry.key ? "true" : "false"} aria-pressed={!!entry && e.key === entry.key} onClick={() => ctl.largeSelectImage(e.key)}>
              {e.label}
            </button>
          ))}
        </div>
        {pos ? (
          <div className="ab-mast-large-step">
            <button type="button" className="ab-mast-icon-btn" aria-label="Previous image (Left)" disabled={pos.index <= 1} onClick={() => ctl.largeStepImage(-1)}>
              <ChevronLeft size={13} aria-hidden="true" />
            </button>
            <span className="ab-mast-mono">{`${pos.index}/${pos.count}`}</span>
            <button type="button" className="ab-mast-icon-btn" aria-label="Next image (Right)" disabled={pos.index >= pos.count} onClick={() => ctl.largeStepImage(1)}>
              <ChevronRight size={13} aria-hidden="true" />
            </button>
          </div>
        ) : null}
      </footer>
    </div>
  );
}

function RetryLarge({ ctl }: { ctl: ArchiveController }) {
  return (
    <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.retryLarge()}>
      Retry
      <kbd className="ab-cmdp-kbd">R</kbd>
    </button>
  );
}

export default function LargeView(props: LargeViewProps) {
  if (!props.state.large.open) return null;
  return <LargeDialog {...props} />;
}
