import type {
  MastError,
  MastErrorCode,
  MastImageEntry,
  MastMissionId,
  MastImageSet,
  MastMission,
  MastPreviewRef,
  MastProductList,
  MastQuery,
  MastResolved,
  MastRow,
  MastSearchPage,
  MastTokenStatus,
} from "../../../shared/types/mast";
import { DEFAULT_FORM, type ArchiveForm, type SortKey } from "../logic/buildQuery";
import type { QueryKind } from "../logic/interpretQuery";
import { mergePage, type ResultSet } from "../logic/paging";
import {
  NOT_DEPLOYED_STATUS,
  clockText,
  resultsGate,
  romanLock,
  viewAvailability,
  type MissionProbes,
  type RomanLock,
  type TokenView,
} from "../logic/missionState";
import {
  createThumbQueue,
  forget,
  markDropped,
  markFailed,
  markRateLimited,
  markReady,
  markStarted,
  resumeNow,
  retryFailed,
  setActive,
  setDemand,
  tickQueue,
  type ThumbQueue,
} from "../logic/thumbQueue";
import {
  CACHE_TROUBLE_TEXT,
  MISSING_FILE,
  ROMAN_CONFIRMED_TEXT,
  ROMAN_NO_ACCESS_TEXT,
  THUMBS_GAVE_UP_TEXT,
  TOKEN_CHECK_UNREACHABLE_TEXT,
  errorView,
  rateLimitText,
  slotError,
  tokenSaveFailureText,
  type BannerKind,
} from "../logic/errorCopy";
import { rowKeyMap, type TableSort } from "../logic/resultsView";
import { DETAIL_WIDTH_DEFAULT, clampDetailWidth } from "../logic/detailLayout";
import type { ProductMode } from "../logic/productView";
import { TOKEN_REMOVED_TEXT, type RomanPanelState } from "../logic/romanPanel";
import { missionLabelOf as staticMissionLabel } from "../logic/missionLabels";

export type ResultsState =
  | "idle"
  | "resolving"
  | "searching"
  | "ready"
  | "empty"
  | "not-resolved"
  | "error"
  | "locked"
  | "unavailable"
  | "no-images";

export type { RomanPanelState };
export type ResultsView = "gallery" | "table";
export type DetailTab = "previews" | "metadata" | "products";
export type BannerRetry = "search" | "page" | "thumbs" | "resume" | null;
export type TokenStep = "info" | "auth";
export type PreviewStage = "download" | "decode" | "ready" | "failed";
export type ProductsStage = "loading" | "ready" | "failed" | "cancelled";
export type DetailMode = "pane" | "drawer";

export interface Banner {
  kind: BannerKind;
  text: string;
  retry: BannerRetry;
}

interface StoredBanner extends Banner {
  until: number | null;
  status: number | null;
}

export interface SearchRun {
  generation: number;
  text: string;
  kind: QueryKind;
  name: string | null;
  query: MastQuery | null;
  phase: "resolving" | "searching";
  resolved: MastResolved | null;
  startedAt: number;
  searchStartedAt: number;
}

export type TokenFocusTarget = "field" | "outcome" | "panel";

export interface TokenState {
  status: MastTokenStatus | null;
  checking: boolean;
  step: TokenStep | null;
  expired: boolean;
  outcome: "rejected" | "unreachable" | null;
  message: string | null;
  checkedAt: number | null;
  replacing: boolean;
  expanded: boolean;
  focus: { target: TokenFocusTarget; seq: number } | null;
  focusSeq: number;
}

export interface DetailState {
  imageKey: string | null;
  group: string | null;
  tab: DetailTab;
  hidden: boolean;
  drawerOpen: boolean;
  width: number;
  narrow: boolean;
}

export interface ListState {
  requestId: string;
  startedAt: number;
  endedAt: number | null;
  error: MastError | null;
}

export interface ProductsState {
  requestId: string;
  stage: ProductsStage;
  startedAt: number;
  list: MastProductList | null;
  error: MastError | null;
}

export interface LargeState {
  open: boolean;
  datasetId: string | null;
  imageKey: string | null;
  group: string | null;
  filter: string | null;
}

export interface PreviewState {
  requestId: string;
  stage: PreviewStage;
  received: number;
  total: number | null;
  startedAt: number;
  endedAt: number | null;
  ref: MastPreviewRef | null;
  url: string | null;
  error: MastError | null;
}

export interface ThumbRef {
  url: string;
  width: number;
  height: number;
  imageCount: number | null;
  filename: string;
}

export interface Unavailable {
  mission: MastMission["id"];
  text: string;
  at: number;
  status: number | null;
  operation: string | undefined;
}

export interface UnavailableInfo {
  mission: MastMissionId;
  label: string;
  operation: string | undefined;
  at: number | null;
  status: number | null;
  checkedOn: string;
}

export interface ArchiveState {
  missions: MastMission[];
  form: ArchiveForm;
  generation: number;
  run: SearchRun | null;
  results: ResultSet | null;
  resolved: MastResolved | null;
  notResolved: { name: string; elapsedMs: number } | null;
  error: MastError | null;
  queryError: { text: string; field: "query" | "radius" } | null;
  unavailable: Unavailable | null;
  probes: MissionProbes;
  selectedId: string | null;
  selectedKey: string | null;
  view: ResultsView;
  loadingMore: number | null;
  recent: string[];
  live: { text: string; seq: number };
  toast: { text: string; seq: number } | null;
  toastSeq: number;
  token: TokenState;
  detail: DetailState;
  large: LargeState;
  imageSets: Record<string, MastImageSet>;
  lists: Record<string, ListState>;
  previews: Record<string, PreviewState>;
  products: Record<string, ProductsState>;
  productMode: ProductMode;
  productHidden: boolean;
  focusRequest: { datasetId: string; seq: number } | null;
  banners: Partial<Record<BannerKind, StoredBanner>>;
  thumbs: ThumbQueue;
  thumbBannerHidden: boolean;
  thumbRefs: Record<string, ThumbRef>;
  resultsQuery: MastQuery | null;
  sort: TableSort;
  tableThumbs: boolean;
}

export type ArchiveAction =
  | { type: "missionsLoaded"; missions: MastMission[] }
  | { type: "missionProbeStarted"; mission: MastMissionId; at: number }
  | { type: "missionProbed"; mission: MastMission; at: number }
  | { type: "missionProbeFailed"; mission: MastMissionId; error: MastError; at: number }
  | { type: "formChanged"; patch: Partial<ArchiveForm> }
  | { type: "missionSwitched"; form: ArchiveForm; generation: number }
  | { type: "queryBlocked"; field: "query" | "radius"; message: string }
  | { type: "searchStarted"; generation: number; text: string; kind: QueryKind; name: string | null; query: MastQuery | null; at: number }
  | { type: "nameResolved"; generation: number; resolved: MastResolved; query: MastQuery; at: number }
  | { type: "serverResolve"; generation: number; query: MastQuery; at: number }
  | { type: "nameNotResolved"; generation: number; name: string; elapsedMs: number }
  | { type: "pageLoaded"; generation: number; page: MastSearchPage }
  | { type: "pageRequested"; generation: number; offset: number }
  | { type: "pageFailed"; generation: number; error: MastError; at: number }
  | { type: "searchFailed"; generation: number; error: MastError; at: number }
  | { type: "searchCancelled"; generation: number }
  | { type: "selected"; datasetId: string | null; rowKey?: string | null }
  | { type: "viewToggled" }
  | { type: "viewSet"; view: ResultsView }
  | { type: "sortSet"; sort: TableSort }
  | { type: "tableThumbsToggled" }
  | { type: "detailTab"; tab: DetailTab }
  | { type: "detailToggled" }
  | { type: "drawerSet"; open: boolean }
  | { type: "detailResized"; width: number }
  | { type: "narrowSet"; narrow: boolean }
  | { type: "imageListStarted"; key: string; requestId: string; at: number }
  | { type: "imageListFailed"; key: string; requestId: string; error: MastError; at: number }
  | { type: "imageListCancelled"; key: string; requestId: string }
  | { type: "imageListRetried"; key: string }
  | { type: "imageListLoaded"; key: string; set: MastImageSet }
  | { type: "imageSelected"; imageKey: string }
  | { type: "groupCycled" }
  | { type: "groupSelected"; group: string }
  | { type: "largeOpened"; datasetId: string; imageKey: string | null }
  | { type: "largeImage"; imageKey: string }
  | { type: "largeDataset"; datasetId: string }
  | { type: "largeClosed" }
  | { type: "previewStarted"; key: string; requestId: string; at: number }
  | { type: "previewProgress"; requestId: string; received: number; total: number | null; stage: "download" | "decode" | "ready" }
  | { type: "previewLoaded"; key: string; requestId: string; ref: MastPreviewRef; url: string }
  | { type: "previewFailed"; key: string; requestId: string; error: MastError; at: number }
  | { type: "previewCancelled"; key: string; requestId: string }
  | { type: "previewRetried"; key: string }
  | { type: "productsStarted"; key: string; requestId: string; at: number }
  | { type: "productsLoaded"; key: string; requestId: string; list: MastProductList }
  | { type: "productsFailed"; key: string; requestId: string; error: MastError }
  | { type: "productsCancelled"; key: string; requestId: string; byUser: boolean }
  | { type: "productsRetried"; key: string }
  | { type: "productModeSet"; mode: ProductMode }
  | { type: "productHiddenToggled" }
  | { type: "focusRequested"; datasetId: string }
  | { type: "tokenStatus"; status: MastTokenStatus }
  | { type: "tokenCheckStarted"; step: TokenStep }
  | { type: "tokenStep"; step: TokenStep }
  | { type: "tokenSaved"; status: MastTokenStatus; at: number }
  | { type: "tokenChecked"; status: MastTokenStatus; at: number }
  | { type: "tokenSaveFailed"; error: MastError; status: MastTokenStatus; saved?: boolean }
  | { type: "tokenCheckFailed"; error: MastError }
  | { type: "tokenCleared" }
  | { type: "tokenReplace"; open: boolean }
  | { type: "tokenPanel"; open: boolean }
  | { type: "tokenFocusDone"; seq: number }
  | { type: "bannerRaised"; banner: Banner }
  | { type: "bannerDismissed"; kind: BannerKind }
  | { type: "thumbDemand"; selected: string[]; visible: string[]; ahead: string[] }
  | { type: "thumbsStarted"; keys: string[]; at: number }
  | { type: "thumbReady"; key: string; ref?: ThumbRef }
  | { type: "thumbFailed"; key: string; error: MastError; at: number }
  | { type: "thumbDropped"; key: string }
  | { type: "thumbRateLimited"; key: string; at: number; retryAfterSec: number; consecutive: number; status: number | null }
  | { type: "thumbsTick"; now: number }
  | { type: "thumbsResumed" }
  | { type: "thumbsRetried"; keys?: string[] }
  | { type: "cacheCleared" }
  | { type: "imageMissing"; url: string }
  | { type: "archiveActive"; active: boolean }
  | { type: "toast"; text: string }
  | { type: "toastDismissed" }
  | { type: "announce"; text: string };

const RECENT_LIMIT = 10;
const BANNER_ORDER: readonly BannerKind[] = ["rate-limited", "network", "timeout", "server", "decode", "cache"];
const THUMB_FAILURE_BANNERS: Partial<Record<MastErrorCode, BannerKind>> = { mast_network: "network", mast_timeout: "timeout", mast_server: "server", mast_cache: "cache" };

const LOCAL_ORDER: TableSort = { keys: [], by: "local" };
const CLOSED_LARGE: LargeState = { open: false, datasetId: null, imageKey: null, group: null, filter: null };
const EMPTY_TOKEN: TokenState = {
  status: null,
  checking: false,
  step: null,
  expired: false,
  outcome: null,
  message: null,
  checkedAt: null,
  replacing: false,
  expanded: false,
  focus: null,
  focusSeq: 0,
};

export function createInitialState(form: Partial<ArchiveForm> = {}): ArchiveState {
  return {
    missions: [],
    form: { ...DEFAULT_FORM, filters: { ...DEFAULT_FORM.filters }, ...form },
    generation: 0,
    run: null,
    results: null,
    resolved: null,
    notResolved: null,
    error: null,
    queryError: null,
    unavailable: null,
    probes: {},
    selectedId: null,
    selectedKey: null,
    view: "gallery",
    loadingMore: null,
    recent: [],
    live: { text: "", seq: 0 },
    toast: null,
    toastSeq: 0,
    token: EMPTY_TOKEN,
    detail: { imageKey: null, group: null, tab: "previews", hidden: false, drawerOpen: false, width: DETAIL_WIDTH_DEFAULT, narrow: false },
    large: CLOSED_LARGE,
    imageSets: {},
    lists: {},
    previews: {},
    products: {},
    productMode: "opens",
    productHidden: false,
    focusRequest: null,
    banners: {},
    thumbs: createThumbQueue(),
    thumbBannerHidden: false,
    thumbRefs: {},
    resultsQuery: null,
    sort: LOCAL_ORDER,
    tableThumbs: false,
  };
}

function announce(s: ArchiveState, text: string): ArchiveState {
  return text ? { ...s, live: { text, seq: s.live.seq + 1 } } : s;
}

function missionOf(s: ArchiveState, id = s.form.mission): MastMission | undefined {
  return s.missions.find((m) => m.id === id);
}

function missionLabel(s: ArchiveState): string {
  return missionLabelOf(s, s.form.mission);
}

function tokenView(s: ArchiveState): TokenView {
  return { status: s.token.status, checking: s.token.checking, expired: s.token.expired };
}

export function currentMission(s: ArchiveState): MastMission | undefined {
  return missionOf(s);
}

export function romanLockOf(s: ArchiveState): RomanLock {
  return romanLock(tokenView(s));
}

export function imageSetKey(mission: string, datasetId: string): string {
  return `${mission}:${datasetId}`;
}

function resultMission(s: ArchiveState): string {
  return s.results?.mission ?? s.form.mission;
}

function setFor(s: ArchiveState, datasetId: string | null): MastImageSet | null {
  if (!datasetId) return null;
  return s.imageSets[imageSetKey(resultMission(s), datasetId)] ?? null;
}

function representative(set: MastImageSet): MastImageEntry | null {
  if (set.representative !== null && set.entries[set.representative]) return set.entries[set.representative];
  return set.entries.find((e) => e.group === set.defaultGroup) ?? set.entries[0] ?? null;
}

function firstInGroup(set: MastImageSet, group: string): MastImageEntry | null {
  const rep = representative(set);
  if (rep && rep.group === group) return rep;
  return set.entries.find((e) => e.group === group) ?? null;
}

function filterOf(entry: MastImageEntry): string {
  return entry.filter ?? entry.label;
}

export function detailEntry(s: ArchiveState): MastImageEntry | null {
  const set = setFor(s, s.selectedId);
  if (!set) return null;
  const chosen = s.detail.imageKey ? set.entries.find((e) => e.key === s.detail.imageKey) : undefined;
  if (chosen) return chosen;
  if (s.detail.group) {
    const inGroup = firstInGroup(set, s.detail.group);
    if (inGroup) return inGroup;
  }
  return representative(set);
}

export function largeEntry(s: ArchiveState): MastImageEntry | null {
  if (!s.large.open) return null;
  const set = setFor(s, s.large.datasetId);
  if (!set) return null;
  const chosen = s.large.imageKey ? set.entries.find((e) => e.key === s.large.imageKey) : undefined;
  if (chosen) return chosen;
  const { group, filter } = s.large;
  if (group) {
    const same = filter ? set.entries.find((e) => e.group === group && filterOf(e) === filter) : undefined;
    if (same) return same;
    const inGroup = firstInGroup(set, group);
    if (inGroup) return inGroup;
  }
  return representative(set);
}

export function rowsOnScreen(s: ArchiveState): boolean {
  if (!s.results || s.results.rows.length === 0) return false;
  const rs = resultsState(s);
  return rs === "ready" || rs === "resolving" || rs === "searching";
}

export function selectedRow(s: ArchiveState): MastRow | null {
  if (!s.selectedId || !s.results || !rowsOnScreen(s)) return null;
  const rows = s.results.rows;
  if (s.selectedKey !== null) {
    const keyOf = rowKeyMap(rows);
    const exact = rows.find((r) => r.datasetId === s.selectedId && keyOf.get(r) === s.selectedKey);
    if (exact) return exact;
  }
  return rows.find((r) => r.datasetId === s.selectedId) ?? null;
}

export function detailMode(s: ArchiveState): DetailMode {
  return s.detail.narrow ? "drawer" : "pane";
}

export function detailShown(s: ArchiveState): boolean {
  if (s.large.open || !selectedRow(s)) return false;
  return s.detail.narrow ? s.detail.drawerOpen : !s.detail.hidden;
}

export function detailTabOf(s: ArchiveState): DetailTab {
  if (s.detail.tab !== "previews") return s.detail.tab;
  const m = missionOf(s, s.results?.mission ?? s.form.mission);
  return m && !m.capabilities.images ? "metadata" : "previews";
}

function without<T>(map: Record<string, T>, key: string): Record<string, T> {
  const next = { ...map };
  delete next[key];
  return next;
}

export function effectiveView(s: ArchiveState): ResultsView {
  const m = missionOf(s);
  if (m && !viewAvailability(m).gallery) return "table";
  return s.view;
}

export function resultsState(s: ArchiveState): ResultsState {
  const m = missionOf(s);
  if (m) {
    const gate = resultsGate(m, romanLock(tokenView(s)));
    if (gate === "locked") return "locked";
    if (gate === "unavailable") return "unavailable";
  }
  if (s.unavailable && s.unavailable.mission === s.form.mission) return "unavailable";
  if (s.run) return s.run.phase;
  if (s.error) return "error";
  if (s.notResolved) return "not-resolved";
  if (s.results) return s.results.rows.length ? "ready" : "empty";
  if (m && !m.capabilities.images) return "no-images";
  return "idle";
}

export function unavailableInfo(s: ArchiveState): UnavailableInfo | null {
  if (resultsState(s) !== "unavailable") return null;
  const m = missionOf(s);
  if (!m) return null;
  const probe = s.probes[m.id];
  const fromProbe = probe?.stage === "done" && m.availability === "not_deployed" ? { at: probe.at, status: NOT_DEPLOYED_STATUS, operation: undefined } : null;
  const u = s.unavailable;
  const fromSearch = u && u.mission === m.id ? { at: u.at, status: u.status, operation: u.operation } : null;
  const latest = fromProbe && fromSearch ? (fromProbe.at >= fromSearch.at ? fromProbe : fromSearch) : (fromProbe ?? fromSearch ?? { at: null, status: null, operation: undefined });
  return { mission: m.id, label: m.label, checkedOn: m.checkedOn, ...latest };
}

export function romanPanelState(s: ArchiveState): RomanPanelState {
  const t = s.token;
  if (t.checking) return "checking";
  if (t.outcome) return t.outcome;
  if (t.expired) return "expired";
  if (!t.status || !t.status.present) return "none";
  if (t.status.authorized === true) return "authorized";
  if (t.status.authorized === false) return "not-authorized";
  return "unreachable";
}

function bannerError(error: MastError): BannerKind | null {
  if (error === MISSING_FILE) return null;
  return THUMB_FAILURE_BANNERS[error.code] ?? null;
}

function thumbFailureBanner(s: ArchiveState): Banner | null {
  if (s.thumbBannerHidden) return null;
  const tally = new Map<MastErrorCode, { count: number; error: MastError }>();
  for (const [key, failure] of Object.entries(s.thumbs.failures)) {
    if (s.thumbs.done[key] !== "failed" || !bannerError(failure.error)) continue;
    const seen = tally.get(failure.error.code);
    tally.set(failure.error.code, { count: (seen?.count ?? 0) + 1, error: seen?.error ?? failure.error });
  }
  let top: { count: number; error: MastError } | null = null;
  for (const code of Object.keys(THUMB_FAILURE_BANNERS) as MastErrorCode[]) {
    const entry = tally.get(code);
    if (entry && (!top || entry.count > top.count)) top = entry;
  }
  if (!top) return null;
  const kind = bannerError(top.error) ?? "server";
  if (kind === "cache") return { kind, text: CACHE_TROUBLE_TEXT, retry: "thumbs" };
  const view = slotError(top.error, { surface: "thumb" });
  const said = [view?.meta, view?.detail].filter(Boolean).join(": ");
  const noun = top.count === 1 ? "thumbnail" : "thumbnails";
  return { kind, text: `${top.count} ${noun} failed: ${view?.title ?? top.error.message}${said ? ` (${said})` : ""}.`, retry: "thumbs" };
}

function thumbPauseBanner(s: ArchiveState, now: number): Banner | null {
  const q = s.thumbs;
  if (q.gaveUp) return { kind: "rate-limited", text: THUMBS_GAVE_UP_TEXT, retry: "thumbs" };
  if (q.pausedUntil === null || now >= q.pausedUntil) return null;
  return { kind: "rate-limited", text: rateLimitText("thumbs", (q.pausedUntil - now) / 1000, q.status), retry: "resume" };
}

function storedBanner(b: StoredBanner, now: number): Banner | null {
  if (b.until === null) return { kind: b.kind, text: b.text, retry: b.retry };
  if (now >= b.until) return null;
  return { kind: b.kind, text: rateLimitText("search", (b.until - now) / 1000, b.status), retry: b.retry };
}

export function visibleBanner(s: ArchiveState, now: number): Banner | null {
  const thumbPause = thumbPauseBanner(s, now);
  const thumbFailure = thumbFailureBanner(s);
  for (const kind of BANNER_ORDER) {
    if (kind === "rate-limited" && thumbPause) return thumbPause;
    const stored = s.banners[kind];
    const shown = stored ? storedBanner(stored, now) : null;
    if (shown) return shown;
    if (thumbFailure && thumbFailure.kind === kind) return thumbFailure;
  }
  return null;
}

function raise(s: ArchiveState, banner: Banner, until: number | null = null, status: number | null = null): ArchiveState {
  return { ...s, banners: { ...s.banners, [banner.kind]: { ...banner, until, status } } };
}

function resetForResults(s: ArchiveState): Pick<ArchiveState, "imageSets" | "lists" | "previews" | "products" | "thumbs" | "thumbBannerHidden" | "thumbRefs" | "detail" | "large" | "banners"> {
  const q = s.thumbs;
  return {
    banners: s.banners.cache ? { cache: s.banners.cache } : {},
    imageSets: {},
    lists: {},
    previews: {},
    products: {},
    thumbs: { ...createThumbQueue(), active: q.active, pausedUntil: q.pausedUntil, retryAfterSec: q.retryAfterSec, status: q.status, consecutive: q.consecutive },
    thumbBannerHidden: false,
    thumbRefs: {},
    detail: { ...s.detail, imageKey: null, group: null, drawerOpen: false },
    large: CLOSED_LARGE,
  };
}

function datasetsText(n: number): string {
  return `${n} ${n === 1 ? "dataset" : "datasets"}`;
}

function orderOf(query: MastQuery | null, page: MastSearchPage): TableSort {
  if (!query || query.sort.length === 0) return LOCAL_ORDER;
  const keys: SortKey[] = page.applied.sortBy.map((column, i) => ({ column, desc: page.applied.sortDesc[i] ?? false }));
  return keys.length ? { keys, by: "server" } : LOCAL_ORDER;
}

function notResolvedState(s: ArchiveState, name: string, elapsedMs: number): ArchiveState {
  return announce(
    { ...s, ...resetForResults(s), run: null, results: null, resultsQuery: null, resolved: null, error: null, selectedId: null, selectedKey: null, notResolved: { name, elapsedMs } },
    "Name not found",
  );
}

function onPageLoaded(s: ArchiveState, a: Extract<ArchiveAction, { type: "pageLoaded" }>): ArchiveState {
  if (a.generation !== s.generation) return s;
  const first = a.page.offset === 0;
  if (first) {
    if (!s.run || s.run.generation !== a.generation) return s;
    const unresolved = a.page.unresolvedTargets[0];
    if (a.page.rows.length === 0 && unresolved) return notResolvedState(s, s.run.name ?? unresolved, a.page.elapsedMs);
    const results = mergePage(null, a.page);
    const count = results.totalReliable && results.totalReported !== null ? Math.max(results.totalReported, results.rows.length) : results.rows.length;
    const next: ArchiveState = {
      ...s,
      ...resetForResults(s),
      run: null,
      results,
      resultsQuery: s.run.query,
      sort: orderOf(s.run.query, a.page),
      resolved: s.run.resolved,
      notResolved: null,
      error: null,
      queryError: null,
      loadingMore: null,
      selectedId: results.rows[0]?.datasetId ?? null,
      selectedKey: null,
    };
    return announce(next, results.rows.length ? `Search finished: ${datasetsText(count)}` : "No results");
  }
  if (!s.results) return s;
  const results = mergePage(s.results, a.page);
  return { ...s, results, loadingMore: null };
}

function onSearchFailed(s: ArchiveState, a: Extract<ArchiveAction, { type: "searchFailed" }>): ArchiveState {
  if (!s.run || s.run.generation !== a.generation) return s;
  const base: ArchiveState = { ...s, run: null };
  const hasResults = !!s.results && s.results.rows.length > 0;
  const view = errorView(a.error, { surface: "search", hasResults, missionLabel: missionLabel(s), now: new Date(a.at) });
  switch (view.placement) {
    case "silent":
      return base;
    case "blocking":
      return announce({ ...base, error: a.error, results: null, resultsQuery: null, notResolved: null, selectedId: null, selectedKey: null }, view.title ?? view.text);
    case "inline":
      return announce({ ...base, queryError: { text: view.text, field: view.field ?? "query" } }, view.text);
    case "unavailable": {
      const unavailable: Unavailable = { mission: s.form.mission, text: view.text, at: a.at, status: a.error.context.status ?? null, operation: a.error.context.operation };
      return announce({ ...base, unavailable }, view.text);
    }
    case "roman":
      return onRomanFailure(base, a.error);
    default: {
      const kind = view.bannerKind ?? "server";
      const rateLimited = a.error.code === "mast_rate_limited";
      const until = rateLimited ? a.at + (a.error.context.retryAfterSec ?? 10) * 1000 : null;
      return announce(raise(base, { kind, text: view.text, retry: "search" }, until, a.error.context.status ?? null), view.text);
    }
  }
}

function onPageFailed(s: ArchiveState, a: Extract<ArchiveAction, { type: "pageFailed" }>): ArchiveState {
  if (a.generation !== s.generation) return s;
  const base: ArchiveState = { ...s, loadingMore: null };
  const view = errorView(a.error, { surface: "search", hasResults: true, missionLabel: missionLabel(s), now: new Date(a.at) });
  if (view.placement === "silent") return base;
  if (view.placement === "roman") return onRomanFailure(base, a.error);
  const kind = view.bannerKind ?? "server";
  const until = a.error.code === "mast_rate_limited" ? a.at + (a.error.context.retryAfterSec ?? 10) * 1000 : null;
  return announce(raise(base, { kind, text: view.text, retry: "page" }, until, a.error.context.status ?? null), view.text);
}

function onRomanFailure(s: ArchiveState, error: MastError): ArchiveState {
  if (error.code === "mast_token_rejected") {
    return announce({ ...s, token: { ...s.token, checking: false, step: null, expired: true, outcome: null, message: null } }, "MAST no longer accepts the saved token.");
  }
  const status = s.token.status;
  if (status?.present) {
    return announce({ ...s, token: { ...s.token, status: { present: true, authorized: false }, outcome: null, message: ROMAN_NO_ACCESS_TEXT } }, ROMAN_NO_ACCESS_TEXT);
  }
  return s;
}

function onMissionSwitched(s: ArchiveState, a: Extract<ArchiveAction, { type: "missionSwitched" }>): ArchiveState {
  return {
    ...s,
    ...resetForResults(s),
    token: { ...s.token, replacing: false, expanded: false, focus: null },
    form: a.form,
    generation: a.generation,
    run: null,
    results: null,
    resultsQuery: null,
    sort: LOCAL_ORDER,
    resolved: null,
    notResolved: null,
    error: null,
    queryError: null,
    selectedId: null,
    selectedKey: null,
    loadingMore: null,
  };
}

function cycleGroup(set: MastImageSet, current: string | null): string | null {
  const groups = set.groups.filter((g) => g.count > 0).map((g) => g.key);
  if (groups.length === 0) return null;
  const at = current ? groups.indexOf(current) : -1;
  return groups[(at + 1) % groups.length];
}

function onGroupCycled(s: ArchiveState): ArchiveState {
  if (s.large.open) {
    const set = setFor(s, s.large.datasetId);
    const entry = largeEntry(s);
    const group = set ? cycleGroup(set, entry?.group ?? null) : null;
    if (!set || !group) return s;
    const next = firstInGroup(set, group);
    return { ...s, large: { ...s.large, imageKey: next?.key ?? null, group, filter: next ? filterOf(next) : s.large.filter } };
  }
  const set = setFor(s, s.selectedId);
  const group = set ? cycleGroup(set, detailEntry(s)?.group ?? null) : null;
  if (!group) return s;
  return { ...s, detail: { ...s.detail, group, imageKey: null } };
}

function rememberLarge(large: LargeState, entry: MastImageEntry | null): LargeState {
  return entry ? { ...large, group: entry.group, filter: filterOf(entry) } : large;
}

function onCacheCleared(s: ArchiveState): ArchiveState {
  const previews = Object.fromEntries(Object.entries(s.previews).filter(([, p]) => p.stage === "download" || p.stage === "decode"));
  const banners = { ...s.banners };
  delete banners.cache;
  return { ...s, banners, imageSets: {}, previews, thumbRefs: {}, thumbs: forget(s.thumbs, Object.keys(s.thumbs.done)), thumbBannerHidden: false };
}

function onImageMissing(s: ArchiveState, url: string): ArchiveState {
  const thumbKeys = Object.keys(s.thumbRefs).filter((k) => s.thumbRefs[k].url === url);
  const previewKeys = Object.keys(s.previews).filter((k) => s.previews[k].stage === "ready" && s.previews[k].url === url);
  if (thumbKeys.length === 0 && previewKeys.length === 0) return s;
  let thumbs = s.thumbs;
  const thumbRefs = { ...s.thumbRefs };
  for (const key of thumbKeys) {
    thumbs = markFailed(thumbs, key, MISSING_FILE, null);
    delete thumbRefs[key];
  }
  const previews = { ...s.previews };
  for (const key of previewKeys) previews[key] = { ...previews[key], stage: "failed", error: MISSING_FILE };
  return { ...s, thumbs, thumbRefs, previews };
}

function onPreviewProgress(s: ArchiveState, a: Extract<ArchiveAction, { type: "previewProgress" }>): ArchiveState {
  const key = Object.keys(s.previews).find((k) => s.previews[k].requestId === a.requestId);
  if (!key) return s;
  const prev = s.previews[key];
  if (prev.stage === "ready" || prev.stage === "failed") return s;
  const stage: PreviewStage = a.stage === "ready" ? prev.stage : a.stage;
  return { ...s, previews: { ...s.previews, [key]: { ...prev, received: a.received, total: a.total, stage } } };
}

function withPreview(s: ArchiveState, key: string, requestId: string, patch: Partial<PreviewState>): ArchiveState {
  const prev = s.previews[key];
  if (!prev || prev.requestId !== requestId) return s;
  return { ...s, previews: { ...s.previews, [key]: { ...prev, ...patch } } };
}

function withProducts(s: ArchiveState, key: string, requestId: string, patch: Partial<ProductsState>): ArchiveState {
  const prev = s.products[key];
  if (!prev || prev.requestId !== requestId) return s;
  return { ...s, products: { ...s.products, [key]: { ...prev, ...patch } } };
}

function focusOn(token: TokenState, target: TokenFocusTarget): TokenState {
  const seq = token.focusSeq + 1;
  return { ...token, focus: { target, seq }, focusSeq: seq };
}

function missionLabelOf(s: ArchiveState, id: MastMissionId): string {
  return missionOf(s, id)?.label ?? staticMissionLabel(id);
}

function probeAnnouncement(m: MastMission, at: number): string {
  if (m.availability === "not_deployed") return `MAST does not serve the ${m.label} Mission Search API right now (checked ${clockText(at)}).`;
  if (m.availability === "token") return `${m.label} needs a MAST token.`;
  if (m.availability === "open" && m.capabilities.search) return `${m.label} answers at MAST now.`;
  return "";
}

function onMissionProbed(s: ArchiveState, a: Extract<ArchiveAction, { type: "missionProbed" }>): ArchiveState {
  const missions = s.missions.map((m) => (m.id === a.mission.id ? a.mission : m));
  const cleared = s.unavailable?.mission === a.mission.id && a.mission.availability !== "not_deployed";
  const probes: MissionProbes = { ...s.probes, [a.mission.id]: { stage: "done", at: a.at, error: null } };
  return announce({ ...s, missions, probes, unavailable: cleared ? null : s.unavailable }, probeAnnouncement(a.mission, a.at));
}

function onMissionProbeFailed(s: ArchiveState, a: Extract<ArchiveAction, { type: "missionProbeFailed" }>): ArchiveState {
  const probes: MissionProbes = { ...s.probes, [a.mission]: { stage: "failed", at: a.at, error: a.error } };
  if (a.error.code === "mast_cancelled") return { ...s, probes };
  const label = missionLabelOf(s, a.mission);
  const text = errorView(a.error, { surface: "search", hasResults: true, missionLabel: label, now: new Date(a.at) }).text;
  return announce({ ...s, probes }, `Could not check ${label} again: ${text}`);
}

function tokenMessage(status: MastTokenStatus): string | null {
  if (status.authorized === true) return ROMAN_CONFIRMED_TEXT;
  if (status.authorized === false) return ROMAN_NO_ACCESS_TEXT;
  return null;
}

function onTokenSettled(s: ArchiveState, a: Extract<ArchiveAction, { type: "tokenSaved" | "tokenChecked" }>): ArchiveState {
  const message = tokenMessage(a.status);
  const settled: TokenState = { ...s.token, status: a.status, checking: false, step: null, expired: false, outcome: null, message, checkedAt: a.at, replacing: false };
  const token = focusOn(settled, "outcome");
  return message ? announce({ ...s, token }, message) : { ...s, token };
}

function onTokenSaveFailed(s: ArchiveState, a: Extract<ArchiveAction, { type: "tokenSaveFailed" }>): ArchiveState {
  const token: TokenState = { ...s.token, checking: false, step: null, status: a.status, expired: false, replacing: false };
  if (a.error.code === "mast_cancelled") return { ...s, token: focusOn({ ...token, outcome: null, message: null }, a.status.present ? "outcome" : "field") };
  const saved = a.saved ?? a.status.present;
  const view = errorView(a.error, { surface: "token-save", tokenSaved: saved });
  const outcome = a.error.code === "mast_token_rejected" || a.error.code === "mast_bad_input" ? "rejected" : "unreachable";
  const message = outcome === "unreachable" ? tokenSaveFailureText(saved) : view.text;
  return announce({ ...s, token: focusOn({ ...token, outcome, message }, outcome === "rejected" ? "field" : "outcome") }, message);
}

function onTokenCheckFailed(s: ArchiveState, a: Extract<ArchiveAction, { type: "tokenCheckFailed" }>): ArchiveState {
  const token: TokenState = focusOn({ ...s.token, checking: false, step: null }, "outcome");
  if (a.error.code === "mast_cancelled") return { ...s, token };
  if (a.error.code === "mast_token_rejected" || a.error.code === "mast_unauthorized") return onRomanFailure({ ...s, token }, a.error);
  return announce({ ...s, token: { ...token, outcome: "unreachable", message: TOKEN_CHECK_UNREACHABLE_TEXT } }, TOKEN_CHECK_UNREACHABLE_TEXT);
}

export function archiveReducer(s: ArchiveState, a: ArchiveAction): ArchiveState {
  switch (a.type) {
    case "missionsLoaded":
      return { ...s, missions: a.missions };
    case "missionProbeStarted":
      return { ...s, probes: { ...s.probes, [a.mission]: { stage: "checking", at: a.at, error: null } } };
    case "missionProbed":
      return onMissionProbed(s, a);
    case "missionProbeFailed":
      return onMissionProbeFailed(s, a);
    case "formChanged":
      return { ...s, form: { ...s.form, ...a.patch }, queryError: null };
    case "missionSwitched":
      return onMissionSwitched(s, a);
    case "queryBlocked":
      return announce({ ...s, queryError: { text: a.message, field: a.field } }, a.message);
    case "searchStarted": {
      const recent = [a.text, ...s.recent.filter((t) => t !== a.text)].slice(0, RECENT_LIMIT);
      const phase = a.kind === "name" && a.query === null ? "resolving" : "searching";
      const run: SearchRun = { generation: a.generation, text: a.text, kind: a.kind, name: a.name, query: a.query, phase, resolved: null, startedAt: a.at, searchStartedAt: a.at };
      const unavailable = s.unavailable?.mission === s.form.mission ? null : s.unavailable;
      return { ...s, generation: a.generation, run, recent, queryError: null, loadingMore: null, unavailable };
    }
    case "nameResolved":
      if (!s.run || s.run.generation !== a.generation) return s;
      return { ...s, run: { ...s.run, phase: "searching", resolved: a.resolved, query: a.query, searchStartedAt: a.at } };
    case "serverResolve":
      if (!s.run || s.run.generation !== a.generation) return s;
      return { ...s, run: { ...s.run, phase: "searching", resolved: null, query: a.query, searchStartedAt: a.at } };
    case "nameNotResolved":
      if (!s.run || s.run.generation !== a.generation) return s;
      return notResolvedState(s, a.name, a.elapsedMs);
    case "pageLoaded":
      return onPageLoaded(s, a);
    case "pageRequested":
      if (a.generation !== s.generation || !s.results) return s;
      return { ...s, loadingMore: a.offset };
    case "pageFailed":
      return onPageFailed(s, a);
    case "searchFailed":
      return onSearchFailed(s, a);
    case "searchCancelled":
      if (!s.run || s.run.generation !== a.generation) return s;
      return { ...s, run: null };
    case "selected": {
      const selectedKey = a.rowKey ?? null;
      if (a.datasetId === s.selectedId) return selectedKey === s.selectedKey ? s : { ...s, selectedKey };
      return { ...s, selectedId: a.datasetId, selectedKey, detail: { ...s.detail, imageKey: null, group: null } };
    }
    case "viewToggled":
      return { ...s, view: effectiveView(s) === "gallery" ? "table" : "gallery" };
    case "viewSet":
      return { ...s, view: a.view };
    case "sortSet":
      return { ...s, sort: a.sort };
    case "tableThumbsToggled":
      return { ...s, tableThumbs: !s.tableThumbs };
    case "detailTab":
      return { ...s, detail: { ...s.detail, tab: a.tab } };
    case "detailToggled":
      return { ...s, detail: { ...s.detail, hidden: !s.detail.hidden } };
    case "drawerSet":
      return { ...s, detail: { ...s.detail, drawerOpen: a.open } };
    case "detailResized": {
      const width = clampDetailWidth(a.width);
      return width === s.detail.width ? s : { ...s, detail: { ...s.detail, width } };
    }
    case "narrowSet":
      return s.detail.narrow === a.narrow ? s : { ...s, detail: { ...s.detail, narrow: a.narrow, drawerOpen: false } };
    case "imageListStarted":
      return { ...s, lists: { ...s.lists, [a.key]: { requestId: a.requestId, startedAt: a.at, endedAt: null, error: null } } };
    case "imageListFailed": {
      const list = s.lists[a.key];
      if (!list || list.requestId !== a.requestId) return s;
      return { ...s, lists: { ...s.lists, [a.key]: { ...list, error: a.error, endedAt: a.at } } };
    }
    case "imageListCancelled": {
      const list = s.lists[a.key];
      if (!list || list.requestId !== a.requestId) return s;
      return { ...s, lists: without(s.lists, a.key) };
    }
    case "imageListRetried":
      return s.lists[a.key]?.error ? { ...s, lists: without(s.lists, a.key) } : s;
    case "imageListLoaded":
      return { ...s, imageSets: { ...s.imageSets, [a.key]: a.set }, lists: a.key in s.lists ? without(s.lists, a.key) : s.lists };
    case "imageSelected":
      return { ...s, detail: { ...s.detail, imageKey: a.imageKey } };
    case "groupCycled":
      return onGroupCycled(s);
    case "groupSelected":
      return { ...s, detail: { ...s.detail, group: a.group, imageKey: null } };
    case "largeOpened": {
      const large: LargeState = { open: true, datasetId: a.datasetId, imageKey: a.imageKey, group: null, filter: null };
      const same = a.datasetId === s.selectedId;
      const detail = same ? s.detail : { ...s.detail, imageKey: null, group: null };
      const next: ArchiveState = { ...s, selectedId: a.datasetId, selectedKey: same ? s.selectedKey : null, detail, large };
      return { ...next, large: rememberLarge(large, largeEntry(next)) };
    }
    case "largeImage": {
      if (!s.large.open) return s;
      const next: ArchiveState = { ...s, large: { ...s.large, imageKey: a.imageKey } };
      return { ...next, large: rememberLarge(next.large, largeEntry(next)) };
    }
    case "largeDataset": {
      if (!s.large.open) return s;
      const remembered = rememberLarge(s.large, largeEntry(s));
      return {
        ...s,
        selectedId: a.datasetId,
        selectedKey: null,
        detail: { ...s.detail, imageKey: null, group: null },
        large: { ...remembered, datasetId: a.datasetId, imageKey: null },
      };
    }
    case "largeClosed": {
      if (!s.large.open) return s;
      const entry = largeEntry(s);
      const detail = entry && s.large.datasetId === s.selectedId ? { ...s.detail, imageKey: entry.key, group: entry.group } : s.detail;
      return { ...s, detail, large: CLOSED_LARGE };
    }
    case "previewStarted":
      return { ...s, previews: { ...s.previews, [a.key]: { requestId: a.requestId, stage: "download", received: 0, total: null, startedAt: a.at, endedAt: null, ref: null, url: null, error: null } } };
    case "previewProgress":
      return onPreviewProgress(s, a);
    case "previewLoaded":
      return withPreview(s, a.key, a.requestId, { stage: "ready", ref: a.ref, url: a.url, error: null });
    case "previewFailed":
      return withPreview(s, a.key, a.requestId, { stage: "failed", error: a.error, endedAt: a.at });
    case "previewCancelled": {
      const prev = s.previews[a.key];
      if (!prev || prev.requestId !== a.requestId) return s;
      return { ...s, previews: without(s.previews, a.key) };
    }
    case "previewRetried":
      return s.previews[a.key]?.stage === "failed" ? { ...s, previews: without(s.previews, a.key) } : s;
    case "productsStarted":
      return { ...s, products: { ...s.products, [a.key]: { requestId: a.requestId, stage: "loading", startedAt: a.at, list: null, error: null } } };
    case "productsLoaded":
      return withProducts(s, a.key, a.requestId, { stage: "ready", list: a.list, error: null });
    case "productsFailed":
      return withProducts(s, a.key, a.requestId, { stage: "failed", error: a.error });
    case "productsCancelled": {
      const prev = s.products[a.key];
      if (!prev || prev.requestId !== a.requestId || prev.stage !== "loading") return s;
      return a.byUser ? withProducts(s, a.key, a.requestId, { stage: "cancelled" }) : { ...s, products: without(s.products, a.key) };
    }
    case "productsRetried": {
      const stage = s.products[a.key]?.stage;
      return stage === "failed" || stage === "cancelled" ? { ...s, products: without(s.products, a.key) } : s;
    }
    case "productModeSet":
      return s.productMode === a.mode ? s : { ...s, productMode: a.mode };
    case "productHiddenToggled":
      return { ...s, productHidden: !s.productHidden };
    case "focusRequested":
      return { ...s, focusRequest: { datasetId: a.datasetId, seq: (s.focusRequest?.seq ?? 0) + 1 } };
    case "tokenStatus": {
      const keepExpired = s.token.expired && a.status.present && a.status.authorized !== true;
      return { ...s, token: { ...s.token, status: a.status, expired: keepExpired, outcome: null, message: keepExpired ? s.token.message : tokenMessage(a.status) } };
    }
    case "tokenCheckStarted":
      return { ...s, token: { ...s.token, checking: true, step: a.step, outcome: null, message: null } };
    case "tokenStep":
      return s.token.checking ? { ...s, token: { ...s.token, step: a.step } } : s;
    case "tokenSaved":
    case "tokenChecked":
      return onTokenSettled(s, a);
    case "tokenSaveFailed":
      return onTokenSaveFailed(s, a);
    case "tokenCheckFailed":
      return onTokenCheckFailed(s, a);
    case "tokenCleared": {
      const cleared: TokenState = { ...EMPTY_TOKEN, status: { present: false, authorized: null }, focusSeq: s.token.focusSeq };
      return announce({ ...s, token: focusOn(cleared, "field") }, TOKEN_REMOVED_TEXT);
    }
    case "tokenReplace":
      return { ...s, token: focusOn({ ...s.token, replacing: a.open }, a.open ? "field" : "panel") };
    case "tokenPanel":
      return { ...s, token: focusOn({ ...s.token, expanded: a.open, replacing: a.open ? s.token.replacing : false }, a.open ? "panel" : "outcome") };
    case "tokenFocusDone":
      return s.token.focus?.seq === a.seq ? { ...s, token: { ...s.token, focus: null } } : s;
    case "bannerRaised":
      return raise(s, a.banner);
    case "bannerDismissed": {
      const banners = { ...s.banners };
      delete banners[a.kind];
      const thumbFailure = thumbFailureBanner(s);
      return { ...s, banners, thumbBannerHidden: s.thumbBannerHidden || thumbFailure?.kind === a.kind };
    }
    case "thumbDemand":
      return { ...s, thumbs: setDemand(s.thumbs, a) };
    case "thumbsStarted":
      return { ...s, thumbs: markStarted(s.thumbs, a.keys, a.at) };
    case "thumbReady":
      return { ...s, thumbs: markReady(s.thumbs, a.key), thumbRefs: a.ref ? { ...s.thumbRefs, [a.key]: a.ref } : s.thumbRefs };
    case "thumbFailed":
      return { ...s, thumbs: markFailed(s.thumbs, a.key, a.error, a.at), thumbBannerHidden: bannerError(a.error) ? false : s.thumbBannerHidden };
    case "thumbDropped":
      return { ...s, thumbs: markDropped(s.thumbs, a.key) };
    case "thumbRateLimited": {
      const thumbs = markRateLimited(s.thumbs, a.key, a.at, a.retryAfterSec, a.consecutive, a.status);
      const text = thumbs.gaveUp ? THUMBS_GAVE_UP_TEXT : `Thumbnails paused for ${thumbs.retryAfterSec ?? a.retryAfterSec} seconds`;
      return announce({ ...s, thumbs }, text);
    }
    case "thumbsTick": {
      const thumbs = tickQueue(s.thumbs, a.now);
      return thumbs === s.thumbs ? s : { ...s, thumbs };
    }
    case "thumbsResumed":
      return { ...s, thumbs: resumeNow(s.thumbs) };
    case "thumbsRetried":
      return { ...s, thumbs: retryFailed(s.thumbs, a.keys), thumbBannerHidden: false };
    case "cacheCleared":
      return onCacheCleared(s);
    case "imageMissing":
      return onImageMissing(s, a.url);
    case "archiveActive":
      return { ...s, thumbs: setActive(s.thumbs, a.active) };
    case "toast": {
      const seq = s.toastSeq + 1;
      return { ...s, toastSeq: seq, toast: { text: a.text, seq } };
    }
    case "toastDismissed":
      return s.toast ? { ...s, toast: null } : s;
    case "announce":
      return announce(s, a.text);
  }
}
