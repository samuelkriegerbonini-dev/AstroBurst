import { useState, useEffect, useRef, useMemo, useCallback } from "react";
import { Search, FileText } from "lucide-react";

export interface PaletteAction {
  id: string;
  label: string;
  hint?: string;
  icon?: React.ComponentType<{ size?: number | string; style?: React.CSSProperties }>;
  run: () => void;
}

export interface PaletteFile {
  id: string;
  name: string;
  filter?: string;
}

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  actions: PaletteAction[];
  files: PaletteFile[];
  selectedId: string | null;
  onSelectFile: (id: string) => void;
}

const FILE_CAP = 50;

interface Entry {
  key: string;
  kind: "action" | "file";
  label: string;
  hint?: string;
  icon?: PaletteAction["icon"];
  run: () => void;
}

function matchScore(text: string, q: string): number {
  if (!q) return 1;
  const t = text.toLowerCase();
  const idx = t.indexOf(q);
  if (idx === -1) return 0;
  if (idx === 0) return 3;
  if (/[\s_\-./]/.test(t[idx - 1])) return 2;
  return 1;
}

export default function CommandPalette({ open, onClose, actions, files, selectedId, onSelectFile }: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [sel, setSel] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setSel(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const entries = useMemo<Entry[]>(() => {
    const q = query.trim().toLowerCase();
    const acts: (Entry & { score: number })[] = actions
      .map((a) => ({
        key: `a:${a.id}`,
        kind: "action" as const,
        label: a.label,
        hint: a.hint,
        icon: a.icon,
        run: a.run,
        score: matchScore(a.label, q),
      }))
      .filter((e) => e.score > 0);
    const fs: (Entry & { score: number })[] = files
      .map((f) => ({
        key: `f:${f.id}`,
        kind: "file" as const,
        label: f.name,
        hint: f.filter,
        icon: FileText,
        run: () => onSelectFile(f.id),
        score: Math.max(matchScore(f.name, q), f.filter ? matchScore(f.filter, q) : 0),
      }))
      .filter((e) => e.score > 0)
      .slice(0, FILE_CAP);
    acts.sort((a, b) => b.score - a.score);
    fs.sort((a, b) => b.score - a.score);
    return [...acts, ...fs];
  }, [actions, files, query, onSelectFile]);

  useEffect(() => { setSel(0); }, [query]);
  const clampedSel = Math.min(sel, Math.max(0, entries.length - 1));

  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>(`[data-index="${clampedSel}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [clampedSel, entries]);

  const runEntry = useCallback((entry: Entry) => {
    onClose();
    entry.run();
  }, [onClose]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSel((s) => Math.min(s + 1, entries.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSel((s) => Math.max(s - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const entry = entries[clampedSel];
      if (entry) runEntry(entry);
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }, [entries, clampedSel, runEntry, onClose]);

  if (!open) return null;

  const firstFileIdx = entries.findIndex((en) => en.kind === "file");

  return (
    <div className="ab-cmdp-overlay" onMouseDown={onClose}>
      <div className="ab-cmdp" onMouseDown={(e) => e.stopPropagation()}>
        <div className="ab-cmdp-input-row">
          <Search size={14} style={{ color: "var(--ab-text-4)", flexShrink: 0 }} />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Search files, tools and actions..."
            className="ab-cmdp-input"
            spellCheck={false}
          />
          <kbd className="ab-cmdp-kbd">esc</kbd>
        </div>
        <div ref={listRef} className="ab-cmdp-list">
          {entries.length === 0 && <div className="ab-cmdp-empty">No matches for &ldquo;{query}&rdquo;</div>}
          {entries.map((entry, i) => {
            const Icon = entry.icon;
            const isFileHeader = i === firstFileIdx;
            return (
              <div key={entry.key}>
                {isFileHeader && <div className="ab-cmdp-section">Files</div>}
                {i === 0 && entry.kind === "action" && <div className="ab-cmdp-section">Actions</div>}
                <button
                  data-index={i}
                  data-active={i === clampedSel}
                  className="ab-cmdp-item"
                  onMouseEnter={() => setSel(i)}
                  onClick={() => runEntry(entry)}
                >
                  {Icon && <Icon size={13} style={{ flexShrink: 0, opacity: 0.7 }} />}
                  <span className="ab-cmdp-item-label">
                    {entry.label}
                    {entry.kind === "file" && entry.key === `f:${selectedId}` && <span className="ab-cmdp-current"> — current</span>}
                  </span>
                  {entry.hint && <span className="ab-cmdp-hint">{entry.hint}</span>}
                </button>
              </div>
            );
          })}
        </div>
        <div className="ab-cmdp-footer">
          <span><kbd className="ab-cmdp-kbd">↑↓</kbd> navigate</span>
          <span><kbd className="ab-cmdp-kbd">↵</kbd> select</span>
          <span><kbd className="ab-cmdp-kbd">esc</kbd> close</span>
        </div>
      </div>
    </div>
  );
}
