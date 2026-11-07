import { useId, useRef, useState } from "react";
import { showSectionChips, type AnalysisSection } from "../../utils/analysisSections";

export interface SectionTip {
  title: string;
  body: string;
}

export type SectionTips = Readonly<Partial<Record<string, SectionTip>>>;

export interface SectionTipState {
  tips: SectionTips;
  openId: string | null;
  noteId: string;
  bodyId: string;
  helpInSection: boolean;
  open(id: string, force?: boolean): void;
  toggle(id: string): void;
  dismiss(restoreFocus?: boolean): void;
}

const NO_TIPS: SectionTips = {};

const storageKey = (id: string) => `ab.section-tip.${id}`;

function dismissedInStorage(id: string): boolean {
  try {
    return localStorage.getItem(storageKey(id)) === "dismissed";
  } catch {
    return false;
  }
}

function storeDismissal(id: string): void {
  try {
    localStorage.setItem(storageKey(id), "dismissed");
  } catch {
    return;
  }
}

export function focusSectionHelp(root: Pick<ParentNode, "querySelector"> | null | undefined, id: string): void {
  root?.querySelector<HTMLElement>(`[data-section-help="${id}"]`)?.focus({ preventScroll: true });
}

export function useSectionTip(sections: readonly AnalysisSection[], tips: SectionTips = NO_TIPS): SectionTipState {
  const noteId = useId();
  const dismissed = useRef(new Set<string>());
  const seen = useRef(new Set<string>());
  const [requested, setRequested] = useState<string | null>(null);
  const openId = requested !== null && tips[requested] !== undefined && sections.some((section) => section.id === requested) ? requested : null;

  const open = (id: string, force = false) => {
    if (openId !== null && openId !== id) seen.current.add(openId);
    const held = dismissed.current.has(id) || seen.current.has(id) || dismissedInStorage(id);
    setRequested(tips[id] && (force || openId === id || !held) ? id : null);
  };

  const dismiss = (restoreFocus = true) => {
    if (openId === null) return;
    dismissed.current.add(openId);
    storeDismissal(openId);
    setRequested(null);
    if (restoreFocus) focusSectionHelp(typeof document === "undefined" ? null : document, openId);
  };

  const toggle = (id: string) => (openId === id ? dismiss() : open(id, true));

  return { tips, openId, noteId, bodyId: `${noteId}-body`, helpInSection: !showSectionChips(sections), open, toggle, dismiss };
}
