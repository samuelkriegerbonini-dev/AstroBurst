import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import { ClipboardCopy } from "lucide-react";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, findElement, textOf } from "../../../test/elementTree";
import CopyCsvButton, { type CopyCsvButtonProps } from "../CopyCsvButton";
import { SMALL_BUTTON_CLASS } from "../uiClasses";

type ButtonProps = { className?: string; title?: string; disabled?: boolean; onClick?: () => void; children?: unknown };

const CopyCsvButtonInner = (CopyCsvButton as unknown as { type: (props: CopyCsvButtonProps) => unknown }).type;

function render(props: CopyCsvButtonProps): { button: ReactElement<ButtonProps>; status: ReactElement<{ children?: unknown }> } {
  const tree = hooks.render(CopyCsvButtonInner, props);
  const button = findElement<ButtonProps>(tree, (e) => e.type === "button");
  const status = findElement<{ role?: string; children?: unknown }>(tree, (e) => e.props.role === "status");
  if (!button || !status) throw new Error("copy button or status not rendered");
  return { button, status };
}

let writeText: ReturnType<typeof vi.fn>;

beforeEach(() => {
  hooks.mount();
  writeText = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});

afterEach(() => {
  hooks.mount();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("CopyCsvButton", () => {
  it("copies the CSV built at click time and says copied for 1.5 s", async () => {
    const props: CopyCsvButtonProps = { text: () => "a,b\r\n1,2\r\n" };
    render(props).button.props.onClick?.();
    await vi.advanceTimersByTimeAsync(0);
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith("a,b\r\n1,2\r\n");
    const copied = render(props);
    expect(textOf(copied.button)).toBe("copied");
    expect(textOf(copied.status)).toBe("Copied");
    await vi.advanceTimersByTimeAsync(1500);
    const settled = render(props);
    expect(textOf(settled.button)).toBe("Copy CSV");
    expect(textOf(settled.status)).toBe("");
  });

  it("does nothing when there is nothing to copy", async () => {
    const props: CopyCsvButtonProps = { text: () => null };
    render(props).button.props.onClick?.();
    await vi.advanceTimersByTimeAsync(0);
    expect(writeText).not.toHaveBeenCalled();
    expect(textOf(render(props).button)).toBe("Copy CSV");
  });

  it("names a refused clipboard write and keeps the button label", async () => {
    writeText.mockRejectedValue(new DOMException("Document is not focused.", "NotAllowedError"));
    const onError = vi.fn();
    const props: CopyCsvButtonProps = { text: () => "a\r\n", onError };
    render(props).button.props.onClick?.();
    await vi.advanceTimersByTimeAsync(0);
    expect(onError).toHaveBeenCalledWith("Clipboard copy failed: Document is not focused.");
    const after = render(props);
    expect(textOf(after.button)).toBe("Copy CSV");
    expect(textOf(after.status)).toBe("");
  });

  it("keeps the caller's class, title, label, icon and disabled state", () => {
    const custom = render({ text: () => "a\r\n", className: "x", title: "Copy the shown rows as CSV", label: "Copy rows", disabled: true }).button;
    expect(custom.props.className).toBe("x");
    expect(custom.props.title).toBe("Copy the shown rows as CSV");
    expect(custom.props.disabled).toBe(true);
    expect(textOf(custom)).toBe("Copy rows");
    const plain = render({ text: () => "a\r\n" }).button;
    expect(plain.props.className).toBe(SMALL_BUTTON_CLASS);
    const icon = elementsOf<{ size?: number }>(plain.props.children)[0];
    expect(icon?.type).toBe(ClipboardCopy);
    expect(icon?.props.size).toBe(10);
  });
});
