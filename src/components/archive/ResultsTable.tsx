import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type FocusEvent, type KeyboardEvent } from "react";
import { ChevronDown, ChevronUp, Lock } from "lucide-react";
import type { MastColumn, MastMission, MastRow } from "../../shared/types/mast";
import type { ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { tableMove } from "./logic/gridNav";
import { thumbStatus } from "./logic/thumbQueue";
import { SORT_BLOCKED_HINT, sortMode } from "./logic/paging";
import { UNKNOWN_TEXT, cellText, columnWidth, fitColumns, isUnknownCell, rovingRow, rowKeyMap, rowsText, sortText, tableColumns } from "./logic/resultsView";
import { rowTsv } from "./logic/copyBuilders";
import { resultsKeyCommand } from "./logic/detailKeys";
import { previewsAllowed } from "./logic/detailView";
import { isPublic } from "./logic/cardLabel";
import { pausedSlot, slotCellTitle, slotError, type SlotErrorView } from "./logic/errorCopy";
import SlotError from "./SlotError";

interface ResultsTableProps {
  state: ArchiveState;
  ctl: ArchiveController;
  rows: MastRow[];
  mission: MastMission | undefined;
  missionLabel: string;
  onOpenDetail?: () => void;
}

const ROW_H = 24;
const ROW_H_THUMBS = 44;
const HEAD_H = 26;
const THUMB_COL = 72;
const OVERSCAN = 8;
const MAX_MOUNTED = 200;
const SORT_HINT_ID = "mast-sort-hint";
const FIRST_BODY_ROW = 2;

function rowAt(index: number): string {
  return `[data-testid="mast-row"][aria-rowindex="${index + FIRST_BODY_ROW}"]`;
}

export default function ResultsTable({ state, ctl, rows, mission, missionLabel, onOpenDetail }: ResultsTableProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ width: 960, height: 640 });
  const [scrollTop, setScrollTop] = useState(0);
  const [cell, setCell] = useState<number | null>(null);
  const pendingFocus = useRef<number | null>(null);
  const frame = useRef<number | null>(null);
  const handledFocus = useRef(state.focusRequest?.seq ?? 0);
  const results = state.results;
  const images = !!mission?.capabilities.images;
  const thumbs = state.tableThumbs && images;
  const rowH = thumbs ? ROW_H_THUMBS : ROW_H;

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

  const allColumns = useMemo(() => (results ? tableColumns(mission, results) : []), [mission, results]);
  const columns = useMemo(() => fitColumns(allColumns, box.width - (thumbs ? THUMB_COL : 0)), [allColumns, box.width, thumbs]);
  const bodyHeight = Math.max(rowH, box.height - HEAD_H);
  const pageRows = Math.max(1, Math.floor(bodyHeight / rowH));
  const first = Math.min(rows.length, Math.floor(scrollTop / rowH));
  const last = Math.min(rows.length, Math.ceil((scrollTop + bodyHeight) / rowH));
  const start = Math.max(0, first - OVERSCAN);
  const end = Math.min(rows.length, last + OVERSCAN, start + MAX_MOUNTED);
  const keyOf = useMemo(() => rowKeyMap(results?.rows ?? rows), [results, rows]);
  const keys = useMemo(() => rows.map((r) => keyOf.get(r) ?? r.datasetId), [rows, keyOf]);
  const rovingIndex = rovingRow(rows, keyOf, state.selectedId, state.selectedKey);
  const rovingMounted = rovingIndex >= start && rovingIndex < end;

  const demand = useMemo(() => {
    if (!thumbs) return { selected: [], visible: [], ahead: [] };
    return {
      selected: state.selectedId && rows.some((r) => r.datasetId === state.selectedId) ? [state.selectedId] : [],
      visible: rows.slice(first, last).map((r) => r.datasetId),
      ahead: rows.slice(last, last + pageRows).map((r) => r.datasetId),
    };
  }, [thumbs, rows, first, last, pageRows, state.selectedId]);

  useEffect(() => {
    ctl.setThumbDemand(demand);
  }, [ctl, demand]);

  const hasMore = !!results?.hasMore;
  const idle = state.loadingMore === null && !state.run;
  useEffect(() => {
    if (hasMore && idle && rows.length > 0 && last >= rows.length) void ctl.loadMore({ auto: true });
  }, [ctl, hasMore, idle, rows.length, last]);

  useLayoutEffect(() => {
    const index = pendingFocus.current;
    if (index === null) return;
    const el = scrollRef.current?.querySelector<HTMLElement>(rowAt(index));
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
      const top = index * rowH;
      if (top < el.scrollTop) el.scrollTop = top;
      else if (HEAD_H + top + rowH > el.scrollTop + el.clientHeight) el.scrollTop = HEAD_H + top + rowH - el.clientHeight;
      setScrollTop(el.scrollTop);
    },
    [rowH],
  );

  useEffect(() => {
    const req = state.focusRequest;
    if (!req || req.seq === handledFocus.current) return;
    handledFocus.current = req.seq;
    if (!rows.some((r) => r.datasetId === req.datasetId)) return;
    const index = rovingRow(rows, keyOf, req.datasetId, state.selectedKey);
    pendingFocus.current = index;
    ensureVisible(index);
    const el = scrollRef.current?.querySelector<HTMLElement>(rowAt(index));
    if (!el) return;
    pendingFocus.current = null;
    el.focus({ preventScroll: true });
  }, [state.focusRequest, state.selectedKey, rows, keyOf, ensureVisible]);

  if (!results) return null;

  const copyCurrent = (row: MastRow, shift: boolean) => {
    if (shift) {
      void ctl.copy(rowTsv(row, allColumns), "the row as TSV");
      return;
    }
    const column = cell !== null ? columns[cell] : undefined;
    if (column) {
      const text = cellText(row.values[column.name], column);
      void ctl.copy(text, text || "an empty cell");
      return;
    }
    void ctl.copy(row.datasetId);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if ((e.target as HTMLElement).closest('[role="columnheader"]')) return;
    const current = rows[rovingIndex];
    if (!current) return;
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "c") {
      e.preventDefault();
      copyCurrent(current, e.shiftKey);
      return;
    }
    if (plain && !e.shiftKey && e.key.toLowerCase() === "t") {
      e.preventDefault();
      if (images) ctl.toggleTableThumbs();
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
      else if (images) ctl.openLarge(current.datasetId);
      return;
    }
    if (plain && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      e.preventDefault();
      const max = columns.length - 1;
      setCell((c) => (c === null ? (e.key === "ArrowRight" ? 0 : max) : Math.max(0, Math.min(max, c + (e.key === "ArrowRight" ? 1 : -1)))));
      return;
    }
    const move = tableMove(rovingIndex, e, { count: rows.length, pageRows });
    if (!move) return;
    e.preventDefault();
    const target = rows[move.index];
    pendingFocus.current = move.index;
    if (move.index !== rovingIndex) ctl.select(target.datasetId, keys[move.index]);
    ensureVisible(move.index);
    if (move.atEnd) void ctl.loadMore({ auto: true });
  };

  const onGridFocus = (e: FocusEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget || rows.length === 0) return;
    pendingFocus.current = rovingIndex;
    ensureVisible(rovingIndex);
  };

  const sortKeyOf = (c: MastColumn) => state.sort.keys.find((k) => k.column === c.name);
  const now = Date.now();
  const paused = thumbs && state.thumbs.pausedUntil !== null ? pausedSlot(state.thumbs, now) : null;
  const failureOf = (row: MastRow, status: string): SlotErrorView | null => {
    const failure = status === "failed" ? state.thumbs.failures[row.datasetId] : undefined;
    return failure ? slotError(failure.error, { surface: "thumb", rowPublic: isPublic(row), waitedMs: failure.waitedMs }) : null;
  };
  const sortLine = sortText(state.sort, allColumns, results);
  const lastShown = Math.max(first + 1, last);

  return (
    <div className="ab-mast-table-wrap">
      <div className="ab-mast-table-bar">
        <label className="ab-mast-check" title={images ? "Show thumbnails in the table (T)" : `MAST's ${missionLabel} search API serves no thumbnails or previews.`}>
          <input type="checkbox" checked={thumbs} disabled={!images} onChange={() => ctl.toggleTableThumbs()} />
          Thumbnails
          <kbd className="ab-cmdp-kbd">T</kbd>
        </label>
      </div>
      <div ref={scrollRef} className="ab-mast-scroll" onScroll={onScroll}>
        <div
          role="grid"
          aria-label={`${missionLabel} results table`}
          aria-rowcount={rows.length + 1}
          aria-colcount={columns.length + (thumbs ? 1 : 0)}
          data-loaded={rows.length}
          tabIndex={rovingMounted ? -1 : 0}
          className="ab-mast-table"
          onKeyDown={onKeyDown}
          onFocus={onGridFocus}
        >
          <div role="row" aria-rowindex={1} className="ab-mast-tr ab-mast-thead" style={{ height: HEAD_H }}>
            {thumbs ? (
              <div role="columnheader" className="ab-mast-th" style={{ width: THUMB_COL }}>
                Thumb
              </div>
            ) : null}
            {columns.map((c, ci) => {
              const sortable = sortMode(results, c) !== "none";
              const key = sortKeyOf(c);
              const ariaSort = !sortable ? undefined : key ? (key.desc ? "descending" : "ascending") : "none";
              return (
                <div
                  key={c.name}
                  role="columnheader"
                  aria-sort={ariaSort}
                  data-kind={c.kind}
                  className="ab-mast-th"
                  style={{ width: columnWidth(c, ci === 0) }}
                  title={sortable ? `Sort by ${c.label} (Shift adds a second key)` : SORT_BLOCKED_HINT}
                >
                  <button
                    type="button"
                    className="ab-mast-th-btn"
                    aria-disabled={!sortable}
                    aria-describedby={sortable ? undefined : SORT_HINT_ID}
                    onClick={(e) => ctl.sortBy(c, e.shiftKey)}
                    onKeyDown={(e) => {
                      if (e.key !== "Enter" && e.key !== " ") return;
                      e.preventDefault();
                      ctl.sortBy(c, e.shiftKey);
                    }}
                  >
                    <span className="ab-mast-th-label">{c.label}</span>
                    {key ? key.desc ? <ChevronDown size={10} aria-hidden="true" /> : <ChevronUp size={10} aria-hidden="true" /> : null}
                  </button>
                </div>
              );
            })}
          </div>
          <div className="ab-mast-tbody" style={{ height: rows.length * rowH }}>
            {rows.slice(start, end).map((row, offset) => {
              const index = start + offset;
              const id = row.datasetId;
              const selected = index === rovingIndex && id === state.selectedId;
              const status = thumbStatus(state.thumbs, id, now);
              const ref = state.thumbRefs[id];
              const locked = !!mission && images && !previewsAllowed(row, mission);
              const failed = thumbs && !locked ? failureOf(row, status) : null;
              const slot = failed ?? (thumbs && !locked && status === "paused" ? paused : null);
              return (
                <div
                  key={keys[index]}
                  role="row"
                  aria-rowindex={index + FIRST_BODY_ROW}
                  data-testid="mast-row"
                  data-dataset={id}
                  data-thumb={locked ? "locked" : status}
                  data-mast-focus={index === rovingIndex ? "true" : undefined}
                  aria-selected={selected}
                  tabIndex={index === rovingIndex ? 0 : -1}
                  className="ab-mast-tr"
                  style={{ top: index * rowH, height: rowH }}
                  onClick={() => ctl.select(id, keys[index])}
                  onDoubleClick={() => ctl.openLarge(id)}
                  onFocus={() => ctl.select(id, keys[index])}
                >
                  {thumbs ? (
                    <div
                      role="gridcell"
                      className="ab-mast-td ab-mast-td-thumb"
                      style={{ width: THUMB_COL }}
                      data-error-code={failed ? failed.code : undefined}
                      data-error-status={failed ? state.thumbs.failures[id]?.error.context.status : undefined}
                      data-error-waited-ms={failed ? (state.thumbs.failures[id]?.waitedMs ?? undefined) : undefined}
                      title={slot ? slotCellTitle(slot) : undefined}
                    >
                      {locked ? (
                        <span className="ab-mast-thumb-note" title="Exclusive access">
                          <Lock size={12} aria-hidden="true" />
                        </span>
                      ) : status === "ready" && ref ? (
                        <img className="ab-mast-row-thumb" src={ref.url} alt="" draggable={false} decoding="async" onError={() => ctl.imageMissing(ref.url)} />
                      ) : slot ? (
                        <SlotError view={slot} size="cell" noun="Thumbnail" />
                      ) : (
                        <span className={status === "loading" ? "ab-mast-row-thumb ab-mast-shimmer" : "ab-mast-row-thumb ab-mast-queued"} aria-hidden="true" />
                      )}
                    </div>
                  ) : null}
                  {columns.map((c, ci) => {
                    const value = row.values[c.name];
                    const unknown = isUnknownCell(c, value);
                    const text = cellText(value, c);
                    return (
                      <div
                        key={c.name}
                        role="gridcell"
                        data-kind={c.kind}
                        data-cell-focus={selected && cell === ci ? "true" : undefined}
                        className="ab-mast-td"
                        style={{ width: columnWidth(c, ci === 0) }}
                        title={unknown ? UNKNOWN_TEXT : text}
                      >
                        {unknown ? <span className="ab-mast-td-unknown">{UNKNOWN_TEXT}</span> : text}
                      </div>
                    );
                  })}
                </div>
              );
            })}
          </div>
        </div>
        <span id={SORT_HINT_ID} className="sr-only">
          {SORT_BLOCKED_HINT}
        </span>
      </div>
      <div className="ab-mast-table-foot">
        <span>{sortLine || (state.loadingMore !== null ? `Loading rows ${state.loadingMore + 1} to ${state.loadingMore + 200}` : "")}</span>
        <span className="ab-mast-mono">{rowsText(rows.length ? first + 1 : 0, rows.length ? lastShown : 0, results)}</span>
      </div>
    </div>
  );
}
