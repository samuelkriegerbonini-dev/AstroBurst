import { useSyncExternalStore } from "react";
import { readTable } from "../services/tables";
import type { LightCurveQuantity, LightCurveTimeScale } from "../shared/types/lightCurve";
import type { TableResult } from "../shared/types/tables";
import { lightCurveDocFromTable, tableGroupKey } from "../utils/lightCurveDoc";
import { lightCurveDocsFromTable, planLightCurveTable, type LightCurveTablePlan } from "../utils/lightCurveTable";
import { LightCurveStore, lightCurveStore } from "./useLightCurveStore";

export interface TableColumnsChoice {
  timeColumn: string;
  timeScale: LightCurveTimeScale;
  valueColumn: string;
  quantity: LightCurveQuantity;
  errColumn: string | null;
}

export interface LightCurveTableState {
  path: string | null;
  loading: boolean;
  error: string | null;
  table: TableResult | null;
  plan: LightCurveTablePlan | null;
  manual: TableColumnsChoice | null;
}

export const EMPTY_TABLE_STATE: LightCurveTableState = Object.freeze({
  path: null,
  loading: false,
  error: null,
  table: null,
  plan: null,
  manual: null,
});

export const NO_SHARED_ROW_ERROR = "no row has both a time and a value in these columns";

export type TableReader = (path: string) => Promise<TableResult>;

type Listener = () => void;

const TABLE_GROUP_PREFIX = tableGroupKey("");

export function tablePathOf(group: string): string {
  return group.startsWith(TABLE_GROUP_PREFIX) ? group.slice(TABLE_GROUP_PREFIX.length) : group;
}

function messageOf(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

export class LightCurveTableLoader {
  private state: LightCurveTableState = EMPTY_TABLE_STATE;
  private seq = 0;
  private listeners = new Set<Listener>();

  constructor(
    private readonly read: TableReader = readTable,
    private readonly store: LightCurveStore = lightCurveStore,
  ) {}

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getState(): LightCurveTableState {
    return this.state;
  }

  private write(next: LightCurveTableState): void {
    this.state = Object.freeze(next);
    this.listeners.forEach((l) => l());
  }

  private storePlan(path: string, table: TableResult, plan: LightCurveTablePlan): void {
    const group = tableGroupKey(path);
    const docs = lightCurveDocsFromTable(table, path, plan);
    if (docs.length > 0) this.store.put(group, docs);
    else this.store.remove(group);
    this.store.setActiveTable(group);
  }

  async open(path: string): Promise<void> {
    const seq = ++this.seq;
    this.write({ path, loading: true, error: null, table: null, plan: null, manual: null });
    this.store.setActiveTable(tableGroupKey(path));
    try {
      const table = await this.read(path);
      if (seq !== this.seq) return;
      const plan = planLightCurveTable(table);
      this.storePlan(path, table, plan);
      this.write({ path, loading: false, error: plan.notes[0] ?? null, table, plan, manual: null });
    } catch (e) {
      if (seq !== this.seq) return;
      this.write({ path, loading: false, error: messageOf(e), table: null, plan: null, manual: null });
    }
  }

  async activate(group: string): Promise<void> {
    const { path, table } = this.state;
    if (path !== null && table !== null && group === tableGroupKey(path)) {
      this.store.setActiveTable(group);
      return;
    }
    await this.open(tablePathOf(group));
  }

  applyColumns(choice: TableColumnsChoice): void {
    const { path, table } = this.state;
    if (path === null || table === null) return;
    let doc;
    try {
      doc = lightCurveDocFromTable(table, { path, ...choice });
    } catch (e) {
      this.write({ ...this.state, error: messageOf(e) });
      return;
    }
    if (doc.time.length === 0) {
      this.write({ ...this.state, error: NO_SHARED_ROW_ERROR });
      return;
    }
    this.store.put(tableGroupKey(path), [doc]);
    this.write({ ...this.state, manual: choice, error: null });
  }

  automatic(): void {
    const { path, table, plan } = this.state;
    if (path === null || table === null || plan === null) return;
    this.storePlan(path, table, plan);
    this.write({ ...this.state, manual: null, error: plan.notes[0] ?? null });
  }

  close(): void {
    this.store.setActiveTable(null);
  }
}

export const lightCurveTableLoader = new LightCurveTableLoader();

export function openLightCurveTable(path: string): Promise<void> {
  return lightCurveTableLoader.open(path);
}

function tableStateSnapshot(): LightCurveTableState {
  return lightCurveTableLoader.getState();
}

export function useLightCurveTableState(): LightCurveTableState {
  return useSyncExternalStore(lightCurveTableLoader.subscribe, tableStateSnapshot, tableStateSnapshot);
}
