import { Fragment, memo, useEffect, useMemo, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { ErrorAlert, SMALL_BUTTON_CLASS } from "../ui";
import { tsoStore, useTsoCurves, type TsoSectionProps } from "../../hooks/useTsoStore";
import { tsoWhtltCompare, whtltImport } from "../../services/tso";
import type { WhtltComparison, WhtltImport } from "../../shared/types/tso";
import { comparisonRowText, defaultWhtltPath, whtltDocs } from "../../utils/tsoWhtlt";
import { fileBaseName } from "../../utils/values";

const SECTION_CLASS = "text-[9px] text-zinc-500 uppercase tracking-wider";
const LINE_CLASS = "text-[10px] font-mono text-zinc-400 break-words";
const CAPTION_CLASS = "text-[9px] text-zinc-500";
const HEAD_CLASS = "text-left px-1.5 py-0.5";
const CELL_CLASS = "text-left px-1.5 py-0.5 text-zinc-300";
const DIFFERS_CLASS = "text-left px-1.5 py-0.5 text-amber-300";
const WHTLT_FILTER = { name: "White-light table", extensions: ["ecsv"] };
const COLUMNS = ["column", "series", "rows paired", "largest relative difference", "verdict"];
const PAIRING_CAPTION = "The comparison uses the pipeline mode with the pipeline's limits; rows are paired on time, never on position.";
const OVERLAY_HINT = 'Choose "white light and whtlt" in the series list of the plot to see both curves.';
const NOT_DRAWN =
  "This table cannot be drawn over the curves: the series has no time axis, no single column of the table belongs to it, or the table has no times on the time scale of the series.";

interface WhtltLoad {
  loading: boolean;
  rows: WhtltComparison[] | null;
  imported: WhtltImport | null;
  error: string | null;
}

const NO_LOAD: WhtltLoad = { loading: false, rows: null, imported: null, error: null };

function TsoWhtltSection({ fileKey, paths, set, group }: TsoSectionProps) {
  const [chosen, setChosen] = useState<{ fileKey: string; path: string } | null>(null);
  const [load, setLoad] = useState<WhtltLoad>(NO_LOAD);
  const seqRef = useRef(0);
  const curves = useTsoCurves(group);
  const pathsKey = paths.join("\n");
  const keyedPathsRef = useRef({ key: pathsKey, paths });
  if (keyedPathsRef.current.key !== pathsKey) keyedPathsRef.current = { key: pathsKey, paths };
  const keyedPaths = keyedPathsRef.current.paths;
  const path = (chosen?.fileKey === fileKey ? chosen.path : null) ?? defaultWhtltPath(set.files);

  useEffect(() => {
    const seq = ++seqRef.current;
    if (path === null) {
      setLoad(NO_LOAD);
      return;
    }
    setLoad({ ...NO_LOAD, loading: true });
    Promise.all([tsoWhtltCompare([...keyedPaths], path), whtltImport(path)]).then(
      ([rows, imported]) => {
        if (seqRef.current === seq) setLoad({ loading: false, rows, imported, error: null });
      },
      (reason: unknown) => {
        if (seqRef.current === seq) setLoad({ ...NO_LOAD, error: reason instanceof Error ? reason.message : String(reason) });
      },
    );
  }, [path, keyedPaths]);

  const extraDocs = useMemo(
    () => (load.imported && curves.result ? whtltDocs(load.imported, curves.result, keyedPaths) : []),
    [load.imported, curves.result, keyedPaths],
  );

  useEffect(() => {
    tsoStore.setExtraDocs(group, extraDocs);
  }, [group, extraDocs]);

  const choose = async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ multiple: false, title: "Choose the whtlt table of this observation", filters: [WHTLT_FILTER] });
    if (typeof picked === "string") setChosen({ fileKey, path: picked });
  };

  return (
    <div className="px-3 py-2 space-y-2" data-tso-whtlt>
      <div className="flex items-center gap-2">
        <span className={SECTION_CLASS}>Pipeline white light (whtlt)</span>
        {load.loading && <Loader2 size={12} className="animate-spin text-amber-400/70" />}
      </div>
      <div className="flex items-center justify-between gap-2">
        <span className={LINE_CLASS}>
          {path === null ? "No _whtlt.ecsv was found beside this file." : `Compared with ${fileBaseName(path)}`}
        </span>
        <button type="button" onClick={() => void choose()} className={SMALL_BUTTON_CLASS}>
          {path === null ? "Choose whtlt file" : "Choose another file"}
        </button>
      </div>
      <ErrorAlert message={load.error} />
      {load.rows && (
        <div className="overflow-auto border border-zinc-800/60 rounded">
          <table className="w-full text-[9px] font-mono" data-tso-whtlt-table>
            <thead className="text-zinc-500">
              <tr>
                {COLUMNS.map((name) => (
                  <th key={name} className={HEAD_CLASS}>
                    {name}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {load.rows.map((row) => {
                const text = comparisonRowText(row);
                const lines = [...(text.limits === null ? [] : [`summed over ${text.limits}`]), ...row.notes];
                return (
                  <Fragment key={row.column}>
                    <tr>
                      <td className={CELL_CLASS}>{text.column}</td>
                      <td className={CELL_CLASS}>{text.series}</td>
                      <td className={CELL_CLASS}>{text.rows}</td>
                      <td className={CELL_CLASS}>{text.difference}</td>
                      <td className={text.verdict === "differs" ? DIFFERS_CLASS : CELL_CLASS}>{text.verdict}</td>
                    </tr>
                    {lines.length > 0 && (
                      <tr>
                        <td colSpan={COLUMNS.length} className="px-1.5 pb-1">
                          {lines.map((line, i) => (
                            <div key={i} className={`${CAPTION_CLASS} break-words`} data-tso-whtlt-note>
                              {line}
                            </div>
                          ))}
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      <div className={CAPTION_CLASS}>{PAIRING_CAPTION}</div>
      {extraDocs.length > 0 ? (
        <div className={CAPTION_CLASS} data-tso-whtlt-overlay>
          {OVERLAY_HINT}
        </div>
      ) : (
        load.imported &&
        curves.result && (
          <div className={CAPTION_CLASS} data-tso-whtlt-overlay>
            {NOT_DRAWN}
          </div>
        )
      )}
    </div>
  );
}

export default memo(TsoWhtltSection);
