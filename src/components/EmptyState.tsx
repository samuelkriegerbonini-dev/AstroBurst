import { Telescope, Upload, FolderOpen, Archive, Sparkles, X } from "lucide-react";
import { SUPPORTED_EXTENSIONS } from "../utils/validation";
import { formatBytes } from "./archive/logic/productView";
import ProgressBar from "./file/ProgressBar";
import type { SampleControls } from "../hooks/useSampleData";
import { sampleDataHint, sampleOfferAction, samplePercent } from "../utils/sampleOffers";
import { createFocusHandoff } from "../utils/focusHandoff";

interface EmptyStateProps {
  onBrowseFiles: () => void;
  onSelectFolder: () => void;
  onBrowseArchive?: () => void;
  sample?: SampleControls;
}

const FORMAT_BADGES = SUPPORTED_EXTENSIONS.map((ext) => `.${ext}`);
const sampleFocus = createFocusHandoff();

export default function EmptyState({ onBrowseFiles, onSelectFolder, onBrowseArchive, sample }: EmptyStateProps) {
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

  const handleArchive = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    e.stopPropagation();
    onBrowseArchive?.();
  };

  const sampleAction = sample ? sampleOfferAction(sample) : null;
  const sampleHint = sample ? sampleDataHint(sample) : undefined;
  const SampleIcon = sampleAction?.icon ?? Sparkles;
  const percent = sample ? samplePercent(sample) : 0;

  const handleSampleData = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    e.stopPropagation();
    sampleAction?.run();
  };

  const handleSampleCancel = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    e.stopPropagation();
    sample?.cancel();
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

            <div className="flex flex-wrap justify-center gap-3">
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
              {onBrowseArchive && (
                <button type="button" onClick={handleArchive} className="ab-btn-secondary" data-testid="empty-browse-archive">
                  <Archive size={16} />
                  Browse the MAST archive
                  <kbd
                    className="ml-1 text-[9px] font-mono px-1.5 py-0.5 rounded"
                    style={{ background: "rgba(255,255,255,0.08)", color: "var(--ab-text-2)" }}
                  >
                    {isMac ? "⌘2" : "Ctrl+2"}
                  </kbd>
                </button>
              )}
              {sampleAction && (
                <button type="button" ref={sampleFocus.arriving} onClick={handleSampleData} className="ab-btn-secondary" data-testid="empty-open-sample" title={sampleHint}>
                  <SampleIcon size={16} />
                  {sampleAction.label}
                </button>
              )}
              {sample?.state === "downloading" && (
                <div className="flex items-center gap-3">
                  <div
                    role="progressbar"
                    aria-label="Sample data download"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={percent}
                    aria-valuetext={`${formatBytes(sample.receivedBytes)} of ${formatBytes(sample.totalBytes)}`}
                    data-testid="empty-sample-progress"
                    className="w-40"
                    title={sample.file ?? undefined}
                  >
                    <ProgressBar value={percent} />
                  </div>
                  <span className="text-zinc-400 text-xs font-mono tabular-nums">{percent}%</span>
                  <button type="button" ref={sampleFocus.leaving} onClick={handleSampleCancel} className="ab-btn-secondary" data-testid="empty-sample-cancel" aria-label="Cancel sample download">
                    <X size={16} />
                    Cancel
                  </button>
                </div>
              )}
            </div>
            {sample && (
              <div className="flex flex-col items-center gap-1 -mt-2 max-w-md text-center text-xs">
                {sample.error && (
                  <p className="text-red-400" role="alert" data-testid="empty-sample-error">{sample.error}</p>
                )}
                <p className="text-zinc-500">{sampleHint}</p>
                <p className="text-zinc-600">{sample.credit}</p>
              </div>
            )}
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
