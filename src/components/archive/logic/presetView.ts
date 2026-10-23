import type { MastMissionId, MastPreset, MastProduct, MastProductClass } from "../../../shared/types/mast";

const LEFT_OUT: readonly MastProductClass[] = ["preview", "reference", "guidestar"];

export function presetsFor(presets: readonly MastPreset[], mission: MastMissionId): MastPreset[] {
  return presets.filter((p) => p.mission === mission);
}

export function presetById(presets: readonly MastPreset[], id: string | null | undefined): MastPreset | null {
  return id ? (presets.find((p) => p.id === id) ?? null) : null;
}

export function conditionText(preset: MastPreset): string {
  return preset.conditions.length ? preset.conditions.map((c) => `${c.column} = ${c.value}`).join(", ") : "no MAST condition";
}

export function presetTitle(preset: MastPreset): string {
  return `${preset.description}. MAST conditions: ${conditionText(preset)}. Files: ${preset.products.join(", ")}`;
}

export const PURE_PARALLEL_PRESET = "pure_parallel";
export const PURE_PARALLEL_NOTE = "Pure-parallel visits carry no proposal category, so Category and Object type match nothing.";

const LEVEL_COLUMN = "productLevel";

export function presetLevel(preset: MastPreset | null): string | null {
  return preset?.conditions.find((c) => c.column === LEVEL_COLUMN)?.value ?? null;
}

export function levelLockText(preset: MastPreset | null): string | null {
  const level = presetLevel(preset);
  return preset && level ? `Level ${level} set by preset ${preset.label}` : null;
}

export function droppedText(columns: readonly string[]): string | null {
  return columns.length ? `MAST ignored ${columns.join(", ")}` : null;
}

export function presetProducts(preset: MastPreset, products: readonly MastProduct[]): MastProduct[] {
  return products.filter((p) => preset.products.includes(p.suffix) && !LEFT_OUT.includes(p.cls));
}
