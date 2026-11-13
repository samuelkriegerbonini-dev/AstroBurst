export interface IdentifiedFilter {
  dataset: string;
  filter: string;
  pivot_um: number;
  instrument: string;
  source: string;
  warning: string | null;
}

export interface FilterNotIdentified {
  error: string;
}

export type FilterIdentification = IdentifiedFilter | FilterNotIdentified;
