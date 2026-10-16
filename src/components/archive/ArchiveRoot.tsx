import "./archive.css";
import { useCallback, useEffect, useLayoutEffect, useRef } from "react";
import QueryBar from "./QueryBar";
import ResultsRegion from "./ResultsRegion";
import DetailPane from "./DetailPane";
import LargeView from "./LargeView";
import ArchiveToast from "./ArchiveToast";
import ArchiveLiveRegion from "./ArchiveLiveRegion";
import { archiveStore, useArchiveState } from "./state/archiveStore";
import { archiveController } from "./state/archiveController";
import { selectedRow } from "./state/archiveReducer";
import { useAroundTarget, type WorkspaceFileRef } from "./state/useAroundTarget";
import { archiveKeyCommand, isTextEntry } from "./logic/archiveKeys";
import { innermostLayer } from "./logic/escLayers";
import { isNarrow } from "./logic/detailLayout";
import { FOCUSABLE_SELECTOR } from "../../utils/focusTrap";

interface ArchiveRootProps {
  active: boolean;
  workspaceFile: WorkspaceFileRef | null;
  onSwitchToWorkspace: () => void;
}

const FOCUS_MARK = '[data-mast-focus="true"]';
const DETAIL_REGION = '[data-mast-region="detail"]';

export default function ArchiveRoot({ active, workspaceFile, onSwitchToWorkspace }: ArchiveRootProps) {
  const state = useArchiveState();
  const ctl = archiveController;
  const around = useAroundTarget(workspaceFile);
  const rootRef = useRef<HTMLDivElement>(null);
  const queryRef = useRef<HTMLInputElement>(null);
  const focusedOnce = useRef(false);
  const refocusResults = useRef(false);
  const focusDetailPending = useRef(false);

  useEffect(() => {
    void ctl.init();
  }, [ctl]);

  useEffect(() => {
    ctl.setActive(active);
  }, [ctl, active]);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const apply = (width: number) => ctl.setNarrow(isNarrow(width));
    apply(el.clientWidth);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(([entry]) => apply(entry.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, [ctl]);

  useEffect(() => {
    if (!active || focusedOnce.current) return;
    focusedOnce.current = true;
    queryRef.current?.focus();
  }, [active]);

  const focusQuery = useCallback(() => {
    const el = queryRef.current;
    if (!el) return;
    el.focus();
    el.select();
  }, []);

  const focusRegion = useCallback((region: HTMLElement) => {
    const target = region.querySelector<HTMLElement>(FOCUS_MARK) ?? region.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
    if (target) target.focus();
    else region.focus();
  }, []);

  const focusResults = useCallback(() => {
    const region = rootRef.current?.querySelector<HTMLElement>('[data-mast-region="results"]');
    if (region) focusRegion(region);
  }, [focusRegion]);

  const focusDetail = useCallback(() => {
    const region = rootRef.current?.querySelector<HTMLElement>(DETAIL_REGION);
    if (!region) return false;
    focusRegion(region);
    return true;
  }, [focusRegion]);

  const openDetail = useCallback(() => {
    const s = archiveStore.getState();
    if (!selectedRow(s)) return;
    if (s.detail.narrow) ctl.openDrawer();
    else if (s.detail.hidden) ctl.toggleDetail();
    focusDetailPending.current = true;
    if (focusDetail()) focusDetailPending.current = false;
  }, [ctl, focusDetail]);

  const cycleRegion = useCallback(
    (dir: 1 | -1) => {
      const root = rootRef.current;
      if (!root) return;
      const regions = [...root.querySelectorAll<HTMLElement>("[data-mast-region]")];
      if (regions.length === 0) return;
      const at = regions.findIndex((r) => r.contains(document.activeElement));
      const next = at < 0 ? (dir > 0 ? 0 : regions.length - 1) : (at + dir + regions.length) % regions.length;
      focusRegion(regions[next]);
    },
    [focusRegion],
  );

  useLayoutEffect(() => {
    if (focusDetailPending.current && focusDetail()) focusDetailPending.current = false;
    if (!refocusResults.current) return;
    refocusResults.current = false;
    focusResults();
  });

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const root = rootRef.current;
      if (!root) return;
      const target = e.target instanceof HTMLElement ? e.target : null;
      if (target && target !== document.body && !root.contains(target)) return;
      const s = archiveStore.getState();
      if (s.large.open) return;
      if (e.key === "Escape") {
        const layer = innermostLayer({ menuOpen: false, largeOpen: false, drawerOpen: s.detail.narrow && s.detail.drawerOpen, tokenChecking: s.token.checking, searching: !!s.run });
        if (layer === "drawer") {
          e.preventDefault();
          ctl.closeDrawer();
        } else if (layer === "token") {
          e.preventDefault();
          ctl.cancelTokenCheck();
        } else if (layer === "search") {
          e.preventDefault();
          ctl.cancelSearch();
        }
        return;
      }
      switch (archiveKeyCommand(e, { inInput: isTextEntry(target) })) {
        case "focus-query":
          e.preventDefault();
          focusQuery();
          return;
        case "next-region":
          e.preventDefault();
          cycleRegion(1);
          return;
        case "previous-region":
          e.preventDefault();
          cycleRegion(-1);
          return;
        case "toggle-view": {
          e.preventDefault();
          const results = root.querySelector('[data-mast-region="results"]');
          const hadFocus = !!results && results.contains(document.activeElement) && document.activeElement !== results;
          const before = s.view;
          refocusResults.current = hadFocus;
          ctl.toggleView();
          if (archiveStore.getState().view === before) refocusResults.current = false;
          return;
        }
        case "retry":
          e.preventDefault();
          if (target?.closest(DETAIL_REGION)) ctl.retryDetail();
          else ctl.retry();
          return;
        case "toggle-detail":
          e.preventDefault();
          ctl.toggleDetail();
          return;
        case "tab-previews":
          e.preventDefault();
          ctl.setDetailTab("previews");
          return;
        case "tab-metadata":
          e.preventDefault();
          ctl.setDetailTab("metadata");
          return;
        case "tab-products":
          e.preventDefault();
          ctl.setDetailTab("products");
          return;
        default:
          return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, ctl, focusQuery, cycleRegion]);

  return (
    <div ref={rootRef} className="ab-archive" data-active={active ? "true" : "false"}>
      <QueryBar state={state} ctl={ctl} queryRef={queryRef} onLeaveQuery={focusResults} />
      <div className="ab-archive-body">
        <ResultsRegion state={state} ctl={ctl} around={around} onEditQuery={focusQuery} onSwitchToWorkspace={onSwitchToWorkspace} onOpenDetail={openDetail} />
        <DetailPane state={state} ctl={ctl} />
      </div>
      <LargeView state={state} ctl={ctl} onFocusQuery={focusQuery} />
      <ArchiveToast toast={state.toast} ctl={ctl} />
      <ArchiveLiveRegion live={state.live} />
    </div>
  );
}
