import { Activity } from "lucide-react";
import { SPECTRUM_VIEWER_HINT, SPECTRUM_VIEWER_OPEN_CUBE, spectrumViewerTitle } from "../../utils/previewShell";

interface SpectrumViewerEmptyProps {
  name: string;
  onOpenCubeTool: () => void;
}

export default function SpectrumViewerEmpty({ name, onOpenCubeTool }: SpectrumViewerEmptyProps) {
  return (
    <div data-spectrum-empty="" className="ab-viewer-empty">
      <div className="flex flex-col items-center gap-3 px-6 text-center max-w-md">
        <Activity size={36} strokeWidth={1} style={{ color: "var(--ab-teal)" }} />
        <span className="text-xs text-zinc-300 break-all">{spectrumViewerTitle(name)}</span>
        <span className="text-[11px] text-zinc-500">{SPECTRUM_VIEWER_HINT}</span>
        <button
          type="button"
          onClick={onOpenCubeTool}
          className="text-[11px] px-3 py-1 rounded text-zinc-200 hover:text-white transition-colors"
          style={{ border: "1px solid var(--ab-border)", background: "rgba(20,184,166,0.12)" }}
        >
          {SPECTRUM_VIEWER_OPEN_CUBE}
        </button>
      </div>
    </div>
  );
}
