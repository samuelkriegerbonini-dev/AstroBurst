import { useEffect, useId, useRef, useState, type CSSProperties } from "react";
import { ArrowRight, Check, EyeOff, ImageIcon, Layers, Plus, Search, Settings, Sparkles, Telescope, X, type LucideIcon } from "lucide-react";
import { dockStore } from "../../hooks/useDockLayout";
import { ANCHOR_LABELS, anchorOf, anchorOpenTool, DOCK_TOOL_META, type DockLayout, type DockToolId } from "../../utils/dockLayout";
import { TOOL_CATEGORIES, TOOL_DESCRIPTIONS } from "../../utils/toolCatalog";
import { DOCK_TOOLS } from "./toolRegistry";
import { dockOnboardingStore } from "./dockOnboardingStore";
import { rightRailToolsTrigger } from "./dockGeometry";
import { openToolBrowserDialog } from "./toolBrowserDialog";
import { announce } from "./useDockDrag";
import "./tool-browser.css";

const CATEGORY_ICONS: Record<(typeof TOOL_CATEGORIES)[number]["id"], LucideIcon> = {
  inspect: ImageIcon,
  analyze: Telescope,
  process: Sparkles,
  create: Layers,
  workspace: Settings,
};

const OPEN_ANNOUNCE_DELAY_MS = 150;

interface ToolBrowserProps {
  layout: DockLayout;
  hasFile: boolean;
  enabledTools: readonly DockToolId[];
  onToggleTool(tool: DockToolId): void;
  onClose(): void;
}

export default function ToolBrowser({ layout, hasFile, enabledTools, onToggleTool, onClose }: ToolBrowserProps) {
  const [query, setQuery] = useState("");
  const dialogRef = useRef<HTMLDialogElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const pressStartedOnBackdrop = useRef(false);
  const titleId = useId();
  const search = query.trim().toLowerCase();
  const groups = TOOL_CATEGORIES.map((group) => ({
    ...group,
    tools: group.tools.filter((tool) => {
      const meta = DOCK_TOOL_META[tool];
      return [meta.label, TOOL_DESCRIPTIONS[tool], group.label, ...(meta.keywords ?? [])].some((text) => text.toLowerCase().includes(search));
    }),
  })).filter((group) => group.tools.length > 0);
  const count = groups.reduce((total, group) => total + group.tools.length, 0);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    return openToolBrowserDialog(dialog, inputRef.current, opener, rightRailToolsTrigger);
  }, []);

  return (
    <dialog
      ref={dialogRef}
      className="ab-tool-browser"
      aria-labelledby={titleId}
      onCancel={(event) => { event.preventDefault(); onClose(); }}
      onPointerDown={(event) => { pressStartedOnBackdrop.current = event.target === event.currentTarget; }}
      onClick={(event) => {
        const startedOnBackdrop = pressStartedOnBackdrop.current;
        pressStartedOnBackdrop.current = false;
        if (startedOnBackdrop && event.target === event.currentTarget) onClose();
      }}
      onKeyDown={(event) => event.stopPropagation()}
      onKeyUp={(event) => event.stopPropagation()}
    >
      <div className="ab-tool-browser-content">
        <header className="ab-tool-browser-header">
          <div>
            <h2 id={titleId}>All tools</h2>
            <p>Open a tool or choose which shortcuts appear in your sidebar.</p>
          </div>
          <button type="button" onClick={onClose} className="ab-tool-browser-close" aria-label="Close tool browser"><X size={18} /></button>
        </header>
        <div className="ab-tool-browser-search">
          <Search size={16} aria-hidden="true" />
          <input ref={inputRef} type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search tools, e.g. plate solve" aria-label="Search tools" />
          {query && <button type="button" aria-label="Clear tool search" onClick={() => { setQuery(""); inputRef.current?.focus(); }}><X size={14} /></button>}
        </div>
        <div className="ab-tool-browser-results" role="status">{count} {count === 1 ? "tool" : "tools"}{search ? " found" : " available"}</div>
        <div className="ab-tool-browser-groups">
          {groups.map((group) => {
            const CategoryIcon = CATEGORY_ICONS[group.id];
            return (
              <section key={group.id} className="ab-tool-browser-category" aria-label={group.label} style={{ "--tool-category-color": group.color } as CSSProperties}>
                <h3><CategoryIcon size={14} aria-hidden="true" />{group.label}<span className="ab-tool-browser-category-count">{group.tools.length}</span></h3>
                <ul role="list" className="ab-tool-browser-grid">
                  {group.tools.map((tool) => {
                    const meta = DOCK_TOOL_META[tool];
                    const Icon = DOCK_TOOLS[tool].icon;
                    const anchor = anchorOf(layout, tool);
                    const open = anchorOpenTool(layout, anchor, hasFile) === tool;
                    const pinned = enabledTools.includes(tool);
                    const toggleAction = pinned ? "Remove from sidebar" : "Add to sidebar";
                    const disabled = meta.needsFile && !hasFile;
                    const descriptionId = `${titleId}-${tool}-desc`;
                    return (
                      <li
                        key={tool}
                        className="ab-tool-browser-card"
                        data-browser-tool={tool}
                        data-selected={pinned}
                        data-open={open || undefined}
                      >
                        <button
                          type="button"
                          className="ab-tool-browser-open"
                          data-open-tool={tool}
                          aria-label={`Open ${meta.label}`}
                          aria-describedby={descriptionId}
                          aria-disabled={disabled || undefined}
                          onClick={() => {
                            if (disabled) return;
                            dockStore.dispatch({ type: "open", tool });
                            dockOnboardingStore.stripToggled(tool);
                            onClose();
                            const message = open ? `${meta.label} is already open` : `${meta.label} opened in ${ANCHOR_LABELS[anchor]}`;
                            setTimeout(() => announce(message), OPEN_ANNOUNCE_DELAY_MS);
                          }}
                        >
                          <span className="ab-tool-browser-icon"><Icon size={18} aria-hidden="true" /></span>
                          <span className="ab-tool-browser-copy">
                            <span className="ab-tool-browser-card-heading"><strong>{meta.label}</strong>{open && <span className="ab-tool-browser-open-state" aria-hidden="true">Open</span>}</span>
                            <span id={descriptionId} className="ab-tool-browser-description">
                              <small>{TOOL_DESCRIPTIONS[tool]}</small>
                              {open && <span className="sr-only"> Currently open.</span>}
                              {disabled && <span className="ab-tool-browser-reason"> Open a file first</span>}
                            </span>
                          </span>
                          <ArrowRight className="ab-tool-browser-arrow" size={13} aria-hidden="true" />
                        </button>
                        <div className="ab-tool-browser-card-actions">
                          <span className="ab-tool-browser-sidebar-state">{pinned && <Check size={12} aria-hidden="true" />}{pinned ? "In sidebar" : "Not in sidebar"}</span>
                          <button
                            type="button"
                            className="ab-tool-browser-toggle"
                            data-toggle-tool={tool}
                            aria-label={`${toggleAction}: ${meta.label}`}
                            onClick={() => onToggleTool(tool)}
                          >
                            {pinned ? <EyeOff size={12} aria-hidden="true" /> : <Plus size={12} aria-hidden="true" />}
                            {toggleAction}
                          </button>
                        </div>
                      </li>
                    );
                  })}
                </ul>
              </section>
            );
          })}
          {count === 0 && <div className="ab-tool-browser-empty"><Search size={24} /><strong>No tools found</strong><span>Try a tool name or an operation such as histogram.</span></div>}
        </div>
        <footer className="ab-tool-browser-footer">Tools removed from the sidebar stay available here.<span><kbd>Esc</kbd> to close</span></footer>
      </div>
    </dialog>
  );
}
