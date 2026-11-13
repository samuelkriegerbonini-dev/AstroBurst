import { useSyncExternalStore } from "react";
import type { TableProbe } from "../shared/types/tables";
import type { TableRouteKind } from "../utils/fileKinds";

export interface TableRouteSource {
  path: string;
  name: string;
  probe: TableProbe;
}

export interface TableRoute extends TableRouteSource {
  kind: TableRouteKind;
  seq: number;
}

type Listener = () => void;

export class TableRouteStore {
  private routes: Partial<Record<TableRouteKind, TableRoute>> = {};
  private listeners = new Set<Listener>();
  private seq = 0;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  get = (kind: TableRouteKind): TableRoute | null => this.routes[kind] ?? null;

  private emit(): void {
    this.listeners.forEach((l) => l());
  }

  request(kind: TableRouteKind, table: TableRouteSource): TableRoute {
    this.seq++;
    const route: TableRoute = { ...table, kind, seq: this.seq };
    this.routes = { ...this.routes, [kind]: route };
    this.emit();
    return route;
  }

  clear(kind: TableRouteKind): void {
    if (!this.routes[kind]) return;
    const rest = { ...this.routes };
    delete rest[kind];
    this.routes = rest;
    this.emit();
  }

  reset(): void {
    if (Object.keys(this.routes).length === 0) return;
    this.routes = {};
    this.emit();
  }
}

export const tableRouteStore = new TableRouteStore();

export function useTableRoute(kind: TableRouteKind): TableRoute | null {
  return useSyncExternalStore(tableRouteStore.subscribe, () => tableRouteStore.get(kind), () => tableRouteStore.get(kind));
}
