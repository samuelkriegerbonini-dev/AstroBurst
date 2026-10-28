import { useCallback, useId, useState } from "react";
import { Database, FolderOpen, RefreshCw, Trash2, X, Download } from "lucide-react";
import type { CatalogDatasetReport } from "../services/catalogStore";
import { catalogStore, useCatalogStore, type CatalogStore } from "../hooks/useCatalogStore";
import {
  IDLE_INSTALL,
  TIER2_ONLINE_TEXT,
  catalogRowView,
  consentBlocker,
  consentText,
  offersZenodo,
  removeConfirmText,
  sortedCatalogRows,
  zenodoBlocker,
  zenodoConfirmLabel,
  zenodoConsentText,
  zenodoFileText,
  type CatalogTone,
  type InstallMap,
  type InstallPhase,
} from "../utils/catalogRows";

export const CATALOG_CREDITS =
  "Data credits: Tycho-2 (Høg et al. 2000; CC BY-NC 3.0 IGO per CDS and ESA, BSD-2-Clause per Debian; Credit: ESA), 2MASS (UMass and IPAC/Caltech for NASA and NSF, public domain), " +
  "Siril HEALPix catalogue format (Team free-astro, CC BY 4.0), Gaia DR3 © ESA/Gaia/DPAC (Vallenari et al. 2023, CC BY-NC 3.0 IGO), " +
  "online through VizieR (CDS, Strasbourg) or as the Siril extract downloaded from Zenodo.";

const TIER2_ROW_ID = "tier2-online";
const SIRIL_FILE_FILTER = { name: "Siril HEALPix catalogue", extensions: ["dat"] };

const TONE_CLASSES: Readonly<Record<CatalogTone, string>> = {
  ok: "text-emerald-400/80",
  muted: "text-zinc-400",
  warn: "text-amber-400",
  error: "text-red-400",
};

const ROW_CLASS = "rounded-md border border-zinc-800/50 bg-zinc-900/40 px-3 py-2 space-y-1.5";
const BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded border border-zinc-700/60 text-[10px] text-zinc-300 hover:text-zinc-100 hover:border-zinc-500 transition-colors disabled:opacity-40 disabled:cursor-not-allowed";
const CONFIRM_CLASS =
  "px-2 py-1 rounded border border-teal-500/40 bg-teal-500/10 text-[10px] text-teal-200 hover:bg-teal-500/20 transition-colors disabled:opacity-40 disabled:cursor-not-allowed";
const ARMED_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded border border-rose-500/50 bg-rose-500/20 text-[10px] text-rose-100 transition-colors";

interface CatalogRowProps {
  report: CatalogDatasetReport;
  install: InstallPhase;
  armed: boolean;
  onAsk: (id: string) => void;
  onDecline: (id: string) => void;
  onConfirm: (id: string) => void;
  onCancel: (id: string) => void;
  onRemove: (id: string) => void;
  onDisarm: () => void;
  onChooseFolder: () => void;
  onChooseFile: (id: string) => void;
}

function CatalogRow({
  report,
  install,
  armed,
  onAsk,
  onDecline,
  onConfirm,
  onCancel,
  onRemove,
  onDisarm,
  onChooseFolder,
  onChooseFile,
}: CatalogRowProps) {
  const consentTextId = useId();
  const view = catalogRowView(report, install);
  const consenting = install.phase === "consent";
  const zenodo = offersZenodo(report);
  const offer = install.phase === "consent" ? install.offer : undefined;
  const blocker = consenting ? (zenodo ? zenodoBlocker(install) : consentBlocker(report)) : null;
  const consentBody = zenodo
    ? offer
      ? zenodoConsentText(offer)
      : `${report.title}: the file list, sizes and checksums come from the Zenodo record before anything is downloaded.`
    : consentText(report);
  const confirmRemove = armed ? removeConfirmText(report) : null;
  const meta = [view.version, view.sizeText].filter(Boolean).join(" · ");
  return (
    <div data-catalog-row={view.id} className={ROW_CLASS}>
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-[9px] font-semibold uppercase tracking-wider text-teal-400 shrink-0">{view.tierLabel}</span>
          <span className="text-[11px] text-zinc-300 truncate" title={view.title}>{view.title}</span>
        </div>
        <span className="text-[10px] font-mono text-zinc-500 shrink-0">{meta}</span>
      </div>
      <div className={`text-[10px] break-words ${TONE_CLASSES[view.tone]}`}>{view.stateText}</div>
      {report.kind === "cache" && <div className="text-[10px] text-zinc-500">{TIER2_ONLINE_TEXT}</div>}
      {view.percent !== null && (
        <div
          role="progressbar"
          aria-label={`${view.title} install progress`}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={view.percent}
          className="h-1 rounded bg-zinc-800 overflow-hidden"
        >
          <div className="h-full bg-teal-500/70" style={{ width: `${view.percent}%` }} />
        </div>
      )}
      {install.phase === "idle" && install.note && <div className="text-[10px] text-zinc-400">{install.note}</div>}
      {install.phase === "failed" && (
        <div role="alert" className="text-[10px] text-red-400 break-words">
          {install.message}
        </div>
      )}
      {consenting && (
        <div
          role="dialog"
          aria-label={`Confirm the download of ${view.title}`}
          aria-describedby={consentTextId}
          className="rounded border border-teal-500/30 bg-teal-500/5 px-2.5 py-2 space-y-1.5"
        >
          <p id={consentTextId} className="text-[10px] text-zinc-300 break-words">
            {consentBody}
          </p>
          {offer && (
            <ul aria-label="Files in the Zenodo record" className="text-[9px] font-mono text-zinc-400 space-y-0.5 break-all">
              {offer.files.map((file) => (
                <li key={file.key}>{zenodoFileText(file)}</li>
              ))}
            </ul>
          )}
          {blocker && (
            <p role="alert" className="text-[10px] text-amber-400">
              {blocker}
            </p>
          )}
          <div className="flex gap-2">
            <button
              type="button"
              autoFocus={blocker === null}
              onClick={() => onConfirm(view.id)}
              disabled={blocker !== null}
              className={CONFIRM_CLASS}
            >
              {zenodo ? zenodoConfirmLabel(install) : `Download ${view.sizeText}`}
            </button>
            <button type="button" autoFocus={blocker !== null} onClick={() => onDecline(view.id)} className={BUTTON_CLASS}>
              Not now
            </button>
          </div>
        </div>
      )}
      <div className="flex flex-wrap gap-1.5">
        {view.actions.install && !consenting && (
          <button
            type="button"
            autoFocus={install.phase === "idle" && install.returnFocus === true}
            onClick={() => onAsk(view.id)}
            className={BUTTON_CLASS}
          >
            <Download size={10} />
            {view.installLabel}
          </button>
        )}
        {view.actions.cancel && (
          <button
            type="button"
            autoFocus={install.phase === "running" && !install.adopted}
            onClick={() => onCancel(view.id)}
            className={BUTTON_CLASS}
          >
            <X size={10} />
            Cancel
          </button>
        )}
        {view.actions.chooseFolder && (
          <button type="button" onClick={onChooseFolder} className={BUTTON_CLASS}>
            <FolderOpen size={10} />
            Use Siril folder
          </button>
        )}
        {view.actions.chooseFile && (
          <button type="button" onClick={() => onChooseFile(view.id)} className={BUTTON_CLASS}>
            Use file
          </button>
        )}
        {view.actions.remove && (
          <button
            type="button"
            onClick={() => onRemove(view.id)}
            onBlur={confirmRemove ? onDisarm : undefined}
            className={confirmRemove ? ARMED_CLASS : BUTTON_CLASS}
          >
            <Trash2 size={10} />
            {confirmRemove ?? view.removeLabel}
          </button>
        )}
      </div>
      <div className="text-[9px] text-zinc-600 break-words">
        Licence: {view.licence}. Credit: {view.credit}
      </div>
    </div>
  );
}

function Tier2OnlineRow() {
  return (
    <div data-catalog-row={TIER2_ROW_ID} className={ROW_CLASS}>
      <div className="flex items-center gap-2">
        <span className="text-[9px] font-semibold uppercase tracking-wider text-teal-400">Tier 2</span>
        <span className="text-[11px] text-zinc-300">Gaia DR3 online (VizieR)</span>
      </div>
      <div className="text-[10px] text-zinc-500">{TIER2_ONLINE_TEXT}</div>
      <div className="text-[10px] text-zinc-400">Online; the cache size is not reported by this build</div>
    </div>
  );
}

export interface CatalogsSectionProps {
  reports: readonly CatalogDatasetReport[] | null;
  installs: InstallMap;
  loading: boolean;
  error: string | null;
  armedRemove: string | null;
  onAsk: (id: string) => void;
  onDecline: (id: string) => void;
  onConfirm: (id: string) => void;
  onCancel: (id: string) => void;
  onRemove: (id: string) => void;
  onDisarm: () => void;
  onChooseFolder: () => void;
  onChooseFile: (id: string) => void;
  onRefresh: () => void;
}

export function CatalogsSection({ reports, installs, loading, error, armedRemove, onRefresh, ...handlers }: CatalogsSectionProps) {
  const rows = reports ? sortedCatalogRows(reports) : [];
  const hasCacheRow = rows.some((report) => report.kind === "cache");
  return (
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4 space-y-3">
      <div className="flex items-center justify-between">
        <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider flex items-center gap-1.5">
          <Database size={12} />
          Catalogs
        </h4>
        <button
          type="button"
          onClick={onRefresh}
          disabled={loading}
          aria-label="Refresh catalogue status"
          title="Refresh catalogue status"
          className="text-zinc-500 hover:text-zinc-300 transition-colors disabled:opacity-30"
        >
          <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
        </button>
      </div>
      <p className="text-[10px] text-zinc-600">
        Star catalogues for plate solving on this computer. The solver uses the deepest tier that is installed and only
        falls back to astrometry.net when you ask it to.
      </p>
      {loading && !reports && <p className="text-[10px] text-zinc-500">Reading the catalogue status...</p>}
      {rows.map((report) => (
        <CatalogRow
          key={report.id}
          report={report}
          install={installs[report.id] ?? IDLE_INSTALL}
          armed={armedRemove === report.id}
          {...handlers}
        />
      ))}
      {reports && !hasCacheRow && <Tier2OnlineRow />}
      {error && (
        <div role="alert" className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">
          {error}
        </div>
      )}
      <p className="text-[9px] text-zinc-600">{CATALOG_CREDITS}</p>
    </div>
  );
}

async function pickPath(options: { directory: boolean; title: string }): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({
    multiple: false,
    directory: options.directory,
    title: options.title,
    filters: options.directory ? undefined : [SIRIL_FILE_FILTER],
  });
  const path = Array.isArray(picked) ? picked[0] : picked;
  return path || null;
}

function removeClick(report: CatalogDatasetReport | undefined, armed: string | null, id: string): "arm" | "remove" {
  return report && removeConfirmText(report) !== null && armed !== id ? "arm" : "remove";
}

export default function CatalogsSettings({ store = catalogStore }: { store?: CatalogStore }) {
  const { reports, installs, loading, error } = useCatalogStore(store);
  const [armedRemove, setArmedRemove] = useState<string | null>(null);
  const [pickError, setPickError] = useState<string | null>(null);

  const onRemove = useCallback(
    (id: string) => {
      if (removeClick(reports?.find((row) => row.id === id), armedRemove, id) === "arm") {
        setArmedRemove(id);
        return;
      }
      setArmedRemove(null);
      void store.remove(id);
    },
    [store, reports, armedRemove],
  );

  const onDisarm = useCallback(() => setArmedRemove(null), []);

  const pick = useCallback(async (options: { directory: boolean; title: string }, use: (path: string) => Promise<void>) => {
    setPickError(null);
    try {
      const path = await pickPath(options);
      if (path) await use(path);
    } catch (e: unknown) {
      setPickError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  return (
    <CatalogsSection
      reports={reports}
      installs={installs}
      loading={loading}
      error={pickError ?? error}
      armedRemove={armedRemove}
      onAsk={(id) => void store.ask(id)}
      onDecline={(id) => store.decline(id)}
      onConfirm={(id) => void store.confirm(id)}
      onCancel={(id) => void store.cancel(id)}
      onRemove={onRemove}
      onDisarm={onDisarm}
      onChooseFolder={() => void pick({ directory: true, title: "Choose your Siril catalogue folder" }, (dir) => store.useSirilDir(dir))}
      onChooseFile={(id) => void pick({ directory: false, title: "Choose an existing Siril catalogue file" }, (path) => store.useFile(id, path))}
      onRefresh={() => void store.refresh()}
    />
  );
}
