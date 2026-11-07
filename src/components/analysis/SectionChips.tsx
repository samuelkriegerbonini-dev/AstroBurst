import { useCallback, useRef, type KeyboardEvent, type ReactNode } from "react";
import { Info, X } from "lucide-react";
import { showSectionChips, type AnalysisSection } from "../../utils/analysisSections";
import type { SectionTipState } from "./useSectionTip";

export interface SectionChipsProps {
  label: string;
  sections: readonly AnalysisSection[];
  tip?: SectionTipState;
}

export interface SectionTipNoteProps {
  tip: SectionTipState;
  sectionId: string;
}

function stopEscape(event: KeyboardEvent<HTMLElement>, active: boolean): boolean {
  if (event.key !== "Escape" || !active) return false;
  event.preventDefault();
  event.stopPropagation();
  return true;
}

function helpButton(tip: SectionTipState, sectionId: string, title: string, className: string, children: ReactNode, onOpen?: () => void) {
  const expanded = tip.openId === sectionId;
  return (
    <button
      type="button"
      data-section-help={sectionId}
      aria-label={`Help: ${title}`}
      title={`Help: ${title}`}
      aria-expanded={expanded}
      aria-controls={expanded ? tip.noteId : undefined}
      aria-describedby={expanded ? tip.bodyId : undefined}
      onClick={() => {
        tip.toggle(sectionId);
        if (!expanded) onOpen?.();
      }}
      className={className}
    >
      {children}
    </button>
  );
}

export default function SectionChips({ label, sections, tip }: SectionChipsProps) {
  const navRef = useRef<HTMLDivElement>(null);
  const scrollToSection = useCallback((id: string) => {
    requestAnimationFrame(() => {
      const el = document.getElementById(id);
      if (!el) return;
      el.style.scrollMarginTop = `${(navRef.current?.offsetHeight ?? 0) + 8}px`;
      const reduceMotion = typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
      el.scrollIntoView({ block: "start", behavior: reduceMotion ? "auto" : "smooth" });
    });
  }, []);

  if (!showSectionChips(sections)) return null;

  const openId = tip?.openId ?? null;
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (stopEscape(event, openId !== null)) tip?.dismiss(false);
  };

  return (
    <div
      ref={navRef}
      onKeyDown={onKeyDown}
      className="sticky top-0 z-20 -mx-3 -mt-3 px-3 py-1.5"
      style={{ background: "rgb(5,5,16)", borderBottom: "1px solid var(--ab-border)" }}
    >
      <nav aria-label={`${label} panels`} className="flex flex-wrap gap-1">
        {sections.map((s) => {
          const help = tip?.tips[s.id];
          return (
            <span key={s.id} className="inline-flex items-center rounded-full" style={{ border: "1px solid rgba(63,63,70,0.5)" }}>
              <button
                type="button"
                onClick={() => {
                  tip?.open(s.id);
                  scrollToSection(s.id);
                }}
                aria-describedby={openId === s.id ? tip?.bodyId : undefined}
                className="text-[10px] px-2 py-1 rounded-full text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800/70 transition-colors"
              >
                {s.label}
              </button>
              {tip && help &&
                helpButton(tip, s.id, help.title, "p-1 mr-0.5 rounded-full text-zinc-500 hover:text-teal-300", <Info size={12} aria-hidden="true" />, () => scrollToSection(s.id))}
            </span>
          );
        })}
      </nav>
    </div>
  );
}

export function SectionTipNote({ tip, sectionId }: SectionTipNoteProps) {
  const content = tip.tips[sectionId];
  if (!content) return null;
  const open = tip.openId === sectionId;
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (stopEscape(event, open)) tip.dismiss();
  };

  return (
    <div data-section-tip={sectionId} onKeyDown={onKeyDown}>
      {tip.helpInSection &&
        helpButton(
          tip,
          sectionId,
          content.title,
          "mb-2 inline-flex items-center gap-1.5 rounded px-1 py-0.5 text-[10px] text-zinc-500 hover:text-teal-300",
          <>
            <Info size={12} aria-hidden="true" />
            {content.title}
          </>,
        )}
      <div aria-live="polite">
        {open && (
          <aside id={tip.noteId} role="note" aria-label={content.title} className="mb-2 flex items-start gap-2 rounded-md border border-teal-700/40 bg-teal-900/15 px-3 py-2 text-[11px] leading-relaxed text-zinc-300">
            <Info size={14} className="mt-0.5 shrink-0 text-teal-300" aria-hidden="true" />
            <div className="min-w-0 flex-1"><strong className="font-medium text-teal-200">{content.title}</strong><p id={tip.bodyId} className="mt-1 break-words">{content.body}</p></div>
            <button type="button" onClick={() => tip.dismiss()} aria-label={`Dismiss ${content.title} tip`} title="Dismiss tip" className="shrink-0 rounded p-1.5 text-zinc-400 hover:text-zinc-100"><X size={14} aria-hidden="true" /></button>
          </aside>
        )}
      </div>
    </div>
  );
}
