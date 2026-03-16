import { Telescope, Upload, FolderOpen } from "lucide-react";

interface EmptyStateProps {
  onBrowseFiles: () => void;
  onSelectFolder: () => void;
}

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

  return (
    <div className="flex flex-col items-center justify-center h-full animate-fade-in">
      <div className="relative">
        <div className="ab-dropzone-ring group cursor-default">
          <div className="flex flex-col items-center gap-6">
            <div className="relative">
              <div className="absolute inset-0 bg-blue-500/20 rounded-full blur-xl scale-150 group-hover:bg-blue-500/30 transition-colors animate-glow-pulse" />
              <Telescope
                size={64}
                className="relative text-blue-400 group-hover:text-blue-300 transition-colors animate-float"
                strokeWidth={1.5}
              />
            </div>

            <div className="text-center">
              <h2 className="text-xl font-semibold text-zinc-100 mb-2">
                Drop your .fits / .asdf files here to begin
              </h2>
              <p className="text-zinc-500 text-sm">
                or use the buttons below
              </p>
            </div>

            <div className="flex items-center gap-4 w-full">
              <div className="flex-1 h-px bg-zinc-800" />
              <span className="text-zinc-600 text-xs font-medium">or</span>
              <div className="flex-1 h-px bg-zinc-800" />
            </div>

            <div className="flex gap-3">
              <button type="button" onClick={handleBrowse} className="ab-btn-primary">
                <Upload size={16} />
                Browse Files
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
        <p className="text-zinc-600 text-xs">
          Supports{" "}
          <span className="font-mono text-zinc-500">.fits</span>{" "}
          <span className="font-mono text-zinc-500">.fit</span>{" "}
          <span className="font-mono text-zinc-500">.fts</span>{" "}
          <span className="font-mono text-zinc-500">.asdf</span>
        </p>
        <p className="text-zinc-600 text-xs mt-1">
          JWST / HST / Generic
        </p>
      </div>
    </div>
  );
}
