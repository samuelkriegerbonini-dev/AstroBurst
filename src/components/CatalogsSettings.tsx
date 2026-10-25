import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Database, FolderOpen, RefreshCw, Trash2, X, Download } from "lucide-react";
import {
  cancelCatalogInstall,
  catalogStatus,
  chooseCatalogFile,
  chooseSirilCatalogDir,
  installCatalog,
  onCatalogInstallProgress,
  removeCatalog,
  zenodoConsent,
  zenodoOffer,
  type CatalogDatasetReport,
} from "../services/catalogStore";
import {
  IDLE_INSTALL,
  INSTALL_COMPLETE_STAGE,
  TIER2_ONLINE_TEXT,
  USED_DOWNLOAD_NOTE,
  catalogRowView,
  consentBlocker,
  consentText,
  installReducer,
  installedNote,
  offersZenodo,
  registersZenodoDownload,
  removeConfirmText,
  removedNote,
  sortedCatalogRows,
  zenodoBlocker,
  zenodoConfirmLabel,
  zenodoConsentText,
  zenodoFileText,
  type CatalogTone,
  type InstallAction,
  type InstallMap,
  type InstallPhase,
} from "../utils/catalogRows";

export const CATALOG_CREDITS =
  "Data credits: Tycho-2 (Høg et al. 2000, BSD-2), 2MASS (UMass and IPAC/Caltech for NASA and NSF, public domain), " +
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

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

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

export default function CatalogsSettings() {
  const [reports, setReports] = useState<CatalogDatasetReport[] | null>(null);
  const [installs, setInstalls] = useState<InstallMap>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [armedRemove, setArmedRemove] = useState<string | null>(null);
  const installsRef = useRef(installs);
  installsRef.current = installs;
  const reportsRef = useRef(reports);
  reportsRef.current = reports;

  const dispatch = useCallback((action: InstallAction) => setInstalls((prev) => installReducer(prev, action)), []);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setReports(await catalogStatus());
      setError(null);
    } catch (e: unknown) {
      setError(errorText(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let stop: (() => void) | null = null;
    let disposed = false;
    onCatalogInstallProgress((progress) => {
      dispatch({ type: "progress", progress });
      if (progress.stage === INSTALL_COMPLETE_STAGE) void refresh();
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stop = unlisten;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      stop?.();
    };
  }, [dispatch, refresh]);

  const onAsk = useCallback(
    async (id: string) => {
      const report = reportsRef.current?.find((row) => row.id === id);
      if (report && registersZenodoDownload(report)) {
        try {
          await chooseSirilCatalogDir(report.folder);
          dispatch({ type: "finished", id, note: USED_DOWNLOAD_NOTE });
        } catch (e: unknown) {
          setError(errorText(e));
        }
        await refresh();
        return;
      }
      dispatch({ type: "ask", id });
      if (!report || !offersZenodo(report)) return;
      try {
        dispatch({ type: "offer", id, offer: await zenodoOffer() });
      } catch (e: unknown) {
        dispatch({ type: "offerFailed", id, message: errorText(e) });
      }
    },
    [dispatch, refresh],
  );
  const onDecline = useCallback((id: string) => dispatch({ type: "decline", id }), [dispatch]);

  const onConfirm = useCallback(
    async (id: string) => {
      const install = installsRef.current[id];
      if (install?.phase !== "consent") return;
      const report = reportsRef.current?.find((row) => row.id === id);
      const zenodo = report !== undefined && offersZenodo(report);
      if (zenodo && !install.offer) return;
      dispatch({ type: "start", id });
      try {
        const outcome = zenodo && install.offer ? await installCatalog(id, zenodoConsent(install.offer)) : await installCatalog(id);
        dispatch({ type: "finished", id, note: installedNote(outcome) });
      } catch (e: unknown) {
        dispatch({ type: "failed", id, message: errorText(e) });
      }
      await refresh();
    },
    [dispatch, refresh],
  );

  const onCancel = useCallback(
    async (id: string) => {
      const install = installsRef.current[id];
      const adopted = install?.phase === "running" && install.adopted === true;
      dispatch({ type: "cancel", id });
      try {
        await cancelCatalogInstall(id);
      } catch (e: unknown) {
        setError(errorText(e));
      }
      if (!adopted) return;
      dispatch({ type: "failed", id, message: "cancelled" });
      await refresh();
    },
    [dispatch, refresh],
  );

  const onRemove = useCallback(
    async (id: string) => {
      const report = reports?.find((row) => row.id === id);
      if (report && removeConfirmText(report) !== null && armedRemove !== id) {
        setArmedRemove(id);
        return;
      }
      setArmedRemove(null);
      try {
        dispatch({ type: "finished", id, note: removedNote(await removeCatalog(id)) });
      } catch (e: unknown) {
        setError(errorText(e));
      }
      await refresh();
    },
    [dispatch, refresh, reports, armedRemove],
  );

  const onDisarm = useCallback(() => setArmedRemove(null), []);

  const onChooseFolder = useCallback(async () => {
    try {
      const dir = await pickPath({ directory: true, title: "Choose your Siril catalogue folder" });
      if (!dir) return;
      await chooseSirilCatalogDir(dir);
      await refresh();
    } catch (e: unknown) {
      setError(errorText(e));
    }
  }, [refresh]);

  const onChooseFile = useCallback(
    async (id: string) => {
      try {
        const path = await pickPath({ directory: false, title: "Choose an existing Siril catalogue file" });
        if (!path) return;
        await chooseCatalogFile(id, path);
        await refresh();
      } catch (e: unknown) {
        setError(errorText(e));
      }
    },
    [refresh],
  );

  return (
    <CatalogsSection
      reports={reports}
      installs={installs}
      loading={loading}
      error={error}
      armedRemove={armedRemove}
      onAsk={(id) => void onAsk(id)}
      onDecline={onDecline}
      onConfirm={(id) => void onConfirm(id)}
      onCancel={(id) => void onCancel(id)}
      onRemove={(id) => void onRemove(id)}
      onDisarm={onDisarm}
      onChooseFolder={() => void onChooseFolder()}
      onChooseFile={(id) => void onChooseFile(id)}
      onRefresh={() => void refresh()}
    />
  );
}
