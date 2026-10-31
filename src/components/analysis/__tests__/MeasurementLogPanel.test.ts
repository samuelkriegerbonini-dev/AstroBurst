import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";

const backend = vi.hoisted(() => ({ saved: [] as string[] }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: { name: "m16.fits", path: "C:/data/m16.fits" } }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async (dialog: { defaultPath: string }) => `C:/out/${dialog.defaultPath}` }));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeTextFile: async (path: string) => void backend.saved.push(path) }));

import { hooks } from "../../../test/hookRuntime";
import { measurementLog } from "../../../utils/measurementLog";

const { default: MeasurementLogPanel } = await import("../MeasurementLogPanel");

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

type Panel = () => unknown;
const panel = (MeasurementLogPanel as unknown as { type: Panel }).type;
const show = () => hooks.render(panel, undefined) as ReactNode;
const notice = () => elements(show()).find((p) => typeof p.className === "string" && p.className.includes("text-emerald-400/90"));
const button = (text: string) =>
  elements(show()).find((p) => p.children !== undefined && textOf(p.children as ReactNode).trim().startsWith(text));

function click(text: string): void {
  const found = button(text);
  expect(found?.disabled, `${text} is enabled`).not.toBe(true);
  (found!.onClick as () => void)();
}

async function savedOnce(): Promise<void> {
  click("Save CSV");
  await vi.waitFor(() => expect(backend.saved).toHaveLength(1));
  expect(textOf(notice()?.children as ReactNode)).toBe(`Saved ${backend.saved[0]}`);
}

describe("MeasurementLogPanel Saved notice", () => {
  beforeEach(() => {
    hooks.mount();
    backend.saved = [];
    measurementLog.clear();
    measurementLog.append({
      kind: "photometry",
      file: "m16.fits",
      image: "original",
      dq: "off",
      unit: "DN",
      source: "aperture r=5 px at (10.0, 12.0)",
      params: { r_ap: 5 },
      values: { net_flux: 1234.5 },
      notes: [],
    });
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true });
  });

  afterEach(() => {
    hooks.mount();
    measurementLog.clear();
    vi.useRealTimers();
  });

  it("cancels the notice timer when the panel closes", async () => {
    await savedOnce();
    expect(vi.getTimerCount(), "the notice timer runs while the panel is open").toBe(1);
    hooks.mount();
    expect(vi.getTimerCount(), "no timer outlives the panel").toBe(0);
  });

  it("shows each save in one timed notice that a later save restarts", async () => {
    await savedOnce();
    await vi.advanceTimersByTimeAsync(3_000);
    click("Save CSV");
    await vi.waitFor(() => expect(backend.saved).toHaveLength(2));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(notice(), "the second save is still shown 5 s after it").toBeDefined();
    await vi.advanceTimersByTimeAsync(1_500);
    expect(notice()).toBeUndefined();
  });

  it("drops the notice and its timer when the log is cleared", async () => {
    await savedOnce();
    click("Clear (1)");
    click("Confirm");
    expect(notice()).toBeUndefined();
    expect(vi.getTimerCount(), "neither the notice nor the confirm timer is left").toBe(0);
  });
});
