import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type FocusEvent, type KeyboardEvent } from "react";
import DatasetCard from "./DatasetCard";
import type { MastRow } from "../../shared/types/mast";
import type { ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { galleryColumns, galleryMove } from "./logic/gridNav";
import { thumbStatus } from "./logic/thumbQueue";
import { resultsKeyCommand } from "./logic/detailKeys";

interface ResultsGalleryProps {
  state: ArchiveState;
  ctl: ArchiveController;
  rows: MastRow[];
  missionLabel: string;
  onOpenDetail?: () => void;
}

const CARD_W = 152;
const CARD_H = 196;
const GAP = 12;
const PAD = 12;
const ROW_H = CARD_H + GAP;
const OVERSCAN_ROWS = 1;
const MAX_MOUNTED = 200;

function cardSelector(id: string): string {
  return `[data-testid="mast-card"][data-dataset="${CSS.escape(id)}"]`;
}

export default function ResultsGallery({ state, ctl, rows, missionLabel, onOpenDetail }: ResultsGalleryProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ width: 960, height: 640 });
  const [scrollTop, setScrollTop] = useState(0);
  const pendingFocus = useRef<string | null>(null);
  const frame = useRef<number | null>(null);
  const handledFocus = useRef(state.focusRequest?.seq ?? 0);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(([entry]) => setBox({ width: entry.contentRect.width, height: entry.contentRect.height }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(
    () => () => {
      if (frame.current !== null) cancelAnimationFrame(frame.current);
    },
    [],
  );

  const columns = galleryColumns(Math.max(0, box.width - PAD * 2), CARD_W, GAP);
  const rowCount = Math.ceil(rows.length / columns);
  const pageRows = Math.max(1, Math.floor(box.height / ROW_H));
  const firstVisible = Math.max(0, Math.floor((scrollTop - PAD) / ROW_H));
  const lastVisible = Math.min(rowCount, Math.ceil((scrollTop + box.height - PAD) / ROW_H));
  const start = Math.max(0, firstVisible - OVERSCAN_ROWS);
  const end = Math.min(rowCount, lastVisible + OVERSCAN_ROWS, start + Math.max(1, Math.floor(MAX_MOUNTED / columns)));
  const selectedIndex = rows.findIndex((r) => r.datasetId === state.selectedId);
  const rovingIndex = selectedIndex >= 0 ? selectedIndex : 0;
  const rovingMounted = rovingIndex >= start * columns && rovingIndex < end * columns;

  const demand = useMemo(() => {
    const visible = rows.slice(firstVisible * columns, lastVisible * columns).map((r) => r.datasetId);
    const ahead = rows.slice(lastVisible * columns, (lastVisible + pageRows) * columns).map((r) => r.datasetId);
    const selected = state.selectedId && rows.some((r) => r.datasetId === state.selectedId) ? [state.selectedId] : [];
    return { selected, visible, ahead };
  }, [rows, columns, firstVisible, lastVisible, pageRows, state.selectedId]);

  useEffect(() => {
    ctl.setThumbDemand(demand);
  }, [ctl, demand]);

  const hasMore = !!state.results?.hasMore;
  const idle = state.loadingMore === null && !state.run;
  useEffect(() => {
    if (hasMore && idle && rowCount > 0 && lastVisible >= rowCount - 1) void ctl.loadMore({ auto: true });
  }, [ctl, hasMore, idle, rowCount, lastVisible]);

  useLayoutEffect(() => {
    const id = pendingFocus.current;
    if (!id) return;
    const el = scrollRef.current?.querySelector<HTMLElement>(cardSelector(id));
    if (!el) return;
    pendingFocus.current = null;
    el.focus({ preventScroll: true });
  });

  const onScroll = useCallback(() => {
    if (frame.current !== null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      if (scrollRef.current) setScrollTop(scrollRef.current.scrollTop);
    });
  }, []);

  const ensureVisible = useCallback(
    (index: number) => {
      const el = scrollRef.current;
      if (!el) return;
      const top = PAD + Math.floor(index / columns) * ROW_H;
      if (top < el.scrollTop) el.scrollTop = Math.max(0, top - PAD);
      else if (top + CARD_H > el.scrollTop + el.clientHeight) el.scrollTop = top + CARD_H + PAD - el.clientHeight;
      setScrollTop(el.scrollTop);
    },
    [columns],
  );

  const onSelect = useCallback((datasetId: string) => ctl.select(datasetId), [ctl]);
  const onOpen = useCallback((datasetId: string) => ctl.openLarge(datasetId), [ctl]);

  useEffect(() => {
    const req = state.focusRequest;
    if (!req || req.seq === handledFocus.current) return;
    handledFocus.current = req.seq;
    const index = rows.findIndex((r) => r.datasetId === req.datasetId);
    if (index < 0) return;
    pendingFocus.current = req.datasetId;
    ensureVisible(index);
    const el = scrollRef.current?.querySelector<HTMLElement>(cardSelector(req.datasetId));
    if (!el) return;
    pendingFocus.current = null;
    el.focus({ preventScroll: true });
  }, [state.focusRequest, rows, ensureVisible]);

  const onGridFocus = (e: FocusEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget || rows.length === 0) return;
    const target = rows[rovingIndex];
    pendingFocus.current = target.datasetId;
    ensureVisible(rovingIndex);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const current = rows[rovingIndex];
    if (!current) return;
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "c") {
      e.preventDefault();
      void ctl.copy(current.datasetId);
      return;
    }
    if (plain && e.key.toLowerCase() === "r" && state.thumbs.done[current.datasetId] === "failed") {
      e.preventDefault();
      ctl.retryThumbs([current.datasetId]);
      return;
    }
    const open = resultsKeyCommand(e);
    if (open) {
      e.preventDefault();
      if (open === "open-detail") onOpenDetail?.();
      else ctl.openLarge(current.datasetId);
      return;
    }
    const move = galleryMove(rovingIndex, e, { count: rows.length, columns, pageRows });
    if (!move) return;
    e.preventDefault();
    const target = rows[move.index];
    pendingFocus.current = target.datasetId;
    if (target.datasetId !== state.selectedId) ctl.select(target.datasetId);
    ensureVisible(move.index);
    if (move.atEnd) void ctl.loadMore({ auto: true });
  };

  const now = Date.now();
  const rowIndexes = Array.from({ length: Math.max(0, end - start) }, (_, i) => start + i);
  const results = state.results;
  return (
    <div ref={scrollRef} className="ab-mast-scroll" onScroll={onScroll}>
      <div
        role="grid"
        aria-label={`${missionLabel} results`}
        aria-rowcount={rowCount}
        aria-colcount={columns}
        data-loaded={rows.length}
        tabIndex={rovingMounted ? -1 : 0}
        className="ab-mast-grid"
        style={{ height: rowCount * ROW_H + PAD * 2 - GAP }}
        onKeyDown={onKeyDown}
        onFocus={onGridFocus}
      >
        {rowIndexes.map((r) => (
          <div key={r} role="row" aria-rowindex={r + 1} className="ab-mast-grid-row" style={{ top: PAD + r * ROW_H, gridTemplateColumns: `repeat(${columns}, ${CARD_W}px)` }}>
            {rows.slice(r * columns, r * columns + columns).map((row, c) => {
              const index = r * columns + c;
              const id = row.datasetId;
              return (
                <DatasetCard
                  key={id}
                  row={row}
                  missionLabel={missionLabel}
                  status={thumbStatus(state.thumbs, id, now)}
                  thumb={state.thumbRefs[id]}
                  reason={state.thumbs.reasons[id]}
                  selected={id === state.selectedId}
                  tabbable={index === rovingIndex}
                  onPick={onSelect}
                  onFocusCard={onSelect}
                  onOpen={onOpen}
                />
              );
            })}
          </div>
        ))}
      </div>
      {state.loadingMore !== null ? (
        <p className="ab-mast-more">{`Loading rows ${state.loadingMore + 1} to ${state.loadingMore + 200}`}</p>
      ) : results?.truncated ? (
        <p className="ab-mast-more">Showing the first 2,000 rows. Narrow the search to see the rest.</p>
      ) : null}
    </div>
  );
}
