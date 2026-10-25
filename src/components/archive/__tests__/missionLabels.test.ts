import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { CATEGORY_COLUMN, MISSION_LABELS, missionLabelOf } from "../logic/missionLabels";
import type { MastMissionId } from "../../../shared/types/mast";

const rust = readFileSync(fileURLToPath(new URL("../../../../src-tauri/src/core/mast/missions.rs", import.meta.url)), "utf8");

function rustLabels(): Record<string, string> {
  const body = rust.slice(rust.indexOf("pub fn label("), rust.indexOf("pub fn dataset_column("));
  const out: Record<string, string> = {};
  for (const m of body.matchAll(/MastMissionId::(\w+)\s*=>\s*"([^"]+)"/g)) out[m[1].replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase()] = m[2];
  return out;
}

describe("mission labels", () => {
  it("match the Rust labels entry for entry", () => {
    expect(Object.keys(rustLabels())).toHaveLength(10);
    expect(MISSION_LABELS).toEqual(rustLabels());
  });

  it("read the synthetic category column under the name Rust gives it", () => {
    expect(rust).toContain(`pub const CATEGORY_COLUMN: &str = "${CATEGORY_COLUMN}";`);
  });

  it("name a mission by its label and an unknown id by the id itself", () => {
    expect(missionLabelOf("roman")).toBe("Roman");
    expect(missionLabelOf("roman_cgi")).toBe("Roman CGI");
    expect(missionLabelOf("kepler" as MastMissionId)).toBe("kepler");
  });
});
