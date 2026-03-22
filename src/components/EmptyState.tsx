import { Telescope, Upload, FolderOpen, Keyboard } from "lucide-react";

interface EmptyStateProps {
  onBrowseFiles: () => void;
  onSelectFolder: () => void;
}

const FORMAT_BADGES = [".fits", ".fit", ".fts", ".asdf"];

export default function EmptyState({ onBrowseFiles, onSelectFolder }: EmptyStateProps) {
  const handleBrowse = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    e.stopPropagation();
    onBrowseFiles();
  };

  const handleFolder = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    e.stopPropagation();
    onSelectFolder();
  };

  const isMac = navigator.platform?.includes("Mac");

  return (
    <div className="flex flex-col items-center justify-center h-full animate-fade-in">
      <div
        className="relative rounded-2xl p-1"
        style={{
          background: "radial-gradient(ellipse at center, rgba(5,5,16,0.92) 0%, rgba(5,5,16,0.75) 60%, transparent 100%)",
        }}
      >
        <div className="ab-dropzone-ring group cursor-default">
          <div className="flex flex-col items-center gap-6">
            <div className="relative">
              <div className="absolute inset-0 bg-blue-500/20 rounded-full blur-xl scale-150 group-hover:bg-blue-500/30 transition-colors animate-glow-pulse" />
              <Telescope
                size={56}
                className="relative text-blue-400 group-hover:text-blue-300 transition-colors animate-float"
                strokeWidth={1.5}
              />
            </div>

            <div className="text-center max-w-md">
              <h2 className="text-xl font-semibold text-zinc-50 mb-2">
                Drop your files here to begin
              </h2>
              <p className="text-zinc-300 text-sm leading-relaxed">
                Process, calibrate, stack and compose JWST / HST astronomical data
              </p>
            </div>

            <div className="flex items-center gap-4 w-full">
              <div className="flex-1 h-px bg-zinc-700/60" />
              <span className="text-zinc-500 text-xs font-medium">or</span>
              <div className="flex-1 h-px bg-zinc-700/60" />
            </div>

            <div className="flex gap-3">
              <button type="button" onClick={handleBrowse} className="ab-btn-primary">
                <Upload size={16} />
                Browse Files
                <kbd
                  className="ml-1 text-[9px] font-mono px-1.5 py-0.5 rounded"
                  style={{ background: "rgba(255,255,255,0.15)", color: "rgba(255,255,255,0.7)" }}
                >
                  {isMac ? "⌘O" : "Ctrl+O"}
                </kbd>
              </button>
              <button type="button" onClick={handleFolder} className="ab-btn-secondary">
                <FolderOpen size={16} />
                Select Folder
              </button>
            </div>
          </div>
        </div>
      </div>

      <div className="mt-6 text-center">
        <div className="flex items-center justify-center gap-2 flex-wrap">
          {FORMAT_BADGES.map((fmt) => (
            <span
              key={fmt}
              className="inline-block font-mono text-xs px-2.5 py-1 rounded"
              style={{
                color: "#a1a1aa",
                background: "rgba(39,39,42,0.6)",
                border: "1px solid rgba(63,63,70,0.5)",
              }}
            >
              {fmt}
            </span>
          ))}
        </div>
        <p className="text-zinc-400 text-xs mt-2 tracking-wide">
          JWST · HST · Generic FITS
        </p>
      </div>
    </div>
  );
}
