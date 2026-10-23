import { memo, useState } from "react";
import { FolderOpen } from "lucide-react";
import { SHOW_IN_FOLDER, outputNote } from "../../utils/outputNote";
import { revealInFolder } from "../../utils/revealInFolder";

function OutputNote({ path }: { path: string }) {
  const [revealError, setRevealError] = useState<string | null>(null);
  const note = outputNote(path);
  return (
    <div data-output-note data-output-path={path} className="flex flex-col gap-0.5 text-[9px] text-zinc-500">
      <div className="flex items-center gap-2 min-w-0">
        <span className="font-mono text-zinc-400 truncate" title={path}>{note.name}</span>
        <button
          type="button"
          onClick={() => void revealInFolder(path).then(setRevealError)}
          className="ml-auto shrink-0 flex items-center gap-1 text-zinc-500 hover:text-zinc-300"
        >
          <FolderOpen size={9} />
          {SHOW_IN_FOLDER}
        </button>
      </div>
      <span className="leading-snug">{note.warning}</span>
      {revealError && (
        <span role="alert" className="text-red-400 break-words">
          {revealError}
        </span>
      )}
    </div>
  );
}

export default memo(OutputNote);
