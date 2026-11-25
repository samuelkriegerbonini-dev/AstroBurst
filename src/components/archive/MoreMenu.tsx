import { useEffect, useRef, useState, type KeyboardEvent, type MouseEvent, type RefObject } from "react";
import { ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import { romanLockOf, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { moreMenuView, notServedTitle, type MoreEntry, type MoreMenuView } from "./logic/missionState";
import { useNow } from "./state/useNow";
import type { MastMissionId } from "../../shared/types/mast";

const MENU_ID = "mast-more-menu";

interface MoreMenuListProps {
  view: MoreMenuView;
  onPick: (id: MastMissionId) => void;
  onCheck: (id: MastMissionId) => void;
  listRef?: RefObject<HTMLDivElement | null>;
  onKeyDown?: (e: KeyboardEvent<HTMLDivElement>) => void;
}

function keepFocusInside(e: MouseEvent<HTMLDivElement>) {
  if (!(e.target instanceof Element) || !e.target.closest("button")) e.preventDefault();
}

export function MoreMenuList({ view, onPick, onCheck, listRef, onKeyDown }: MoreMenuListProps) {
  const [notServedOpen, setNotServedOpen] = useState(false);
  const checking = view.notServed.some((e) => e.check === "checking");
  const notes = view.notServed.map((e) => e.note).filter((note): note is string => !!note);
  const row = (e: MoreEntry, withCheck = true) => {
    const reasonId = `mast-more-reason-${e.id}`;
    return (
      <div key={e.id} className="ab-mast-missions-row">
        <button
          type="button"
          data-mission={e.id}
          data-current={e.current ? "true" : "false"}
          aria-disabled={!e.enabled}
          aria-current={e.current ? "true" : undefined}
          aria-describedby={e.reason || e.note ? reasonId : undefined}
          className="ab-mast-missions-item"
          onClick={() => {
            if (e.enabled) onPick(e.id);
          }}
        >
          <span className="ab-mast-missions-label">
            {e.label}
            <span className="ab-mast-missions-status" data-status={e.status}>
              {e.status}
            </span>
          </span>
          {e.reason || e.note ? (
            <span id={reasonId} className="ab-mast-missions-reason">
              {e.reason}
              {e.note ? <span className="ab-mast-missions-note">{e.note}</span> : null}
            </span>
          ) : null}
        </button>
        {withCheck && e.check ? (
          <button
            type="button"
            data-check-mission={e.id}
            aria-disabled={e.check === "checking"}
            aria-label={e.check === "checking" ? `Checking ${e.label}` : `Check ${e.label} again`}
            className="ab-mast-missions-check"
            onClick={() => {
              if (e.check === "idle") onCheck(e.id);
            }}
          >
            {e.check === "checking" ? (
              <>
                <Loader2 size={10} aria-hidden="true" className="ab-mast-spin" />
                Checking…
              </>
            ) : (
              "Check again"
            )}
          </button>
        ) : null}
      </div>
    );
  };

  return (
    <div id={MENU_ID} ref={listRef} data-testid="mast-more-menu" role="group" aria-label="More missions" className="ab-mast-menu ab-mast-missions-menu" onKeyDown={onKeyDown} onMouseDown={keepFocusInside}>
      {view.enabled.map((e) => row(e))}
      {view.enabled.length && view.disabled.length + view.notServed.length ? <div role="separator" className="ab-mast-missions-sep" /> : null}
      {view.disabled.map((e) => row(e))}
      {view.notServed.length ? (
        <div className="ab-mast-missions-row" data-testid="mast-more-not-served">
          <button type="button" className="ab-mast-missions-item" aria-expanded={notServedOpen} onClick={() => setNotServedOpen((v) => !v)}>
            <span className="ab-mast-missions-label">
              {notServedOpen ? <ChevronDown size={10} aria-hidden="true" /> : <ChevronRight size={10} aria-hidden="true" />}
              {notServedTitle(view.notServed)}
            </span>
            <span className="ab-mast-missions-reason">
              {view.notServed.map((e) => e.label).join(", ")}
              {notes.length ? <span className="ab-mast-missions-note">{notes[0]}</span> : null}
            </span>
          </button>
          <button
            type="button"
            data-check-missions={view.notServed.map((e) => e.id).join(" ")}
            aria-disabled={checking}
            aria-label={checking ? "Checking the missions MAST does not serve" : "Check the missions MAST does not serve again"}
            className="ab-mast-missions-check"
            onClick={() => {
              if (!checking) view.notServed.forEach((e) => onCheck(e.id));
            }}
          >
            {checking ? (
              <>
                <Loader2 size={10} aria-hidden="true" className="ab-mast-spin" />
                Checking…
              </>
            ) : (
              "Check again"
            )}
          </button>
        </div>
      ) : null}
      {notServedOpen ? view.notServed.map((e) => row(e, false)) : null}
    </div>
  );
}

interface MoreMenuProps {
  state: ArchiveState;
  ctl: Pick<ArchiveController, "switchMission" | "checkMission">;
  onPicked?: () => void;
}

export default function MoreMenu({ state, ctl, onPicked }: MoreMenuProps) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const now = useNow(open ? 30_000 : null);
  const view = moreMenuView(state.missions, romanLockOf(state), state.probes, state.form.mission, now);

  useEffect(() => {
    if (!open) return;
    listRef.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const onDown = (e: PointerEvent) => {
      if (e.target instanceof Node && wrapRef.current?.contains(e.target)) return;
      setOpen(false);
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [open]);

  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) buttonRef.current?.focus();
  };

  const onListKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close(true);
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Home" && e.key !== "End") return;
    const items = [...(listRef.current?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    if (items.length === 0) return;
    e.preventDefault();
    const at = items.findIndex((b) => b === document.activeElement);
    let next = at;
    if (e.key === "Home") next = 0;
    else if (e.key === "End") next = items.length - 1;
    else next = (Math.max(0, at) + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
    if (at < 0 && e.key === "ArrowDown") next = 0;
    items[next].focus();
  };

  const pick = (id: MastMissionId) => {
    setOpen(false);
    ctl.switchMission(id);
    onPicked?.();
  };

  return (
    <div
      ref={wrapRef}
      className="ab-mast-missions"
      onBlur={(e) => {
        if (!open) return;
        const next = e.relatedTarget;
        if (next instanceof Node && wrapRef.current?.contains(next)) return;
        if (next) setOpen(false);
      }}
    >
      <button
        ref={buttonRef}
        type="button"
        data-testid="mast-more"
        className="ab-pill ab-mast-missions-btn"
        aria-expanded={open}
        aria-controls={open ? MENU_ID : undefined}
        title="More missions"
        onClick={() => setOpen((v) => !v)}
        onKeyDown={(e) => {
          if (e.key !== "ArrowDown" || open) return;
          e.preventDefault();
          setOpen(true);
        }}
      >
        More
        <ChevronDown size={11} aria-hidden="true" />
      </button>
      {open ? <MoreMenuList view={view} onPick={pick} onCheck={(id) => void ctl.checkMission(id)} listRef={listRef} onKeyDown={onListKey} /> : null}
    </div>
  );
}
