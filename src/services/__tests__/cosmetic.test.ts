import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CosmeticConfig, CosmeticOptions } from "../../shared/types/cosmetic";

const { withPreviewMock } = vi.hoisted(() => ({ withPreviewMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ withPreview: withPreviewMock }));

import { cosmeticCorrect, cosmeticDryRun } from "../cosmetic";

const CONFIG: CosmeticConfig = {
  use_master_dark: false,
  dark_hot_sigma: null,
  dark_cold_sigma: null,
  auto_hot_sigma: 3,
  auto_cold_sigma: null,
  defects: [],
  cfa: null,
  amount: 1,
  replacement: "median",
};
const OPTIONS: CosmeticOptions = { masterDarkPath: null, config: CONFIG, defectListText: "Point 1 2" };
const LIGHT = "C:/r7/b3/cosmetic_stars_hot.fits";

describe("cosmetic service", () => {
  beforeEach(() => {
    withPreviewMock.mockReset();
    withPreviewMock.mockResolvedValue({});
  });

  it("asks the cosmetic command for a dry run with the same options as a run", async () => {
    await cosmeticDryRun(LIGHT, "C:/out", OPTIONS);
    expect(withPreviewMock).toHaveBeenCalledWith("cosmetic_correct_cmd", "C:/out", {
      path: LIGHT,
      masterDarkPath: null,
      config: CONFIG,
      defectListText: "Point 1 2",
      dryRun: true,
    });
  });

  it("leaves the dry run off for a real correction", async () => {
    await cosmeticCorrect(LIGHT, "C:/out", OPTIONS);
    const args = withPreviewMock.mock.calls[0][2] as Record<string, unknown>;
    expect(args.dryRun).not.toBe(true);
    expect(args.path).toBe(LIGHT);
  });
});
