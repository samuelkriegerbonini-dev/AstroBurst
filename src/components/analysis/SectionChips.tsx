import { useCallback, useRef } from "react";
import { showSectionChips, type AnalysisSection } from "../../utils/analysisSections";

export interface SectionChipsProps {
  label: string;
  sections: readonly AnalysisSection[];
}

export default function SectionChips({ label, sections }: SectionChipsProps) {
  const navRef = useRef<HTMLDivElement>(null);
  const jumpTo = useCallback((id: string) => {
    const el = document.getElementById(id);
    if (!el) return;
    el.style.scrollMarginTop = `${(navRef.current?.offsetHeight ?? 0) + 8}px`;
    el.scrollIntoView({ block: "start", behavior: "smooth" });
  }, []);

  if (!showSectionChips(sections)) return null;

  return (
    <div
      ref={navRef}
      className="sticky top-0 z-20 -mx-3 -mt-3 px-3 py-1.5"
      style={{ background: "rgb(5,5,16)", borderBottom: "1px solid var(--ab-border)" }}
    >
      <nav aria-label={`${label} panels`} className="flex flex-wrap gap-1">
        {sections.map((s) => (
          <button
            key={s.id}
            type="button"
            onClick={() => jumpTo(s.id)}
            className="text-[9px] px-1.5 py-0.5 rounded-full text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800/70 transition-colors"
            style={{ border: "1px solid rgba(63,63,70,0.5)" }}
          >
            {s.label}
          </button>
        ))}
      </nav>
    </div>
  );
}
