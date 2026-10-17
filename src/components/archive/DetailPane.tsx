import { useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type RefObject } from "react";
import { Copy, Lock, X } from "lucide-react";
import PreviewsTab from "./PreviewsTab";
import MetadataTab from "./MetadataTab";
import ProductsTab from "./ProductsTab";
import FootprintToggle from "./FootprintToggle";
import { detailMode, detailTabOf, imageSetKey, selectedRow, type ArchiveState, type DetailTab } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { detailHeader, tabLabels } from "./logic/detailView";
import { viewAvailability } from "./logic/missionState";
import { DETAIL_WIDTH_DEFAULT, DETAIL_WIDTH_MAX, DETAIL_WIDTH_MIN, keyResize, resizeWidth } from "./logic/detailLayout";

interface DetailPaneProps {
  state: ArchiveState;
  ctl: ArchiveController;
}

const TABS: readonly DetailTab[] = ["previews", "metadata", "products"];
const TAB_KEYS: Record<DetailTab, string> = { previews: "Alt+1", metadata: "Alt+2", products: "Alt+3" };
const PANEL_ID = "mast-detail-panel";

function WidthHandle({ width, paneRef, onCommit }: { width: number; paneRef: RefObject<HTMLElement | null>; onCommit: (width: number) => void }) {
  const drag = useRef<{ startX: number; startW: number; w: number } | null>(null);
  const [dragging, setDragging] = useState(false);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { startX: e.clientX, startW: width, w: width };
    setDragging(true);
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    d.w = resizeWidth(d.startW, d.startX, e.clientX);
    if (paneRef.current) paneRef.current.style.width = `${d.w}px`;
  };
  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    drag.current = null;
    setDragging(false);
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    onCommit(d.w);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const next = keyResize(width, e.key);
    if (next === null) return;
    e.preventDefault();
    onCommit(next);
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the detail pane"
      aria-valuemin={DETAIL_WIDTH_MIN}
      aria-valuemax={DETAIL_WIDTH_MAX}
      aria-valuenow={width}
      tabIndex={0}
      className="ab-resize-handle ab-mast-detail-handle"
      data-open="true"
      data-dragging={dragging ? "true" : undefined}
      title="Drag to resize, double-click to reset"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onDoubleClick={() => onCommit(DETAIL_WIDTH_DEFAULT)}
      onKeyDown={onKeyDown}
    />
  );
}

export default function DetailPane({ state, ctl }: DetailPaneProps) {
  const paneRef = useRef<HTMLElement>(null);
  const tabFocus = useRef<DetailTab | null>(null);

  useLayoutEffect(() => {
    const next = tabFocus.current;
    if (!next) return;
    tabFocus.current = null;
    paneRef.current?.querySelector<HTMLElement>(`[data-testid="mast-tab-${next}"]`)?.focus();
  });

  const row = selectedRow(state);
  const results = state.results;
  const mission = results ? state.missions.find((m) => m.id === results.mission) : undefined;
  if (!row || !mission) return null;
  const mode = detailMode(state);
  if (mode === "pane" && state.detail.hidden) return null;

  const visible = mode === "pane" || state.detail.drawerOpen;
  const header = detailHeader(row, mission.label);
  const avail = viewAvailability(mission);
  const tab = detailTabOf(state);
  const key = imageSetKey(mission.id, row.datasetId);
  const labels = tabLabels({ set: state.imageSets[key] ?? null, products: state.products[key]?.list ?? null, previewsEnabled: avail.previews });
  const enabled = TABS.filter((t) => t !== "previews" || avail.previews);

  const onTabsKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const at = enabled.indexOf(tab);
    const next = enabled[(at + (e.key === "ArrowRight" ? 1 : enabled.length - 1)) % enabled.length];
    tabFocus.current = next;
    ctl.setDetailTab(next);
  };

  return (
    <>
      {mode === "pane" ? <WidthHandle width={state.detail.width} paneRef={paneRef} onCommit={(w) => ctl.setDetailWidth(w)} /> : null}
      <aside
        ref={paneRef}
        data-testid="mast-detail"
        data-dataset={row.datasetId}
        data-mode={mode}
        data-open={visible ? "true" : "false"}
        data-mast-region={visible ? "detail" : undefined}
        role="complementary"
        aria-label={`Details for ${row.datasetId}`}
        className="ab-mast-detail"
        style={{ width: state.detail.width }}
        inert={!visible}
        tabIndex={-1}
      >
        <header className="ab-mast-detail-head">
          <div className="ab-mast-detail-idrow">
            <span className="ab-mast-detail-id ab-mast-mono" title={row.datasetId}>
              {row.datasetId}
            </span>
            <button type="button" className="ab-mast-icon-btn" aria-label="Copy dataset id" title="Copy dataset id" onClick={() => void ctl.copy(row.datasetId)}>
              <Copy size={12} aria-hidden="true" />
            </button>
            {mode === "drawer" ? (
              <button type="button" className="ab-mast-icon-btn" aria-label="Close the detail" title="Close the detail (Esc)" onClick={() => ctl.closeDrawer()}>
                <X size={13} aria-hidden="true" />
              </button>
            ) : null}
          </div>
          <div className="ab-mast-detail-badges">
            {header.badges.map((b, i) => (
              <span key={`${b}-${i}`} className="ab-mast-detail-badge">
                {b}
              </span>
            ))}
            {header.exclusive ? (
              <span className="ab-mast-detail-excl">
                <Lock size={10} aria-hidden="true" />
                {header.exclusive}
              </span>
            ) : null}
          </div>
          {header.line ? <p className="ab-mast-detail-line">{header.line}</p> : null}
          <FootprintToggle row={row} mission={mission.id} />
        </header>
        <div role="tablist" aria-label="Detail" className="ab-mast-detail-tabs" onKeyDown={onTabsKey}>
          {TABS.map((t) => {
            const disabled = t === "previews" && !avail.previews;
            const selected = t === tab;
            return (
              <button
                key={t}
                type="button"
                role="tab"
                id={`mast-tab-${t}`}
                data-testid={`mast-tab-${t}`}
                data-active={selected ? "true" : "false"}
                data-mast-focus={selected ? "true" : undefined}
                aria-selected={selected}
                aria-controls={PANEL_ID}
                aria-disabled={disabled ? true : undefined}
                title={disabled ? avail.previewsReason ?? undefined : `${labels[t]} (${TAB_KEYS[t]})`}
                tabIndex={selected ? 0 : -1}
                className="ab-mast-detail-tab"
                onClick={() => {
                  if (!disabled) ctl.setDetailTab(t);
                }}
              >
                {labels[t]}
              </button>
            );
          })}
        </div>
        {!avail.previews && avail.previewsReason ? (
          <p data-testid="mast-no-previews" className="ab-mast-detail-note ab-mast-detail-noprev">
            {avail.previewsReason}
          </p>
        ) : null}
        <div id={PANEL_ID} role="tabpanel" aria-labelledby={`mast-tab-${tab}`} className="ab-mast-detail-body">
          {tab === "previews" ? (
            <PreviewsTab state={state} ctl={ctl} row={row} mission={mission} />
          ) : tab === "metadata" ? (
            <MetadataTab state={state} ctl={ctl} row={row} mission={mission} />
          ) : (
            <ProductsTab state={state} ctl={ctl} row={row} mission={mission} />
          )}
        </div>
      </aside>
    </>
  );
}
