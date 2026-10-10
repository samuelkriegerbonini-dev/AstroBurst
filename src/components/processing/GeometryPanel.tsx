import { useCallback, type ReactNode } from "react";
import { ArrowLeftRight, RefreshCw, RotateCcw } from "lucide-react";
import { ResultGrid, ErrorAlert, SectionHeader } from "../ui";
import { useDisplayedImage } from "../../context/PreviewContext";
import { bustPreviewUrl, useProcessingRun } from "../../hooks/useProcessingRun";
import { ingestUnlessLoaded } from "../../hooks/useFileIngest";
import { GEOMETRY_OPS, transformGeometry, type GeometryOp, type GeometryResult } from "../../services/imageGeometry";
import { fileOnlyNotice, type CompositeNoticeProps } from "./compositeProps";

interface GeometryPanelProps extends CompositeNoticeProps {
  selectedFile: { path: string; name?: string; result?: { previewUrl?: string | null } | null } | null;
  outputDir: string;
  fileKey?: string | null;
}

interface GeometryRun {
  res: GeometryResult;
  beforeUrl: string | null;
  afterUrl: string | undefined;
}

const OP_LABELS: Record<GeometryOp, { short: string; title: string; icon: ReactNode }> = {
  rot90: { short: "90° CW", title: "Rotate 90° clockwise", icon: <RotateCcw size={12} className="-scale-x-100" /> },
  rot180: { short: "180°", title: "Rotate 180°", icon: <RefreshCw size={12} /> },
  rot270: { short: "90° CCW", title: "Rotate 90° counter-clockwise (270° clockwise)", icon: <RotateCcw size={12} /> },
  flip_h: { short: "Flip H", title: "Mirror left-right", icon: <ArrowLeftRight size={12} /> },
  flip_v: { short: "Flip V", title: "Mirror top-bottom", icon: <ArrowLeftRight size={12} className="rotate-90" /> },
};

const ICON = <RotateCcw size={14} className="text-orange-400 -scale-x-100" />;

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function geometryResultLine(res: Pick<GeometryResult, "fits_path" | "dimensions">): string {
  return `Written ${baseName(res.fits_path)} (${res.dimensions[0]}x${res.dimensions[1]}); select it in Files to continue.`;
}

export default function GeometryPanel({ selectedFile, outputDir, fileKey, compositeMode, fileName }: GeometryPanelProps) {
  const displayed = useDisplayedImage();
  const { running, blocked, busyTitle, result, error, run } = useProcessingRun<GeometryRun>("geometry", fileKey ?? null);
  const inputPath = selectedFile ? displayed.path : null;
  const canRun = inputPath !== null;

  const handleOp = useCallback(
    (op: GeometryOp) => {
      if (!inputPath) return;
      const path = inputPath;
      const beforeUrl = displayed.previewOnly ? (selectedFile?.result?.previewUrl ?? null) : displayed.previewUrl;
      void run(async () => {
        const res = await transformGeometry(path, outputDir, op);
        ingestUnlessLoaded(res.fits_path);
        return { res, beforeUrl, afterUrl: bustPreviewUrl(res.previewUrl, Date.now()) };
      });
    },
    [inputPath, displayed.previewOnly, displayed.previewUrl, selectedFile, outputDir, run],
  );

  return (
    <div className="flex flex-col gap-3 p-3">
      <SectionHeader icon={ICON} title="Geometry" subtitle="Exact rotations and flips, written as a new file with the WCS updated" />

      {compositeMode && (
        <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-[11px] text-amber-300">
          {fileOnlyNotice("Geometry acts on", fileName)}
        </div>
      )}
      {!selectedFile && <div className="text-[10px] text-zinc-500 italic">Select a FITS file first.</div>}
      {inputPath && (
        <div className="text-[10px] text-zinc-500 truncate" title={inputPath}>
          Input: <span className="text-zinc-300">{displayed.previewOnly ? baseName(inputPath) : (displayed.label ?? baseName(inputPath))}</span>
        </div>
      )}

      <div className="grid grid-cols-5 gap-1.5" title={busyTitle}>
        {GEOMETRY_OPS.map((op) => (
          <button
            key={op}
            type="button"
            data-testid={`geometry-op-${op}`}
            title={OP_LABELS[op].title}
            disabled={!canRun || running || blocked}
            onClick={() => handleOp(op)}
            className="flex flex-col items-center gap-1 rounded-md bg-zinc-800/50 px-1 py-2 text-[10px] text-zinc-300 transition-colors hover:bg-orange-600/20 hover:text-orange-300 disabled:opacity-40 disabled:hover:bg-zinc-800/50 disabled:hover:text-zinc-300"
          >
            {OP_LABELS[op].icon}
            {OP_LABELS[op].short}
          </button>
        ))}
      </div>
      {running && <div className="text-[10px] text-zinc-500">Writing the transformed file...</div>}
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-2 animate-fade-in">
          <div data-testid="geometry-result" className="text-[11px] text-emerald-300">{geometryResultLine(result.res)}</div>
          <ResultGrid items={[
            { label: "Operation", value: OP_LABELS[result.res.op]?.title ?? result.res.op },
            { label: "WCS", value: result.res.wcs_updated ? "updated" : "none" },
            { label: "Time", value: `${(result.res.elapsed_ms / 1000).toFixed(2)}s` },
          ]} />
          {result.beforeUrl && result.afterUrl && (
            <div className="grid grid-cols-2 gap-2">
              <figure className="flex flex-col gap-1">
                <img src={result.beforeUrl} alt="Before" className="h-36 w-full rounded bg-zinc-900 object-contain" draggable={false} />
                <figcaption className="text-center text-[10px] text-zinc-500">Before</figcaption>
              </figure>
              <figure className="flex flex-col gap-1">
                <img src={result.afterUrl} alt="After" className="h-36 w-full rounded bg-zinc-900 object-contain" draggable={false} />
                <figcaption className="text-center text-[10px] text-zinc-500">After</figcaption>
              </figure>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
