import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import type { ConeSearchResult } from "../../../shared/types/catalog";

const WCS_PATH = "C:/data/m16.fits";

const backend = vi.hoisted(() => ({
  cone: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  exported: [] as string[],
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/catalog", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/catalog")>()),
  catalogConeSearch: backend.cone,
  catalogExportCsv: async (_path: string, outputPath: string) => {
    backend.exported.push(outputPath);
    return { output_path: outputPath, n: 1 };
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async (dialog: { defaultPath: string }) => `C:/out/${dialog.defaultPath}` }));

import { hooks } from "../../../test/hookRuntime";

const { default: CatalogPanel } = await import("../CatalogPanel");

type Props = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): Props[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Props>(node)) return [];
  return [node.props, ...elements(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<Props>(node) ? textOf(node.props.children) : "";
}

const CONE: ConeSearchResult = {
  rows: [
    {
      id: "Gaia DR3 4146599224155426304", ra: 274.7, dec: -13.8, ra_epoch: 274.7, dec_epoch: -13.8, pm_ra_masyr: null,
      pm_dec_masyr: null, g: 12.1, bp: 12.5, rp: 11.6, bp_rp: 0.9, parallax_mas: null, x: 100, y: 120, on_image: true,
    },
  ],
  epoch_year: 2024.5,
  n_total: 1,
  n_on_image: 1,
  radius_arcmin: 12,
  center_ra: 274.7,
  center_dec: -13.8,
  elapsed_ms: 4,
  warnings: [],
};

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

type Panel = (props: { filePath: string | null }) => unknown;
const panel = (CatalogPanel as unknown as { type: Panel }).type;
const show = () => hooks.render(panel, { filePath: WCS_PATH }) as ReactNode;
const notice = () => elements(show()).find((p) => typeof p.className === "string" && p.className.includes("text-emerald-400/90"));

function exportCatalogCsv(): void {
  const button = elements(show()).find((p) => p.children !== undefined && textOf(p.children as ReactNode).trim() === "Catalog CSV");
  expect(button?.disabled, "Catalog CSV is enabled").toBe(false);
  (button!.onClick as () => void)();
}

async function searched(): Promise<void> {
  const search = elements(show()).find((p) => p.label === "Search Gaia DR3");
  (search!.onClick as () => void)();
  await flush();
}

describe("CatalogPanel Saved notice", () => {
  beforeEach(() => {
    hooks.mount();
    backend.cone.mockReset().mockResolvedValue(CONE);
    backend.exported = [];
  });

  afterEach(() => {
    hooks.mount();
    vi.useRealTimers();
  });

  it("cancels the notice timer when the panel closes", async () => {
    await searched();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true });
    exportCatalogCsv();
    await vi.waitFor(() => expect(backend.exported).toHaveLength(1));
    expect(textOf(notice()?.children as ReactNode)).toBe(`Saved ${backend.exported[0]}`);
    expect(vi.getTimerCount(), "the notice timer runs while the panel is open").toBe(1);
    hooks.mount();
    expect(vi.getTimerCount(), "no timer outlives the panel").toBe(0);
  });

  it("shows each export in one timed notice that a later export restarts", async () => {
    await searched();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true });
    exportCatalogCsv();
    await vi.waitFor(() => expect(backend.exported).toHaveLength(1));
    await vi.advanceTimersByTimeAsync(3_000);
    exportCatalogCsv();
    await vi.waitFor(() => expect(backend.exported).toHaveLength(2));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(notice(), "the second export is still shown 5 s after it").toBeDefined();
    await vi.advanceTimersByTimeAsync(1_500);
    expect(notice()).toBeUndefined();
  });
});
