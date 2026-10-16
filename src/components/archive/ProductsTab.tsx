import { useState } from "react";
import { AlertTriangle, ChevronDown, ChevronRight, RefreshCw } from "lucide-react";
import type { MastMission, MastProductClass, MastRow } from "../../shared/types/mast";
import { imageSetKey, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { useNow } from "./state/useNow";
import { PRODUCTS_SLOW_NOTE, productsFailureText, productsListingText } from "./logic/detailView";
import { viewAvailability } from "./logic/missionState";
import { errorView } from "./logic/errorCopy";
import {
  CLASS_LABELS,
  copyDownloadUrls,
  copyMastUris,
  exactBytes,
  formatBytes,
  opensParts,
  productView,
  sectionExpanded,
  shortFilename,
  visibleProducts,
  type ProductSection,
  type SectionOverrides,
} from "./logic/productView";

interface ProductsTabProps {
  state: ArchiveState;
  ctl: ArchiveController;
  row: MastRow;
  mission: MastMission;
}

const DOWNLOAD_NOTE = "Downloading inside AstroBurst arrives with the download engine (v1.1).";
const FILE_NOUNS: Partial<Record<MastProductClass, string>> = { reference: "CRDS reference", guidestar: "guide-star" };

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

function SectionDetail({ section }: { section: ProductSection }) {
  if (section.classes.length === 1) return <span className="ab-mast-pgroup-detail">{section.detail}</span>;
  if (section.key === "opens") {
    const parts = opensParts(section);
    return (
      <span className="ab-mast-pgroup-detail">
        {parts.map((p, i) => (
          <span key={p.cls} data-group={p.cls} data-count={p.count}>
            {i > 0 ? `, ${p.text}` : p.text}
          </span>
        ))}
        {`, ${formatBytes(section.bytes)}`}
      </span>
    );
  }
  const counts = section.classes.map((cls) => [cls, section.products.filter((p) => p.cls === cls).length] as [MastProductClass, number]);
  return (
    <span className="ab-mast-pgroup-detail">
      {counts.map(([cls, n], i) => (
        <span key={cls} data-group={cls} data-count={n}>
          {`${i > 0 ? ", " : ""}${plural(n, `${FILE_NOUNS[cls] ?? CLASS_LABELS[cls]} file`, `${FILE_NOUNS[cls] ?? CLASS_LABELS[cls]} files`)}`}
        </span>
      ))}
      {`, ${formatBytes(section.bytes)}`}
    </span>
  );
}

function SectionView({ section, expanded, onToggle }: { section: ProductSection; expanded: boolean; onToggle: () => void }) {
  const single = section.classes.length === 1;
  return (
    <section className="ab-mast-pgroup" data-section={section.key} data-group={single ? section.classes[0] : undefined} data-count={single ? section.count : undefined} aria-label={section.title}>
      <button type="button" className="ab-mast-pgroup-head" aria-expanded={expanded} onClick={onToggle}>
        {expanded ? <ChevronDown size={11} aria-hidden="true" /> : <ChevronRight size={11} aria-hidden="true" />}
        <span className="ab-mast-pgroup-title">{section.title}</span>
        <SectionDetail section={section} />
        {section.note ? <span className="ab-mast-pgroup-note">{`(${section.note})`}</span> : null}
      </button>
      {expanded ? (
        <ul className="ab-mast-plist">
          {section.products.map((p) => (
            <li key={`${p.datasetId}:${p.filename}`} className="ab-mast-prow" data-cls={p.cls}>
              <span className="ab-mast-prow-name ab-mast-mono" title={p.filename}>
                {shortFilename(p.filename, p.datasetId)}
              </span>
              <span className="ab-mast-prow-cls">{CLASS_LABELS[p.cls]}</span>
              <span className="ab-mast-prow-size ab-mast-mono" title={exactBytes(p.sizeBytes)}>
                {formatBytes(p.sizeBytes)}
              </span>
              {p.badges.includes("hap_aligned") ? <span className="ab-mast-badge">HAP aligned</span> : null}
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}

export default function ProductsTab({ state, ctl, row, mission }: ProductsTabProps) {
  const [overrides, setOverrides] = useState<SectionOverrides>({});
  const key = imageSetKey(mission.id, row.datasetId);
  const products = state.products[key];
  const loading = !products || products.stage === "loading";
  const now = useNow(products?.stage === "loading" ? 500 : null);
  const avail = viewAvailability(mission);

  if (!avail.products) {
    return (
      <div data-testid="mast-products" data-state="unavailable" className="ab-mast-products">
        <p className="ab-mast-detail-note">{avail.productsReason}</p>
      </div>
    );
  }
  if (loading) {
    return (
      <div data-testid="mast-products" data-state={products ? "loading" : "idle"} className="ab-mast-products" role="status">
        <p>{productsListingText(row.datasetId, products ? Math.max(0, now - products.startedAt) : 0)}</p>
        <p className="ab-mast-detail-note">{PRODUCTS_SLOW_NOTE}</p>
        {products ? (
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.cancelProducts()}>
            Cancel
          </button>
        ) : null}
      </div>
    );
  }
  if (products.stage === "cancelled") {
    return (
      <div data-testid="mast-products" data-state="cancelled" className="ab-mast-products">
        <p>Product listing stopped.</p>
        <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.retryDetail()}>
          List products
          <kbd className="ab-cmdp-kbd">R</kbd>
        </button>
      </div>
    );
  }
  if (products.stage === "failed" || !products.list) {
    const err = products.error;
    const kind = err ? errorView(err, { surface: "products" }).bannerKind : undefined;
    return (
      <div data-testid="mast-products" data-state="failed" className="ab-mast-products">
        <div className="ab-mast-banner" data-kind={kind ?? "server"} role="alert">
          <AlertTriangle size={13} aria-hidden="true" />
          <span className="ab-mast-banner-text">{err ? productsFailureText(err) : "MAST answered without a product list."}</span>
          <button type="button" className="ab-mast-link-btn" onClick={() => ctl.retryDetail()}>
            Retry
            <kbd className="ab-cmdp-kbd">R</kbd>
          </button>
        </div>
      </div>
    );
  }

  const view = productView(products.list, state.productMode, state.productHidden);
  const visible = visibleProducts(view, overrides);
  const setMode = (mode: "opens" | "everything") => {
    setOverrides({});
    ctl.setProductMode(mode);
  };

  return (
    <div data-testid="mast-products" data-state="ready" className="ab-mast-products">
      <p className="ab-mast-products-summary">{view.summary}</p>
      <div className="ab-mast-products-bar">
        <fieldset className="ab-mast-radio">
          <legend className="ab-mast-filter-label">Show</legend>
          <label className="ab-mast-check">
            <input type="radio" name="mast-products-mode" checked={state.productMode === "opens"} onChange={() => setMode("opens")} />
            What the app opens
          </label>
          <label className="ab-mast-check">
            <input type="radio" name="mast-products-mode" checked={state.productMode === "everything"} onChange={() => setMode("everything")} />
            Everything
          </label>
        </fieldset>
        <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.refreshProducts()} title="List again, past the 24 h cache">
          <RefreshCw size={11} aria-hidden="true" />
          Refresh
        </button>
      </div>
      {view.sections.map((section) => (
        <SectionView
          key={section.key}
          section={section}
          expanded={sectionExpanded(section, overrides)}
          onToggle={() => setOverrides((o) => ({ ...o, [section.key]: !sectionExpanded(section, o) }))}
        />
      ))}
      {view.hidden.count > 0 ? (
        <div className="ab-mast-hidden">
          <p data-testid="mast-hidden-summary" data-reference={view.hidden.reference} data-guidestar={view.hidden.guidestar} className="ab-mast-hidden-text">
            {`Hidden: ${view.hidden.text}`}
          </p>
          <button type="button" className="ab-btn-secondary ab-mast-action" aria-pressed={state.productHidden} onClick={() => ctl.toggleHiddenProducts()}>
            {state.productHidden ? "Hide them again" : "Show hidden"}
          </button>
        </div>
      ) : null}
      <div className="ab-mast-actions">
        <button type="button" className="ab-btn-secondary ab-mast-action" disabled={visible.length === 0} onClick={() => void ctl.copy(copyMastUris(visible), plural(visible.length, "MAST URI", "MAST URIs"))}>
          Copy MAST URIs
        </button>
        <button
          type="button"
          className="ab-btn-secondary ab-mast-action"
          disabled={visible.length === 0}
          onClick={() => void ctl.copy(copyDownloadUrls(visible), plural(visible.length, "Download/file URL", "Download/file URLs"))}
        >
          Copy Download/file URLs
        </button>
      </div>
      <p className="ab-mast-detail-note">{DOWNLOAD_NOTE}</p>
    </div>
  );
}
