import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";

const backend = vi.hoisted(() => ({ saved: [] as string[], texts: [] as string[] }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: { name: "m16.fits", path: "C:/data/m16.fits" } }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async (dialog: { defaultPath: string }) => `C:/out/${dialog.defaultPath}` }));
vi.mock("@tauri-apps/plugin-fs", () => ({
  writeTextFile: async (path: string, text: string) => {
    backend.saved.push(path);
    backend.texts.push(text);
  },
}));

import { hooks } from "../../../test/hookRuntime";
import { measurementLog } from "../../../utils/measurementLog";
import type { MeasurementKind, MeasurementLogDraft } from "../../../utils/measurementLog";
import { APP_VERSION } from "../../../utils/constants";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

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
  elements(show()).find((p) => typeof p.onClick === "function" && p.children !== undefined && textOf(p.children as ReactNode).trim().startsWith(text));

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
    backend.texts = [];
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

describe("MeasurementLogPanel kind filter", () => {
  const row = (kind: MeasurementKind): MeasurementLogDraft => ({
    kind,
    file: "m16.fits",
    image: "original",
    dq: "off",
    unit: null,
    source: "s",
    params: {},
    values: {},
    notes: [],
  });
  const optionValues = () => elements(show()).filter((p) => typeof p.value === "string" && p.onChange === undefined).map((p) => p.value);
  const select = () => elements(show()).find((p) => typeof p.value === "string" && typeof p.onChange === "function");

  beforeEach(() => {
    hooks.mount();
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
    measurementLog.append(row("layer_assignment"));
    measurementLog.append(row("tso_light_curve"));
  });

  afterEach(() => {
    hooks.mount();
    measurementLog.clear();
  });

  it("offers the kinds that have rows, the new ones included, in the order of MEASUREMENT_KINDS", () => {
    expect(optionValues()).toEqual(["all", "photometry", "tso_light_curve", "layer_assignment"]);
  });

  it("filters the table to a new kind", () => {
    (select()!.onChange as (e: { target: { value: string } }) => void)({ target: { value: "tso_light_curve" } });
    const rendered = elements(show());
    expect(rendered.find((p) => typeof p.value === "string" && typeof p.onChange === "function")?.value).toBe("tso_light_curve");
    expect(textOf(rendered.find((p) => p.className === "font-mono text-[9px] text-zinc-600")?.children as ReactNode)).toBe("1/3");
    const rows = rendered.filter((p) => p.className === "text-zinc-300");
    expect(rows).toHaveLength(1);
    expect(textOf(rows[0].children as ReactNode)).toContain("tso_light_curve");
  });
});

describe("MeasurementLogPanel export and clear confirmation", () => {
  const ISO = /^# exported_utc: \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/;
  const copyButton = () => elements(show()).find((p) => p.title === "Copy the shown rows as CSV");
  const group = () => elements(show()).find((p) => p.role === "group");
  const panelError = () => elements(show()).find((p) => "message" in p)?.message;
  const kindSelect = () => elements(show()).find((p) => typeof p.value === "string" && typeof p.onChange === "function");

  beforeEach(() => {
    hooks.mount();
    backend.saved = [];
    backend.texts = [];
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

  it("hands Copy CSV the shown rows with the provenance of the export", () => {
    const props = copyButton();
    expect(typeof props?.text).toBe("function");
    expect(props?.disabled).toBe(false);
    const lines = (props!.text as () => string | null)()!.split("\r\n");
    expect(lines[0]).toBe(`# software: AstroBurst ${APP_VERSION}`);
    expect(lines[1]).toMatch(ISO);
    expect(lines[2]).toBe("# file: m16.fits");
    expect(lines[3]).toBe("# rows: 1");
    expect(lines[4]).toBe("# kinds: photometry");
    expect(lines[8]).toBe("timestamp_utc,kind,file,image,dq,unit,source,notes,p_r_ap,v_net_flux");
    expect(lines[9].endsWith(',photometry,m16.fits,original,off,DN,"aperture r=5 px at (10.0, 12.0)",,5,1234.5')).toBe(true);
  });

  it("gives Copy CSV nothing to copy when no row is shown", () => {
    measurementLog.clear();
    expect(typeof copyButton()?.text).toBe("function");
    expect(copyButton()?.disabled).toBe(true);
    expect((copyButton()!.text as () => string | null)()).toBeNull();
  });

  it("shows a refused clipboard write in the alert of the panel and clears it at the next copy", () => {
    expect(typeof copyButton()?.onError).toBe("function");
    (copyButton()!.onError as (message: string) => void)("Clipboard copy failed: Document is not focused.");
    expect(panelError()).toBe("Clipboard copy failed: Document is not focused.");
    (copyButton()!.text as () => string | null)();
    expect(panelError()).toBeNull();
  });

  it("saves the shown rows with the provenance of the export", async () => {
    click("Save CSV");
    await vi.waitFor(() => expect(backend.saved).toHaveLength(1));
    expect(backend.texts[0].startsWith(`# software: AstroBurst ${APP_VERSION}\r\n# exported_utc: `)).toBe(true);
    expect(backend.texts[0]).toContain("\r\n# rows: 1\r\n");
    expect(backend.texts[0]).toContain("\r\ntimestamp_utc,kind,file,image,dq,unit,source,notes,p_r_ap,v_net_flux\r\n");
  });

  it("announces the saved file", async () => {
    await savedOnce();
    expect(notice()?.role).toBe("status");
  });

  it("opens the clear confirmation with the focus on Confirm and announces the question", () => {
    click("Clear (1)");
    expect(button("Confirm")?.autoFocus).toBe(true);
    const alert = elements(show()).find((p) => p.role === "alert");
    expect(textOf(alert?.children as ReactNode)).toBe("Clear 1 row?");
    expect(group()?.["aria-label"]).toBe("Confirm clearing the measurement log");
  });

  it("keeps the confirmation open while the focus is inside it and closes it 5 s after the focus leaves", async () => {
    click("Clear (1)");
    (group()?.onFocus as (() => void) | undefined)?.();
    show();
    await vi.advanceTimersByTimeAsync(6_000);
    expect(button("Confirm"), "the focus is inside the confirmation").toBeDefined();
    (group()!.onBlur as (e: unknown) => void)({ currentTarget: { contains: () => true }, relatedTarget: {} });
    show();
    await vi.advanceTimersByTimeAsync(6_000);
    expect(button("Confirm"), "the focus moved between Confirm and Cancel").toBeDefined();
    (group()!.onBlur as (e: unknown) => void)({ currentTarget: { contains: () => false }, relatedTarget: null });
    show();
    await vi.advanceTimersByTimeAsync(4_000);
    expect(button("Confirm"), "4 s after the focus left").toBeDefined();
    await vi.advanceTimersByTimeAsync(1_500);
    expect(button("Confirm")).toBeUndefined();
    expect(button("Clear (1)")).toBeDefined();
  });

  it("gives the focus back to Clear on Cancel and moves it to the Kind filter after the log is cleared", () => {
    const clearFocus = vi.fn();
    const kindFocus = vi.fn();
    const clearRef = button("Clear (1)")?.ref as { current: unknown } | undefined;
    const kindRef = kindSelect()?.ref as { current: unknown } | undefined;
    expect(clearRef).toBeDefined();
    expect(kindRef).toBeDefined();
    clearRef!.current = { focus: clearFocus };
    kindRef!.current = { focus: kindFocus };
    click("Clear (1)");
    click("Cancel");
    show();
    expect(clearFocus).toHaveBeenCalledTimes(1);
    expect(kindFocus).not.toHaveBeenCalled();
    click("Clear (1)");
    click("Confirm");
    show();
    expect(kindFocus).toHaveBeenCalledTimes(1);
    expect(clearFocus).toHaveBeenCalledTimes(1);
  });

  it("moves no focus when the confirmation times out", async () => {
    const clearFocus = vi.fn();
    const kindFocus = vi.fn();
    const clearRef = button("Clear (1)")?.ref as { current: unknown } | undefined;
    const kindRef = kindSelect()?.ref as { current: unknown } | undefined;
    expect(clearRef).toBeDefined();
    expect(kindRef).toBeDefined();
    clearRef!.current = { focus: clearFocus };
    kindRef!.current = { focus: kindFocus };
    click("Clear (1)");
    show();
    await vi.advanceTimersByTimeAsync(6_000);
    show();
    expect(button("Clear (1)")).toBeDefined();
    expect(clearFocus).not.toHaveBeenCalled();
    expect(kindFocus).not.toHaveBeenCalled();
  });

  it("uses the shared small button class", () => {
    const source = readFileSync(fileURLToPath(new URL("../MeasurementLogPanel.tsx", import.meta.url)), "utf8");
    expect(source).not.toContain("const SMALL_BUTTON_CLASS =");
  });
});
