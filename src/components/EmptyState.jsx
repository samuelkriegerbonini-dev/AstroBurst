import { motion } from "framer-motion";
import { Telescope, Upload, FolderOpen } from "lucide-react";

const scaleIn = {
  initial: { scale: 0.95, opacity: 0 },
  animate: { scale: 1, opacity: 1 },
  exit: { scale: 0.95, opacity: 0 },
  transition: { duration: 0.3 },
};

export default function EmptyState({ onBrowseFiles, onSelectFolder }) {
  const handleBrowse = (e) => {
    e.preventDefault();
    e.stopPropagation();
    onBrowseFiles();
  };

  const handleFolder = (e) => {
    e.preventDefault();
    e.stopPropagation();
    onSelectFolder();
  };

  return (
    <motion.div
      className="flex flex-col items-center justify-center h-full"
      {...scaleIn}
    >
      <div className="relative">
        <div className="relative border-2 border-dashed border-zinc-700 hover:border-blue-500 rounded-2xl px-16 py-12 transition-all duration-300 hover:scale-[1.01] hover:shadow-[0_0_30px_rgba(59,130,246,0.15)] group cursor-default">
          <div className="flex flex-col items-center gap-6">
            <div className="relative">
              <div className="absolute inset-0 bg-blue-500/20 rounded-full blur-xl scale-150 group-hover:bg-blue-500/30 transition-colors" />
              <Telescope
                size={64}
                className="relative text-blue-400 group-hover:text-blue-300 transition-colors"
                strokeWidth={1.5}
              />
            </div>

            <div className="text-center">
              <h2 className="text-xl font-semibold text-zinc-100 mb-2">
                Drop your .fits files here to begin
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
              <button
                type="button"
                onClick={handleBrowse}
                className="flex items-center gap-2 bg-blue-500 hover:bg-blue-600 text-white rounded-lg px-5 py-2.5 font-medium transition-colors text-sm cursor-pointer"
              >
                <Upload size={16} />
                Browse Files
              </button>
              <button
                type="button"
                onClick={handleFolder}
                className="flex items-center gap-2 bg-zinc-800 hover:bg-zinc-700 text-zinc-200 rounded-lg px-5 py-2.5 font-medium transition-colors text-sm cursor-pointer"
              >
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
          <span className="font-mono text-zinc-500">.fts</span>
        </p>
        <p className="text-zinc-600 text-xs mt-1">
          JWST / HST / Generic
        </p>
      </div>
    </motion.div>
  );
}
