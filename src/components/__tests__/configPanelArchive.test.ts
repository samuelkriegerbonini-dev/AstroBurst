import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../services/config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/config")>()),
  getConfig: async () => ({ astrometry_api_url: "https://nova.astrometry.net", plate_solve_timeout_secs: 120, data_dir_override: null, mast_quota_gb: null }),
  getApiKey: async () => ({ key: null }),
  getOutputDirInfo: async () => ({ output_dir: "C:/out", total_size: 0, file_count: 0 }),
}));
vi.mock("../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../infrastructure/tauri")>()),
  getOutputDir: async () => "C:/out",
  getHistoryDir: async () => "C:/history",
  onCommandOutputs: () => () => {},
}));
vi.mock("../../context/PreviewContext", () => ({ listRenderRecords: () => [], useRenderActions: () => ({ forgetOutputs: () => {} }) }));
vi.mock("../../context/CompositeContext", () => ({ useCompositePreview: () => ({ compositePreviewUrl: null }) }));
vi.mock("../../context/ComposeWizardContext", () => ({ useComposeWizardContext: () => ({ state: {} }) }));

import { hooks } from "../../test/hookRuntime";
import ConfigPanel, { ArchiveDownloadsFields, QUOTA_POSITIVE_ERROR, type ArchiveDownloadsFieldsProps } from "../ConfigPanel";
import CatalogsSettings from "../CatalogsSettings";

type AnyProps = Record<string, unknown> & { children?: ReactNode };

const DATA_ID = "data-folder";
const QUOTA_ID = "mast-quota";

function fieldProps(over: Partial<ArchiveDownloadsFieldsProps> = {}): ArchiveDownloadsFieldsProps {
  return {
    dataDirId: DATA_ID,
    quotaId: QUOTA_ID,
    platform: "Win32",
    dataDirDraft: "",
    quotaDraft: "",
    savedDataDir: null,
    savedQuota: null,
    onDataDirDraftChange: vi.fn(),
    onQuotaDraftChange: vi.fn(),
    onCommit: vi.fn(),
    onError: vi.fn(),
    ...over,
  };
}

function findElement(node: ReactNode, match: (props: AnyProps) => boolean): ReactElement<AnyProps> {
  const stack: ReactNode[] = [node];
  while (stack.length > 0) {
    const current = stack.pop();
    if (Array.isArray(current)) {
      stack.push(...current);
      continue;
    }
    if (!isValidElement<AnyProps>(current)) continue;
    if (match(current.props)) return current;
    stack.push(current.props.children);
  }
  throw new Error("element not found");
}

function input(props: ArchiveDownloadsFieldsProps, id: string): AnyProps {
  return findElement(ArchiveDownloadsFields(props), (p) => p.id === id).props;
}

function pressEnter(props: ArchiveDownloadsFieldsProps, id: string) {
  const blur = vi.fn();
  (input(props, id).onKeyDown as (e: unknown) => void)({ key: "Enter", currentTarget: { blur } });
  return blur;
}

function blur(props: ArchiveDownloadsFieldsProps, id: string): void {
  (input(props, id).onBlur as () => void)();
}

function elementsInOrder(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(elementsInOrder);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...elementsInOrder(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<AnyProps>(node) ? textOf(node.props.children) : "";
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

async function renderedConfigPanel(): Promise<ReactNode> {
  hooks.mount();
  hooks.render(ConfigPanel, {});
  await flush();
  return hooks.render(ConfigPanel, {}) as ReactNode;
}

describe("Settings panel", () => {
  afterEach(() => hooks.mount());

  it("lists the catalogues right after the Plate Solving section and before the archive downloads", async () => {
    const order = elementsInOrder(await renderedConfigPanel());
    const plateSolving = order.findIndex((el) => el.type === "h4" && textOf(el).includes("Plate Solving"));
    const catalogs = order.findIndex((el) => el.type === CatalogsSettings);
    const archive = order.findIndex((el) => el.type === ArchiveDownloadsFields);
    expect(plateSolving).toBeGreaterThan(0);
    expect(catalogs).toBeGreaterThan(plateSolving);
    expect(archive).toBeGreaterThan(catalogs);
  });

  it("says the astrometry.net key is only for the optional fallback", async () => {
    const text = textOf(await renderedConfigPanel()).replace(/\s+/g, " ");
    expect(text).not.toContain("Required for plate solving");
    expect(text).toContain("Only for the optional astrometry.net fallback after an offline solve fails. Get one at nova.astrometry.net");
  });
});

describe("Archive Downloads settings", () => {
  it("names the real default data folder for each platform instead of the app's own data folder", () => {
    const placeholder = (platform: string) =>
      input(fieldProps({ platform }), DATA_ID).placeholder;
    expect(placeholder("Win32")).toBe("Default: %LOCALAPPDATA%\\astroburst");
    expect(placeholder("MacIntel")).toBe("Default: ~/Library/Application Support/astroburst");
    expect(placeholder("Linux x86_64")).toBe("Default: ~/.local/share/astroburst");
    const markup = renderToStaticMarkup(createElement(ArchiveDownloadsFields, fieldProps()));
    expect(markup).toContain('placeholder="Default: %LOCALAPPDATA%\\astroburst"');
    expect(markup).not.toContain("local data folder");
  });

  it("commits the data folder trimmed on Enter, an empty one as null, and nothing when it is unchanged", () => {
    const cleared = fieldProps({ dataDirDraft: "   ", savedDataDir: "D:/old" });
    pressEnter(cleared, DATA_ID);
    expect(cleared.onCommit).toHaveBeenCalledWith("data_dir_override", null);

    const typed = fieldProps({ dataDirDraft: "  D:/x ", savedDataDir: null });
    pressEnter(typed, DATA_ID);
    expect(typed.onCommit).toHaveBeenCalledWith("data_dir_override", "D:/x");

    const unchanged = fieldProps({ dataDirDraft: "D:/x ", savedDataDir: "D:/x" });
    pressEnter(unchanged, DATA_ID);
    expect(unchanged.onCommit).not.toHaveBeenCalled();
  });

  it("does not create a folder for a half-typed path when the data folder loses focus", () => {
    const halfTyped = fieldProps({ dataDirDraft: "D:\\Astor", savedDataDir: null });
    expect(input(halfTyped, DATA_ID).onBlur).toBeUndefined();
    (input(halfTyped, DATA_ID).onKeyDown as (e: unknown) => void)({ key: "a", currentTarget: { blur: vi.fn() } });
    expect(halfTyped.onCommit).not.toHaveBeenCalled();
    const markup = renderToStaticMarkup(createElement(ArchiveDownloadsFields, halfTyped));
    expect(markup).toContain("Press Enter to apply");
    const saved = renderToStaticMarkup(createElement(ArchiveDownloadsFields, fieldProps({ dataDirDraft: "D:/x", savedDataDir: "D:/x" })));
    expect(saved).not.toContain("Press Enter to apply");
  });

  it("sends an empty quota as null and a positive one as a number", () => {
    const empty = fieldProps({ quotaDraft: "", savedQuota: 5 });
    blur(empty, QUOTA_ID);
    expect(empty.onCommit).toHaveBeenCalledWith("mast_quota_gb", null);

    const half = fieldProps({ quotaDraft: " 0.5 ", savedQuota: null });
    blur(half, QUOTA_ID);
    expect(half.onCommit).toHaveBeenCalledWith("mast_quota_gb", 0.5);

    const same = fieldProps({ quotaDraft: "5", savedQuota: 5 });
    blur(same, QUOTA_ID);
    expect(same.onCommit).not.toHaveBeenCalled();
  });

  it("refuses a quota of zero or less locally, naming the setting, without calling Rust", () => {
    expect(QUOTA_POSITIVE_ERROR).toBe("MAST download quota: Use a value greater than 0");
    for (const draft of ["0", "-5", "0.0"]) {
      const props = fieldProps({ quotaDraft: draft, savedQuota: null });
      blur(props, QUOTA_ID);
      expect(props.onCommit, draft).not.toHaveBeenCalled();
      expect(props.onError, draft).toHaveBeenCalledWith(QUOTA_POSITIVE_ERROR);
    }
    const comma = fieldProps({ quotaDraft: "2,5" });
    blur(comma, QUOTA_ID);
    expect(comma.onError).toHaveBeenCalledWith("MAST download quota: Use a dot (.) as the decimal separator");
    expect(comma.onCommit).not.toHaveBeenCalled();
  });

  it("commits the quota on Enter by leaving the field", () => {
    const props = fieldProps({ quotaDraft: "12", savedQuota: null });
    const left = pressEnter(props, QUOTA_ID);
    expect(left).toHaveBeenCalledTimes(1);
    blur(props, QUOTA_ID);
    expect(props.onCommit).toHaveBeenCalledWith("mast_quota_gb", 12);
  });

  it("hands typed text to the draft setters", () => {
    const props = fieldProps();
    (input(props, DATA_ID).onChange as (e: unknown) => void)({ target: { value: "E:/astro" } });
    (input(props, QUOTA_ID).onChange as (e: unknown) => void)({ target: { value: "7" } });
    expect(props.onDataDirDraftChange).toHaveBeenCalledWith("E:/astro");
    expect(props.onQuotaDraftChange).toHaveBeenCalledWith("7");
  });
});
