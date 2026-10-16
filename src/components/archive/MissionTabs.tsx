import type { KeyboardEvent } from "react";
import { Lock, LockOpen } from "lucide-react";
import { missionEntries, tabMissions, type MissionEntry, type RomanLock } from "./logic/missionState";
import { romanLockOf, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";

interface MissionTabsProps {
  state: ArchiveState;
  ctl: Pick<ArchiveController, "switchMission">;
}

const LOCK_WORDS: Record<RomanLock, string> = {
  locked: "locked",
  checking: "checking the token",
  unlocked: "unlocked",
  expired: "token no longer accepted",
};

function tabLabel(entry: MissionEntry): string {
  return entry.lock && entry.lock !== "unlocked" ? `${entry.label}, ${LOCK_WORDS[entry.lock]}` : entry.label;
}

export default function MissionTabs({ state, ctl }: MissionTabsProps) {
  const tabs = tabMissions(missionEntries(state.missions, romanLockOf(state)), state.form.mission);
  const current = state.form.mission;

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight" && e.key !== "Home" && e.key !== "End") return;
    const usable = tabs.filter((t) => t.enabled);
    if (usable.length === 0) return;
    e.preventDefault();
    const at = usable.findIndex((t) => t.id === current);
    let next = at;
    if (e.key === "Home") next = 0;
    else if (e.key === "End") next = usable.length - 1;
    else next = (Math.max(0, at) + (e.key === "ArrowRight" ? 1 : -1) + usable.length) % usable.length;
    const target = usable[next];
    ctl.switchMission(target.id);
    const button = e.currentTarget.querySelector<HTMLButtonElement>(`[data-mission="${target.id}"]`);
    button?.focus();
  };

  return (
    <div role="tablist" aria-label="Mission" className="ab-mast-tabs" onKeyDown={onKeyDown}>
      {tabs.map((t) => {
        const selected = t.id === current;
        return (
          <button
            key={t.id}
            type="button"
            role="tab"
            data-testid="mast-mission-tab"
            data-mission={t.id}
            data-accent={t.accent}
            data-active={selected ? "true" : "false"}
            data-lock={t.lock ?? undefined}
            aria-selected={selected}
            aria-disabled={!t.enabled}
            aria-controls="mast-results-region"
            aria-label={tabLabel(t)}
            tabIndex={selected ? 0 : -1}
            title={t.reason ?? undefined}
            className="ab-pill ab-mast-tab"
            onClick={() => {
              if (t.enabled && !selected) ctl.switchMission(t.id);
            }}
          >
            {t.lock ? t.lock === "unlocked" ? <LockOpen size={11} aria-hidden="true" /> : <Lock size={11} aria-hidden="true" /> : null}
            {t.label}
          </button>
        );
      })}
    </div>
  );
}
