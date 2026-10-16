import type { ReactNode } from "react";
import { Search } from "lucide-react";
import { AstroLogo } from "./AstroLogo";
import ModeSwitch from "./ModeSwitch";
import type { AppMode } from "../hooks/useAppMode";
import { APP_VERSION } from "../utils/constants";

interface AppFooterProps {
  mode: AppMode;
  onModeChange: (mode: AppMode) => void;
  onOpenPalette: () => void;
  onOpenShortcuts: () => void;
  children?: ReactNode;
}

export default function AppFooter({ mode, onModeChange, onOpenPalette, onOpenShortcuts, children }: AppFooterProps) {
  return (
    <footer
      data-testid="app-footer"
      className="px-4 py-1.5 flex items-center justify-between shrink-0"
      style={{ borderTop: "1px solid var(--ab-border)", background: "rgba(5,5,16,0.6)" }}
    >
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2.5 pointer-events-auto select-none">
          <AstroLogo size={22} showText={false} className="opacity-40" />
          <span className="text-[10px] font-bold tracking-[0.25em] uppercase cosmic-text" style={{ opacity: 0.6 }}>AstroBurst</span>
          <span className="text-[9px] font-mono uppercase" style={{ color: "rgba(20,184,166,0.55)" }}>{APP_VERSION}</span>
        </div>
        <ModeSwitch mode={mode} onChange={onModeChange} />
        {children ? (
          <>
            <div className="w-px h-3" style={{ background: "var(--ab-border)" }} />
            {children}
          </>
        ) : null}
      </div>
      <div className="flex items-center gap-1">
        <button
          type="button"
          onClick={onOpenPalette}
          title="Search everywhere (Ctrl+K or double Shift)"
          className="flex items-center gap-1.5 px-2 py-1 rounded text-[10px] transition-colors text-zinc-500 hover:text-zinc-300"
          style={{ background: "transparent", border: "none" }}
        >
          <Search size={10} />
          Search
          <kbd className="ab-cmdp-kbd">Ctrl+K</kbd>
        </button>
        <button
          type="button"
          onClick={onOpenShortcuts}
          title="Keyboard shortcuts (?)"
          className="px-2 py-1 rounded text-[10px] transition-colors text-zinc-500 hover:text-zinc-300"
          style={{ background: "transparent", border: "none" }}
        >
          <kbd className="ab-cmdp-kbd">?</kbd>
        </button>
      </div>
    </footer>
  );
}
