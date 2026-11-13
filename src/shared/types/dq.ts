import type { ErrOrigin } from "./layers";

export type DqTableName = "jwst" | "roman" | "hst" | "unknown";

export interface DqFlag {
  bit: number;
  name: string;
}

export interface DqFlagTable {
  table: DqTableName;
  label: string;
  flags: DqFlag[];
  default_mask: number;
  exclusion_mask: number;
  dq_ref: string | null;
  err_ref: string | null;
  source?: string;
  saturated_mask?: number;
  exclusion_default?: number;
  exclusion_edited?: boolean;
  dq_origin?: string | null;
  dq_unavailable?: string | null;
  err_origin?: ErrOrigin | null;
  err_unavailable?: string | null;
}

export interface DqProbe {
  bits: number;
  value: number;
  names: string[];
  table: DqTableName;
  text: string;
  source?: string;
}

export interface ErrProbe {
  value: number | null;
  unit: string | null;
  source?: string | null;
}

export interface DqMaskData {
  width: number;
  height: number;
  mask: number;
  tableId: number;
  cells: Uint8Array;
}

export interface DqOverlaySettings {
  enabled: boolean;
  mask: number;
}
