import { useState } from "react";
import { Copy } from "lucide-react";
import type { MastMission, MastRow } from "../../shared/types/mast";
import type { ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { allColumns, metadataSections, type MetaRow } from "./logic/metadataView";
import { curlCommand, requestJson, rowTsv } from "./logic/copyBuilders";

interface MetadataTabProps {
  state: ArchiveState;
  ctl: ArchiveController;
  row: MastRow;
  mission: MastMission;
}

const SHORT_COPY = 40;

function copyLabel(r: MetaRow, text: string): string {
  return text.length > SHORT_COPY ? `the ${r.label}` : text;
}

export default function MetadataTab({ state, ctl, row, mission }: MetadataTabProps) {
  const [filter, setFilter] = useState("");
  const results = state.results;
  const sections = metadataSections({ row, resolved: state.resolved, searchPosition: results?.searchPosition ?? null }).filter((s) => s.rows.length > 0);
  const columns = results ? allColumns(row, results.columns, filter) : [];
  const total = results?.columns.length ?? 0;
  const body = results?.requestBody ?? null;

  return (
    <div className="ab-mast-meta">
      {sections.map((section) => (
        <section key={section.key} className="ab-mast-meta-section" aria-label={section.title}>
          <h3 className="ab-mast-meta-title">{section.title}</h3>
          <dl className="ab-mast-meta-list">
            {section.rows.map((r) => {
              const copy = r.copy;
              return (
                <div key={r.label} className="ab-mast-meta-row">
                  <dt className="ab-mast-meta-label">{r.label}</dt>
                  <dd className="ab-mast-meta-cell">
                    <span className="ab-mast-meta-value" title={r.value}>
                      {r.value}
                    </span>
                    {r.note ? <span className="ab-mast-meta-note">{r.note}</span> : null}
                  </dd>
                  {copy ? (
                    <button type="button" className="ab-mast-icon-btn" aria-label={`Copy ${r.label}`} title={`Copy ${r.label}`} onClick={() => void ctl.copy(copy, copyLabel(r, copy))}>
                      <Copy size={11} aria-hidden="true" />
                    </button>
                  ) : null}
                </div>
              );
            })}
          </dl>
        </section>
      ))}
      <section className="ab-mast-meta-section" aria-label="Query">
        <h3 className="ab-mast-meta-title">Query</h3>
        <div className="ab-mast-actions">
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => void ctl.copy(requestJson(body), "the request JSON")}>
            Copy request JSON
          </button>
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => void ctl.copy(curlCommand(mission.id, body), "the curl command")}>
            Copy curl
          </button>
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => void ctl.copy(rowTsv(row, results?.columns ?? []), "the row as TSV")}>
            Copy row as TSV
          </button>
        </div>
      </section>
      <section className="ab-mast-meta-section" aria-label="All columns">
        <div className="ab-mast-meta-head">
          <h3 className="ab-mast-meta-title">{`All columns (${total})`}</h3>
          <input
            type="search"
            className="ab-mast-text ab-mast-text-columns"
            placeholder="Filter columns"
            aria-label="Filter columns"
            spellCheck={false}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>
        <dl className="ab-mast-meta-list">
          {columns.map((c) => (
            <div key={c.name} className="ab-mast-meta-row">
              <dt className="ab-mast-meta-label ab-mast-mono" title={c.label}>
                {c.name}
              </dt>
              <dd className="ab-mast-meta-cell">
                <span className="ab-mast-meta-value ab-mast-mono" title={c.value}>
                  {c.value}
                </span>
              </dd>
            </div>
          ))}
        </dl>
        {columns.length === 0 && filter ? <p className="ab-mast-detail-note">{`No column matches "${filter}".`}</p> : null}
      </section>
    </div>
  );
}
