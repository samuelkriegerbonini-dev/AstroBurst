import { memo, useState } from "react";
import { FolderOpen } from "lucide-react";
import { SHOW_IN_FOLDER, outputNote } from "../../utils/outputNote";
import { revealInFolder } from "../../utils/revealInFolder";
import type { ScientificProvenance } from "../../shared/types/cube";

const MANIFEST_WARNING = "In the output folder, which is cleared when AstroBurst starts. Export > FITS does not copy this manifest, so keep a copy next to the exported files.";

function OutputNote({ path, kind, provenance }: { path: string; kind?: "manifest"; provenance?: ScientificProvenance }) {
  const [revealFailure, setRevealFailure] = useState<{ path: string; message: string | null } | null>(null);
  const revealError = revealFailure?.path === path ? revealFailure.message : null;
  const note = outputNote(path);
  const source = provenance?.source;
  const verification = provenance?.integrity === "sha256_before_open_and_before_publication"
    ? "SHA-256 before reading and before publishing"
    : provenance?.integrity;
  return (
    <div data-output-note data-output-path={path} className="flex flex-col gap-0.5 text-[9px] text-zinc-500">
      <div className="flex items-center gap-2 min-w-0">
        <span className="font-mono text-zinc-400 truncate" title={path}>{note.name}</span>
        <button
          type="button"
          onClick={() => void revealInFolder(path).then((message) => setRevealFailure({ path, message }))}
          className="ml-auto shrink-0 flex items-center gap-1 text-zinc-500 hover:text-zinc-300"
        >
          <FolderOpen size={9} />
          {SHOW_IN_FOLDER}
        </button>
      </div>
      <span className="leading-snug">{kind === "manifest" ? MANIFEST_WARNING : note.warning}</span>
      {source?.sha256 && <span className="font-mono break-all" title={`${source.input_ref}\n${source.sha256}`}>Input SHA-256: {source.sha256.slice(0, 12)}{source.sha256.length > 12 ? "…" : ""}</span>}
      {verification && <span className="leading-snug">Verification: {verification}</span>}
      {provenance?.integrity_limit && <span className="leading-snug">{provenance.integrity_limit}</span>}
      {revealError && (
        <span role="alert" className="text-red-400 break-words">
          {revealError}
        </span>
      )}
    </div>
  );
}

export default memo(OutputNote);
