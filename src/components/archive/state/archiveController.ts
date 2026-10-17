import type { WcsInfo } from "../../../shared/types/astrometry";
import type {
  MastCacheInfo,
  MastColumn,
  MastError,
  MastErrorCode,
  MastFilterValues,
  MastImageRef,
  MastImageSet,
  MastLevelClass,
  MastMission,
  MastMissionId,
  MastPreset,
  MastPreviewRef,
  MastProductList,
  MastProgressEvent,
  MastQuery,
  MastResolved,
  MastSearchPage,
  MastTokenStatus,
} from "../../../shared/types/mast";
import {
  mastCacheClear,
  mastCacheInfo,
  mastCancel,
  mastErrorOf,
  mastImageList,
  mastMissions,
  mastPresets,
  mastPreview,
  mastProbeMission,
  mastProducts,
  mastResolve,
  mastSearch,
  mastThumbnail,
  mastTokenClear,
  mastTokenSave,
  mastTokenStatus,
  onMastProgress,
  type MastCacheKind,
} from "../../../services/mast";
import { getWcsInfo } from "../../../services/astrometry";
import { getPreviewUrl } from "../../../infrastructure/tauri";
import {
  DEFAULT_RADIUS_ARCMIN,
  browserLevelStorage,
  loadLevel,
  planSearch,
  saveLevel,
  switchMission as switchMissionForm,
  type ArchiveForm,
  type LevelStorage,
  type QuickFilterId,
  type SortKey,
} from "../logic/buildQuery";
import { interpretQuery, type QueryKind } from "../logic/interpretQuery";
import { nextOffset, nextSort, sortMode, SORT_BLOCKED_HINT } from "../logic/paging";
import { failedCount, nextStarts } from "../logic/thumbQueue";
import { generationOf, generationPrefix, nonThumbPrefixes, pageId, resolveId, searchId, thumbId, TOKEN_CHECK_ID, TOKEN_SAVE_ID } from "../logic/requestIds";
import { errorView, slotError, type BannerKind } from "../logic/errorCopy";
import { missionEntries, resultsGate, romanLock } from "../logic/missionState";
import { aroundQueryText, type AroundTarget } from "../logic/aroundQuery";
import { formatBytes, type ProductMode } from "../logic/productView";
import { displayRows, type ChipId } from "../logic/resultsView";
import type { Suggestion } from "../logic/suggestions";
import { parseStripKey, previewsAllowed, stepEntry, STRIP_PRIORITY } from "../logic/detailView";
import { MAST_TOKEN_PAGE_URL, checkTokenFormat } from "../logic/tokenFormat";
import { OPEN_PAGE_FAILED_TEXT, tokenWasSaved } from "../logic/romanPanel";
import { detailEntry, imageSetKey, largeEntry, resultsState, visibleBanner, type ArchiveState, type DetailTab, type ResultsView } from "./archiveReducer";
import { archiveStore, type ArchiveStore } from "./archiveStore";
import { archiveInflight, type InflightCounter, type InflightKind } from "./inflight";
import { createDetailEffects } from "./detailEffects";

export interface ArchiveServices {
  missions(): Promise<MastMission[]>;
  presets(): Promise<MastPreset[]>;
  probeMission(mission: MastMissionId): Promise<MastMission>;
  resolve(requestId: string, name: string): Promise<MastResolved | null>;
  search(requestId: string, query: MastQuery, refresh: boolean): Promise<MastSearchPage>;
  imageList(requestId: string, mission: MastMissionId, datasetId: string): Promise<MastImageSet>;
  thumbnail(requestId: string, mission: MastMissionId, datasetId: string, levelClass: MastLevelClass, filename: string | null): Promise<MastImageRef>;
  preview(requestId: string, mission: MastMissionId, datasetId: string, filename: string, size: "pane" | "large"): Promise<MastPreviewRef>;
  products(requestId: string, mission: MastMissionId, datasetIds: string[], refresh: boolean): Promise<MastProductList>;
  cancel(target: { requestIds?: string[]; prefix?: string }): Promise<number>;
  tokenStatus(requestId: string, check: boolean): Promise<MastTokenStatus>;
  tokenSave(requestId: string, token: string): Promise<MastTokenStatus>;
  tokenClear(): Promise<MastTokenStatus>;
  cacheInfo(): Promise<MastCacheInfo>;
  cacheClear(kind: MastCacheKind): Promise<MastCacheInfo>;
  onProgress(cb: (event: MastProgressEvent) => void): Promise<() => void>;
  previewUrl(path: string): Promise<string>;
  wcsInfo(path: string): Promise<WcsInfo>;
  copyText(text: string): Promise<void>;
  openUrl(url: string): Promise<void>;
}

export interface ArchiveClock {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface ArchiveControllerDeps {
  store: ArchiveStore;
  services: ArchiveServices;
  clock: ArchiveClock;
  storage: LevelStorage | null;
  inflight: InflightCounter;
}

export interface ThumbDemand {
  selected: string[];
  visible: string[];
  ahead: string[];
  firstRow?: number;
}

export interface ArchiveController {
  init(): Promise<void>;
  setActive(active: boolean): void;
  setText(text: string): void;
  setKindOverride(kind: QueryKind | null): void;
  setRadius(text: string): void;
  setFilters(patch: Partial<MastFilterValues>, opts?: { rerun?: boolean }): void;
  setPreset(id: string | null, opts?: { rerun?: boolean }): void;
  removeFilter(id: ChipId): Promise<void>;
  switchMission(id: MastMissionId): void;
  tryMission(id: MastMissionId): Promise<void>;
  checkMission(id: MastMissionId): Promise<void>;
  openRomanAccess(): void;
  saveToken(raw: string): Promise<void>;
  checkToken(): Promise<void>;
  cancelTokenCheck(): void;
  clearToken(): Promise<void>;
  openTokenPage(): Promise<void>;
  replaceToken(open: boolean): void;
  setRomanPanel(open: boolean): void;
  tokenFocusDone(seq: number): void;
  search(opts?: { refresh?: boolean; sort?: SortKey[] }): Promise<void>;
  cancelSearch(): void;
  loadMore(opts?: { auto?: boolean }): Promise<void>;
  retry(): void;
  runSuggestion(suggestion: Suggestion): Promise<void>;
  searchAround(target: AroundTarget): Promise<void>;
  widen(radiusArcmin: number): Promise<void>;
  select(datasetId: string | null, rowKey?: string | null): void;
  toggleView(): void;
  setView(view: ResultsView): void;
  toggleTableThumbs(): void;
  sortBy(column: MastColumn, additive: boolean): void;
  setThumbDemand(demand: ThumbDemand): void;
  retryThumbs(keys?: string[]): void;
  resumeThumbs(): void;
  dismissBanner(kind: BannerKind): void;
  copy(text: string, label?: string): Promise<void>;
  dismissToast(): void;
  clearCache(): Promise<void>;
  imageMissing(url: string): void;
  setNarrow(narrow: boolean): void;
  setDetailTab(tab: DetailTab): void;
  toggleDetail(): void;
  setDetailWidth(width: number): void;
  openDrawer(): void;
  closeDrawer(): void;
  selectImage(imageKey: string): void;
  selectGroup(group: string): void;
  cycleGroup(): void;
  stepImage(dir: 1 | -1): void;
  openLarge(datasetId: string, imageKey?: string | null): void;
  closeLarge(returnFocus?: boolean): void;
  largeStepImage(dir: 1 | -1): void;
  largeStepDataset(dir: 1 | -1): void;
  largeSelectImage(imageKey: string): void;
  cancelPreview(size: "pane" | "large"): void;
  retryDetail(): void;
  retryLarge(): void;
  refreshProducts(): void;
  cancelProducts(): void;
  setProductMode(mode: ProductMode): void;
  toggleHiddenProducts(): void;
  dispose(): void;
}

const LOOKUP_FALLBACK = new Set(["mast_network", "mast_server", "mast_timeout", "mast_decode"]);
const DEFAULT_RETRY_AFTER_SEC = 10;
const FAILURE_BURST_MS = 1500;
const QUIET_FAILURES = new Set(["mast_no_image", "mast_unauthorized", "mast_cancelled"]);

const CHIP_PATCH: Record<ChipId, Partial<MastFilterValues>> = {
  level: { level: "all" },
  instrument: { instrument: undefined },
  filter: { filter: undefined },
  dateFrom: { dateFrom: undefined },
  dateTo: { dateTo: undefined },
  publicOnly: { publicOnly: undefined },
};

function cleanFilters(filters: MastFilterValues): MastFilterValues {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(filters)) {
    if (v === undefined || v === null || v === "" || v === false) continue;
    out[k] = v;
  }
  return out as MastFilterValues;
}

export function createArchiveController(deps: ArchiveControllerDeps): ArchiveController {
  const { store, services, clock, storage, inflight } = deps;
  const get = () => store.getState();
  const dispatch = store.dispatch;
  const thumbRequests = new Map<string, string>();
  const supersededThumbs = new Map<string, string>();
  let initRun: Promise<void> | null = null;
  let pauseTimer: { at: number; handle: unknown } | null = null;
  let pumping = false;
  let cardDemand: ThumbDemand = { selected: [], visible: [], ahead: [] };
  let stripDemand: string[] = [];
  let progressAsked = false;
  let progressOff: (() => void) | null = null;
  let tokenRequest: string | null = null;
  let autoChecked = false;
  let failedPage: { generation: number; offset: number } | null = null;
  let failureBurst: { count: number; byCode: Map<MastErrorCode, { count: number; error: MastError }>; handle: unknown } | null = null;

  function call<T>(kind: InflightKind, fn: () => Promise<T>): Promise<T> {
    const end = inflight.begin(kind);
    let p: Promise<T>;
    try {
      p = fn();
    } catch (err) {
      p = Promise.reject(err);
    }
    return p.finally(end);
  }

  function cancelGeneration(generation: number): void {
    services.cancel({ prefix: generationPrefix(generation) }).catch(() => 0);
  }

  function cancelAllButThumbs(generation: number): void {
    for (const prefix of nonThumbPrefixes(generation)) services.cancel({ prefix }).catch(() => 0);
  }

  function cancelThumbs(requestIds: string[]): void {
    if (requestIds.length) services.cancel({ requestIds }).catch(() => 0);
  }

  function cancelEveryThumb(): void {
    const ids = [...thumbRequests.values(), ...supersededThumbs.keys()];
    thumbRequests.clear();
    supersededThumbs.clear();
    cancelThumbs(ids);
  }

  function pruneThumbs(s: ArchiveState): void {
    if (s.run || (thumbRequests.size === 0 && supersededThumbs.size === 0)) return;
    let onScreen: Set<string> | null = null;
    const shown = (key: string) => {
      onScreen ??= new Set(s.results?.rows.map((r) => r.datasetId) ?? []);
      return onScreen.has(parseStripKey(key)?.datasetId ?? key);
    };
    for (const [key, requestId] of thumbRequests) {
      if (generationOf(requestId) === s.generation || shown(key)) continue;
      thumbRequests.delete(key);
      cancelThumbs([requestId]);
      if (s.thumbs.inflight.includes(key)) dispatch({ type: "thumbDropped", key });
    }
    for (const [requestId, key] of supersededThumbs) {
      if (thumbRequests.has(key) && shown(key)) continue;
      supersededThumbs.delete(requestId);
      cancelThumbs([requestId]);
    }
  }

  function missionOf(s: ArchiveState, id: MastMissionId = s.form.mission): MastMission | undefined {
    return s.missions.find((m) => m.id === id);
  }

  function lockOf(s: ArchiveState) {
    return romanLock({ status: s.token.status, checking: s.token.checking, expired: s.token.expired });
  }

  function gateOpen(s: ArchiveState): boolean {
    const m = missionOf(s);
    return !!m && resultsGate(m, lockOf(s)) === "open";
  }

  function hasSearched(s: ArchiveState): boolean {
    return !!(s.results || s.run || s.error || s.notResolved);
  }

  function enabled(s: ArchiveState, id: MastMissionId): boolean {
    return !!missionEntries(s.missions, lockOf(s)).find((e) => e.id === id)?.enabled;
  }

  function isCurrent(generation: number): boolean {
    const run = get().run;
    return !!run && run.generation === generation;
  }

  function thumbAllowed(s: ArchiveState): (key: string) => boolean {
    const results = s.results;
    const mission = results ? missionOf(s, results.mission) : undefined;
    if (!results || !mission) return () => true;
    return (key) => {
      const datasetId = parseStripKey(key)?.datasetId ?? key;
      const row = results.rows.find((r) => r.datasetId === datasetId);
      return !row || previewsAllowed(row, mission);
    };
  }

  function pushDemand(): void {
    const allowed = thumbAllowed(get());
    const cards = new Set([...cardDemand.selected, ...cardDemand.visible]);
    const strip = stripDemand.filter((k) => !cards.has(k));
    const firstRow = cardDemand.visible.slice(0, cardDemand.firstRow ?? cardDemand.visible.length);
    const otherCards = cardDemand.visible.slice(firstRow.length);
    const visible = [...firstRow, ...strip.slice(0, STRIP_PRIORITY), ...otherCards, ...strip.slice(STRIP_PRIORITY)];
    dispatch({ type: "thumbDemand", selected: cardDemand.selected.filter(allowed), visible: visible.filter(allowed), ahead: cardDemand.ahead.filter(allowed) });
  }

  const detail = createDetailEffects({
    store,
    services,
    clock,
    call,
    onStripDemand(keys) {
      stripDemand = keys;
      pushDemand();
    },
  });

  function listenProgress(): void {
    if (progressAsked) return;
    progressAsked = true;
    services
      .onProgress((event) => detail.onProgress(event))
      .then((off) => {
        progressOff = off;
      })
      .catch(() => {
        progressAsked = false;
      });
  }

  function init(): Promise<void> {
    listenProgress();
    if (initRun) return initRun;
    initRun = (async () => {
      const form = get().form;
      const level = loadLevel(storage);
      if (form.mission === "jwst" && form.filters.level !== level) dispatch({ type: "formChanged", patch: { filters: { ...form.filters, level } } });
      const [missions, token, presets] = await Promise.allSettled([services.missions(), services.tokenStatus(TOKEN_CHECK_ID, false), services.presets()]);
      if (presets.status === "fulfilled") dispatch({ type: "presetsLoaded", presets: presets.value });
      if (missions.status === "fulfilled") {
        dispatch({ type: "missionsLoaded", missions: missions.value });
      } else {
        initRun = null;
        const view = errorView(mastErrorOf(missions.reason), { surface: "search", hasResults: true });
        dispatch({ type: "bannerRaised", banner: { kind: view.bannerKind ?? "decode", text: view.text, retry: null } });
      }
      if (token.status === "fulfilled") dispatch({ type: "tokenStatus", status: token.value });
      maybeAutoCheck();
    })();
    return initRun;
  }

  function afterAuthorized(status: MastTokenStatus): void {
    const s = get();
    if (status.authorized === true && s.form.mission === "roman" && s.form.text.trim() && !s.run) void search();
  }

  async function storedStatus(fallback: MastTokenStatus | null): Promise<MastTokenStatus> {
    try {
      return await services.tokenStatus(TOKEN_CHECK_ID, false);
    } catch {
      return fallback ?? { present: false, authorized: null };
    }
  }

  async function saveToken(raw: string): Promise<void> {
    const s = get();
    if (s.token.checking) return;
    const before = s.token.status;
    const format = checkTokenFormat(raw);
    if (!format.ok) {
      dispatch({ type: "tokenSaveFailed", error: { code: "mast_bad_input", message: format.reason, context: {} }, status: before ?? { present: false, authorized: null } });
      return;
    }
    dispatch({ type: "tokenCheckStarted", step: "info" });
    tokenRequest = TOKEN_SAVE_ID;
    try {
      const status = await call("other", () => services.tokenSave(TOKEN_SAVE_ID, format.token));
      tokenRequest = null;
      dispatch({ type: "tokenSaved", status, at: clock.now() });
      afterAuthorized(status);
    } catch (err) {
      const error = mastErrorOf(err);
      const status = await storedStatus(before);
      tokenRequest = null;
      dispatch({ type: "tokenSaveFailed", error, status, saved: tokenWasSaved(error, before, status) });
    }
  }

  async function checkToken(): Promise<void> {
    if (get().token.checking) return;
    dispatch({ type: "tokenCheckStarted", step: "info" });
    tokenRequest = TOKEN_CHECK_ID;
    try {
      const status = await call("other", () => services.tokenStatus(TOKEN_CHECK_ID, true));
      tokenRequest = null;
      dispatch({ type: "tokenChecked", status, at: clock.now() });
      afterAuthorized(status);
    } catch (err) {
      tokenRequest = null;
      dispatch({ type: "tokenCheckFailed", error: mastErrorOf(err) });
    }
  }

  function maybeAutoCheck(): void {
    const s = get();
    const status = s.token.status;
    if (autoChecked || s.form.mission !== "roman" || s.token.checking || s.token.expired || s.token.outcome) return;
    if (!status?.present || status.authorized !== null) return;
    autoChecked = true;
    void checkToken();
  }

  function cancelTokenCheck(): void {
    if (!get().token.checking || !tokenRequest) return;
    services.cancel({ requestIds: [tokenRequest] }).catch(() => 0);
  }

  async function clearToken(): Promise<void> {
    cancelTokenCheck();
    try {
      await call("other", () => services.tokenClear());
      dispatch({ type: "tokenCleared" });
    } catch (err) {
      dispatch({ type: "toast", text: `Could not remove the token: ${mastErrorOf(err).message}` });
    }
  }

  async function openTokenPage(): Promise<void> {
    try {
      await services.openUrl(MAST_TOKEN_PAGE_URL);
    } catch {
      dispatch({ type: "toast", text: OPEN_PAGE_FAILED_TEXT });
    }
  }

  async function resolveStep(generation: number, name: string, form: ArchiveForm, available: readonly QuickFilterId[], sort: SortKey[]): Promise<MastQuery | null> {
    const started = clock.now();
    try {
      const resolved = await call("other", () => services.resolve(resolveId(generation), name));
      if (!isCurrent(generation)) return null;
      if (!resolved) {
        dispatch({ type: "nameNotResolved", generation, name, elapsedMs: clock.now() - started });
        return null;
      }
      const plan = planSearch(form, available, { resolved, sort });
      if (plan.kind !== "search") return null;
      dispatch({ type: "nameResolved", generation, resolved, query: plan.query, at: clock.now() });
      return plan.query;
    } catch (err) {
      const e = mastErrorOf(err);
      if (e.code === "mast_cancelled" || !isCurrent(generation)) return null;
      if (!LOOKUP_FALLBACK.has(e.code)) {
        dispatch({ type: "searchFailed", generation, error: e, at: clock.now() });
        return null;
      }
      const plan = planSearch(form, available, { serverResolve: true, sort });
      if (plan.kind !== "search") return null;
      dispatch({ type: "serverResolve", generation, query: plan.query, at: clock.now() });
      return plan.query;
    }
  }

  async function search(opts: { refresh?: boolean; sort?: SortKey[] } = {}): Promise<void> {
    const s = get();
    const mission = missionOf(s);
    if (!mission || !gateOpen(s)) return;
    const form = s.form;
    const sort = opts.sort ?? [];
    const available = mission.quickFilters.map((f) => f.id);
    const plan = planSearch(form, available, { sort });
    if (plan.kind === "blocked") {
      dispatch({ type: "queryBlocked", field: plan.field, message: plan.message });
      return;
    }
    const generation = s.generation + 1;
    cancelAllButThumbs(s.generation);
    const meaning = interpretQuery(form.text, form.kindOverride);
    const kind: QueryKind = meaning.kind === "empty" ? "name" : meaning.kind;
    dispatch({
      type: "searchStarted",
      generation,
      text: form.text.trim(),
      kind,
      name: plan.kind === "resolve" ? plan.name : null,
      query: plan.kind === "search" ? plan.query : null,
      at: clock.now(),
    });
    const query = plan.kind === "resolve" ? await resolveStep(generation, plan.name, form, available, sort) : plan.query;
    if (!query || !isCurrent(generation)) return;
    try {
      const page = await call("other", () => services.search(searchId(generation), query, opts.refresh ?? false));
      dispatch({ type: "pageLoaded", generation, page });
    } catch (err) {
      const e = mastErrorOf(err);
      if (e.code !== "mast_cancelled") dispatch({ type: "searchFailed", generation, error: e, at: clock.now() });
    }
  }

  function cancelSearch(): void {
    const run = get().run;
    if (!run) return;
    services.cancel({ prefix: generationPrefix(run.generation) }).catch(() => 0);
    dispatch({ type: "searchCancelled", generation: run.generation });
  }

  async function loadMore(opts: { auto?: boolean } = {}): Promise<void> {
    const s = get();
    if (!s.results || s.run || s.loadingMore !== null || !s.resultsQuery) return;
    const offset = nextOffset(s.results);
    if (offset === null) return;
    const generation = s.generation;
    if (opts.auto && failedPage?.generation === generation && failedPage.offset === offset) return;
    const query: MastQuery = { ...s.resultsQuery, offset };
    dispatch({ type: "pageRequested", generation, offset });
    try {
      const page = await call("other", () => services.search(pageId(generation, offset), query, false));
      failedPage = null;
      dispatch({ type: "pageLoaded", generation, page });
    } catch (err) {
      const error = mastErrorOf(err);
      if (error.code !== "mast_cancelled") failedPage = { generation, offset };
      dispatch({ type: "pageFailed", generation, error, at: clock.now() });
    }
  }

  function moveTo(form: ArchiveForm): void {
    const s = get();
    cancelEveryThumb();
    cancelGeneration(s.generation);
    dispatch({ type: "missionSwitched", form, generation: s.generation + 1 });
  }

  function switchMission(id: MastMissionId): void {
    const s = get();
    if (id === s.form.mission || !enabled(s, id)) return;
    const rerun = hasSearched(s) && s.form.text.trim() !== "";
    moveTo(switchMissionForm(s.form, id, storage));
    if (rerun) void search();
    if (id === "roman") maybeAutoCheck();
  }

  async function checkMission(id: MastMissionId): Promise<void> {
    if (get().probes[id]?.stage === "checking") return;
    dispatch({ type: "missionProbeStarted", mission: id, at: clock.now() });
    try {
      const mission = await call("other", () => services.probeMission(id));
      const wasUnavailable = get().form.mission === id && resultsState(get()) === "unavailable";
      dispatch({ type: "missionProbed", mission, at: clock.now() });
      const s = get();
      if (wasUnavailable && s.form.mission === id && resultsState(s) !== "unavailable" && s.form.text.trim() && !s.run) void search();
    } catch (err) {
      dispatch({ type: "missionProbeFailed", mission: id, error: mastErrorOf(err), at: clock.now() });
    }
  }

  async function tryMission(id: MastMissionId): Promise<void> {
    const s = get();
    if (id !== s.form.mission) {
      if (!enabled(s, id)) return;
      moveTo(switchMissionForm(s.form, id, storage));
    }
    await search();
  }

  function openRomanAccess(): void {
    const go = () => {
      if (get().form.mission !== "roman") switchMission("roman");
      if (get().form.mission !== "roman") return;
      dispatch({ type: "tokenPanel", open: true });
      maybeAutoCheck();
    };
    if (get().missions.length) go();
    else void init().then(go);
  }

  function setFilters(patch: Partial<MastFilterValues>, opts: { rerun?: boolean } = {}): void {
    const s = get();
    dispatch({ type: "formChanged", patch: { filters: cleanFilters({ ...s.form.filters, ...patch }) } });
    if (patch.level) saveLevel(storage, patch.level);
    if (opts.rerun && hasSearched(s)) void search();
  }

  function setPreset(id: string | null, opts: { rerun?: boolean } = {}): void {
    const s = get();
    dispatch({ type: "formChanged", patch: { preset: id } });
    if (opts.rerun && hasSearched(s)) void search();
  }

  async function removeFilter(id: ChipId): Promise<void> {
    setFilters(CHIP_PATCH[id]);
    await search();
  }

  async function widen(radiusArcmin: number): Promise<void> {
    dispatch({ type: "formChanged", patch: { radiusText: String(radiusArcmin) } });
    await search();
  }

  async function runSuggestion(suggestion: Suggestion): Promise<void> {
    await init();
    const s = get();
    if (!enabled(s, suggestion.mission)) return;
    const fields = { text: suggestion.text, kindOverride: null, radiusText: String(DEFAULT_RADIUS_ARCMIN), filters: { ...suggestion.filters }, preset: null };
    if (suggestion.mission !== s.form.mission) moveTo({ ...switchMissionForm(s.form, suggestion.mission, storage), ...fields });
    else dispatch({ type: "formChanged", patch: fields });
    if (suggestion.filters.level) saveLevel(storage, suggestion.filters.level);
    await search();
  }

  async function searchAround(target: AroundTarget): Promise<void> {
    await init();
    const s = get();
    const fields = { text: aroundQueryText(target), kindOverride: null, radiusText: String(target.radiusArcmin) };
    const mission = missionOf(s);
    if (!mission || !gateOpen(s) || !mission.capabilities.images) moveTo({ ...switchMissionForm(s.form, "jwst", storage), ...fields });
    else dispatch({ type: "formChanged", patch: fields });
    await search();
  }

  function galleryAllowed(s: ArchiveState): boolean {
    const m = missionOf(s);
    return !m || (m.capabilities.images && m.views.gallery);
  }

  function sortBy(column: MastColumn, additive: boolean): void {
    const s = get();
    const results = s.results;
    if (!results || s.run) return;
    const mode = sortMode(results, column);
    if (mode === "local") {
      dispatch({ type: "sortSet", sort: { keys: nextSort(s.sort.by === "local" ? s.sort.keys : [], column.name, additive), by: "local" } });
      return;
    }
    if (mode === "none") {
      dispatch({ type: "announce", text: SORT_BLOCKED_HINT });
      return;
    }
    void search({ sort: nextSort(s.sort.by === "server" ? s.sort.keys : [], column.name, additive) });
  }

  function retry(): void {
    const s = get();
    if (s.run) return;
    if (s.error || s.notResolved) {
      void search();
      return;
    }
    const banner = visibleBanner(s, clock.now());
    if (banner?.retry === "search") void search();
    else if (banner?.retry === "page") void loadMore();
    else if (banner?.retry === "resume") dispatch({ type: "thumbsResumed" });
    else if (banner?.retry === "thumbs" || failedCount(s.thumbs) > 0) dispatch({ type: "thumbsRetried" });
  }

  function schedulePause(s: ArchiveState): void {
    const q = s.thumbs;
    const at = q.pausedUntil !== null && !q.gaveUp ? q.pausedUntil : null;
    if (pauseTimer && pauseTimer.at === at) return;
    if (pauseTimer) clock.clearTimeout(pauseTimer.handle);
    pauseTimer = null;
    if (at === null) return;
    const handle = clock.setTimeout(() => {
      pauseTimer = null;
      dispatch({ type: "thumbsTick", now: clock.now() });
    }, Math.max(0, at - clock.now()));
    pauseTimer = { at, handle };
  }

  function startable(s: ArchiveState): string[] {
    if (!s.results || s.run) return [];
    const mission = missionOf(s, s.results.mission);
    if (!mission?.capabilities.images) return [];
    return nextStarts(s.thumbs, clock.now());
  }

  function noteFailure(error: MastError): void {
    if (QUIET_FAILURES.has(error.code)) return;
    if (!failureBurst) {
      const handle = clock.setTimeout(() => {
        const burst = failureBurst;
        failureBurst = null;
        if (!burst) return;
        let top: { count: number; error: MastError } | null = null;
        for (const entry of burst.byCode.values()) if (!top || entry.count > top.count) top = entry;
        if (!top) return;
        const view = slotError(top.error, { surface: "thumb" });
        const noun = burst.count === 1 ? "thumbnail" : "thumbnails";
        dispatch({ type: "announce", text: `${burst.count} ${noun} failed: ${[view?.title ?? top.error.message, view?.meta].filter(Boolean).join(", ")}` });
      }, FAILURE_BURST_MS);
      failureBurst = { count: 0, byCode: new Map(), handle };
    }
    failureBurst.count += 1;
    const seen = failureBurst.byCode.get(error.code);
    failureBurst.byCode.set(error.code, { count: (seen?.count ?? 0) + 1, error: seen?.error ?? error });
  }

  function startThumb(s: ArchiveState, key: string): void {
    const results = s.results;
    if (!results) return;
    const generation = s.generation;
    const mission = results.mission;
    const strip = parseStripKey(key);
    const datasetId = strip?.datasetId ?? key;
    const filename = strip?.filename ?? null;
    const row = results.rows.find((r) => r.datasetId === datasetId);
    const requestId = thumbId(generation, datasetId, filename);
    const older = thumbRequests.get(key);
    if (older !== undefined && older !== requestId) supersededThumbs.set(older, key);
    thumbRequests.set(key, requestId);
    const finish = (apply: () => void) => {
      supersededThumbs.delete(requestId);
      if (thumbRequests.get(key) !== requestId) return;
      thumbRequests.delete(key);
      if (get().generation !== generation) {
        dispatch({ type: "thumbDropped", key });
        return;
      }
      apply();
    };
    call("thumb", () => services.thumbnail(requestId, mission, datasetId, row?.levelClass ?? "unknown", filename))
      .then(async (ref) => {
        const url = await services.previewUrl(ref.path);
        finish(() => dispatch({ type: "thumbReady", key, ref: { url, width: ref.width, height: ref.height, imageCount: ref.imageCount, filename: ref.filename } }));
      })
      .catch((err) => {
        const e = mastErrorOf(err);
        finish(() => {
          if (e.code === "mast_cancelled") dispatch({ type: "thumbDropped", key });
          else if (e.code === "mast_rate_limited") {
            const c = e.context;
            dispatch({ type: "thumbRateLimited", key, at: clock.now(), retryAfterSec: c.retryAfterSec ?? DEFAULT_RETRY_AFTER_SEC, consecutive: c.consecutive ?? 1, status: c.status ?? null });
          } else {
            dispatch({ type: "thumbFailed", key, error: e, at: clock.now() });
            noteFailure(e);
          }
        });
      });
  }

  function pump(): void {
    if (pumping) return;
    pumping = true;
    try {
      pruneThumbs(get());
      for (;;) {
        const s = get();
        schedulePause(s);
        const starts = startable(s);
        if (starts.length === 0) return;
        dispatch({ type: "thumbsStarted", keys: starts, at: clock.now() });
        for (const key of starts) startThumb(s, key);
      }
    } finally {
      pumping = false;
    }
  }

  const unsubscribe = store.subscribe(pump);
  const unsubscribeDetail = store.subscribe(detail.sync);

  function setOf(s: ArchiveState, datasetId: string | null) {
    return s.results && datasetId ? s.imageSets[imageSetKey(s.results.mission, datasetId)] : undefined;
  }

  function openLarge(datasetId: string, imageKey: string | null = null): void {
    const s = get();
    const mission = s.results ? missionOf(s, s.results.mission) : undefined;
    if (!mission?.capabilities.images || !s.results?.rows.some((r) => r.datasetId === datasetId)) return;
    const key = imageKey ?? (datasetId === s.selectedId ? (detailEntry(s)?.key ?? null) : null);
    dispatch({ type: "largeOpened", datasetId, imageKey: key });
  }

  function closeLarge(returnFocus = true): void {
    if (!get().large.open) return;
    dispatch({ type: "largeClosed" });
    const id = get().selectedId;
    if (id && returnFocus) dispatch({ type: "focusRequested", datasetId: id });
  }

  function largeStepImage(dir: 1 | -1): void {
    const s = get();
    const set = s.large.open ? setOf(s, s.large.datasetId) : undefined;
    const entry = largeEntry(s);
    if (!set || !entry) return;
    const next = stepEntry(set, entry.key, dir);
    if (next) dispatch({ type: "largeImage", imageKey: next });
  }

  function largeStepDataset(dir: 1 | -1): void {
    const s = get();
    if (!s.large.open || !s.results) return;
    const ids = [...new Set(displayRows(s.results, s.sort).map((r) => r.datasetId))];
    const at = s.large.datasetId ? ids.indexOf(s.large.datasetId) : -1;
    const next = at >= 0 ? ids[at + dir] : undefined;
    if (next) dispatch({ type: "largeDataset", datasetId: next });
  }

  function stepImage(dir: 1 | -1): void {
    const s = get();
    const set = setOf(s, s.selectedId);
    const entry = detailEntry(s);
    if (!set || !entry) return;
    const next = stepEntry(set, entry.key, dir);
    if (next) dispatch({ type: "imageSelected", imageKey: next });
  }

  return {
    init,
    setActive: (active) => dispatch({ type: "archiveActive", active }),
    setText: (text) => dispatch({ type: "formChanged", patch: { text } }),
    setKindOverride: (kind) => dispatch({ type: "formChanged", patch: { kindOverride: kind } }),
    setRadius: (radiusText) => dispatch({ type: "formChanged", patch: { radiusText } }),
    setFilters,
    setPreset,
    removeFilter,
    switchMission,
    tryMission,
    checkMission,
    openRomanAccess,
    saveToken,
    checkToken,
    cancelTokenCheck,
    clearToken,
    openTokenPage,
    replaceToken: (open) => dispatch({ type: "tokenReplace", open }),
    setRomanPanel: (open) => dispatch({ type: "tokenPanel", open }),
    tokenFocusDone: (seq) => dispatch({ type: "tokenFocusDone", seq }),
    search,
    cancelSearch,
    loadMore,
    retry,
    runSuggestion,
    searchAround,
    widen,
    select: (datasetId, rowKey) => dispatch({ type: "selected", datasetId, rowKey: rowKey ?? null }),
    toggleView: () => {
      if (galleryAllowed(get())) dispatch({ type: "viewToggled" });
    },
    setView: (view) => {
      if (view === "table" || galleryAllowed(get())) dispatch({ type: "viewSet", view });
    },
    toggleTableThumbs: () => dispatch({ type: "tableThumbsToggled" }),
    sortBy,
    setThumbDemand: (demand) => {
      cardDemand = { selected: [...demand.selected], visible: [...demand.visible], ahead: [...demand.ahead], firstRow: demand.firstRow };
      pushDemand();
    },
    retryThumbs: (keys) => dispatch({ type: "thumbsRetried", keys }),
    resumeThumbs: () => dispatch({ type: "thumbsResumed" }),
    dismissBanner: (kind) => dispatch({ type: "bannerDismissed", kind }),
    async copy(text, label = text) {
      try {
        await services.copyText(text);
        dispatch({ type: "toast", text: `Copied ${label}` });
      } catch {
        dispatch({ type: "toast", text: "Could not copy to the clipboard" });
      }
    },
    dismissToast: () => dispatch({ type: "toastDismissed" }),
    async clearCache() {
      try {
        const info = await services.cacheClear("all");
        dispatch({ type: "cacheCleared" });
        dispatch({ type: "toast", text: `MAST cache cleared: ${formatBytes(info.bytes)} freed` });
      } catch (err) {
        dispatch({ type: "toast", text: `Could not clear the MAST cache: ${mastErrorOf(err).message}` });
      }
    },
    imageMissing: (url) => dispatch({ type: "imageMissing", url }),
    setNarrow: (narrow) => dispatch({ type: "narrowSet", narrow }),
    setDetailTab: (tab) => dispatch({ type: "detailTab", tab }),
    toggleDetail: () => {
      const d = get().detail;
      if (d.narrow) dispatch({ type: "drawerSet", open: !d.drawerOpen });
      else dispatch({ type: "detailToggled" });
    },
    setDetailWidth: (width) => dispatch({ type: "detailResized", width }),
    openDrawer: () => dispatch({ type: "drawerSet", open: true }),
    closeDrawer: () => {
      dispatch({ type: "drawerSet", open: false });
      const id = get().selectedId;
      if (id) dispatch({ type: "focusRequested", datasetId: id });
    },
    selectImage: (imageKey) => dispatch({ type: "imageSelected", imageKey }),
    selectGroup: (group) => dispatch({ type: "groupSelected", group }),
    cycleGroup: () => dispatch({ type: "groupCycled" }),
    stepImage,
    openLarge,
    closeLarge,
    largeStepImage,
    largeStepDataset,
    largeSelectImage: (imageKey) => dispatch({ type: "largeImage", imageKey }),
    cancelPreview: (size) => detail.cancelPreview(size),
    retryDetail: () => detail.retry(),
    retryLarge: () => detail.retryLarge(),
    refreshProducts: () => detail.refreshProducts(),
    cancelProducts: () => detail.cancelProducts(),
    setProductMode: (mode) => dispatch({ type: "productModeSet", mode }),
    toggleHiddenProducts: () => dispatch({ type: "productHiddenToggled" }),
    dispose() {
      unsubscribe();
      unsubscribeDetail();
      detail.dispose();
      progressOff?.();
      progressOff = null;
      if (pauseTimer) clock.clearTimeout(pauseTimer.handle);
      pauseTimer = null;
      if (failureBurst) clock.clearTimeout(failureBurst.handle);
      failureBurst = null;
    },
  };
}

const realClock: ArchiveClock = {
  now: () => Date.now(),
  setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
  clearTimeout: (handle) => globalThis.clearTimeout(handle as ReturnType<typeof globalThis.setTimeout>),
};

export function defaultArchiveServices(): ArchiveServices {
  return {
    missions: mastMissions,
    presets: mastPresets,
    probeMission: mastProbeMission,
    resolve: mastResolve,
    search: mastSearch,
    imageList: mastImageList,
    thumbnail: mastThumbnail,
    preview: mastPreview,
    products: mastProducts,
    cancel: mastCancel,
    tokenStatus: mastTokenStatus,
    tokenSave: mastTokenSave,
    tokenClear: mastTokenClear,
    cacheInfo: mastCacheInfo,
    cacheClear: mastCacheClear,
    onProgress: onMastProgress,
    previewUrl: getPreviewUrl,
    wcsInfo: getWcsInfo,
    copyText: (text) => navigator.clipboard.writeText(text),
    openUrl: async (url) => {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    },
  };
}

export const archiveController: ArchiveController = createArchiveController({
  store: archiveStore,
  services: defaultArchiveServices(),
  clock: realClock,
  storage: browserLevelStorage(),
  inflight: archiveInflight,
});
