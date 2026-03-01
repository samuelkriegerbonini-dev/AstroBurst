import { motion } from "framer-motion";
import FileItem from "./FileItem";
import type { ProcessedFile } from "../utils/types";

const slideIn = {
  initial: { x: -30, opacity: 0 },
  animate: { x: 0, opacity: 1 },
  transition: { duration: 0.3 },
};

interface FileListProps {
  files: ProcessedFile[];
  selected: string | null;
  onSelect: (id: string) => void;
}

export default function FileList({ files, selected, onSelect }: FileListProps) {
  return (
    <motion.div
      {...slideIn}
      className="flex flex-col h-full bg-zinc-900 border border-zinc-800 rounded-xl overflow-hidden"
    >
      <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
        <h3 className="text-sm font-semibold text-zinc-300">Files</h3>
        <span className="text-xs font-mono text-zinc-500">{files.length} total</span>
      </div>

      <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
        {files.map((file, index) => (
          <FileItem
            key={file.id}
            file={file}
            isSelected={file.id === selected}
            onSelect={onSelect}
            index={index}
          />
        ))}
      </div>
    </motion.div>
  );
}
