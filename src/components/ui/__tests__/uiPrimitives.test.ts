import { describe, expect, it, vi } from "vitest";
import { createElement, type ComponentProps, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ErrorAlert from "../ErrorAlert";
import SectionHeader from "../SectionHeader";
import RunButton from "../RunButton";
import Toggle from "../Toggle";
import { SMALL_BUTTON_CLASS, ERROR_ALERT_COMPACT_CLASS } from "../uiClasses";

type RunButtonProps = ComponentProps<typeof RunButton>;
type ToggleProps = ComponentProps<typeof Toggle>;

const ICON = createElement("i", { "data-icon": "" });

function runButtonTag(props: Partial<RunButtonProps>): string {
  const markup = renderToStaticMarkup(createElement(RunButton, { label: "Run deconvolution", runningLabel: "Deconvolving", running: false, onClick: () => {}, ...props }));
  const tag = /<button[^>]*>/.exec(markup)?.[0];
  if (!tag) throw new Error("no run button rendered");
  return tag;
}

function clickRunButton(props: Partial<RunButtonProps>): ReturnType<typeof vi.fn> {
  const onClick = vi.fn();
  const view = (RunButton as unknown as { type: (p: RunButtonProps) => ReactElement<{ onClick?: () => void }> }).type;
  const button = view({ label: "Run deconvolution", running: false, ...props, onClick });
  button.props.onClick?.();
  return onClick;
}

describe("ErrorAlert", () => {
  it("announces a run failure as an alert", () => {
    const markup = renderToStaticMarkup(createElement(ErrorAlert, { message: "Deconvolution failed: out of memory" }));
    expect(markup).toMatch(/^<div class="ab-error-alert" role="alert">Deconvolution failed: out of memory<\/div>$/);
  });

  it("renders nothing without a message", () => {
    expect(renderToStaticMarkup(createElement(ErrorAlert, { message: null }))).toBe("");
    expect(renderToStaticMarkup(createElement(ErrorAlert, { message: "" }))).toBe("");
  });

  it("keeps one compact look for panel failures and still announces them", () => {
    const markup = renderToStaticMarkup(createElement(ErrorAlert, { message: "Star detection failed: no stars above 5 sigma", compact: true }));
    expect(markup).toBe('<div class="ab-error-alert ab-error-alert-compact" role="alert">Star detection failed: no stars above 5 sigma</div>');
  });

  it("appends the layout classes a panel passes", () => {
    const markup = renderToStaticMarkup(createElement(ErrorAlert, { message: "Cube collapse failed: no channels", compact: true, className: "w-full" }));
    expect(markup).toBe('<div class="ab-error-alert ab-error-alert-compact w-full" role="alert">Cube collapse failed: no channels</div>');
  });
});

describe("shared panel classes", () => {
  it("exports the 10 px small button class of the science panels", () => {
    expect(SMALL_BUTTON_CLASS).toBe(
      "flex items-center gap-1 px-2 py-1 rounded text-[10px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40 disabled:cursor-not-allowed",
    );
  });

  it("names the compact alert classes for elements that keep their own markup", () => {
    expect(ERROR_ALERT_COMPACT_CLASS).toBe("ab-error-alert ab-error-alert-compact");
  });
});

describe("Toggle", () => {
  function toggleMarkup(props: Partial<ToggleProps>): string {
    return renderToStaticMarkup(createElement(Toggle, { label: "EE fraction", checked: false, onChange: () => {}, ...props }));
  }

  function rowTag(markup: string): string {
    const tag = /^<div[^>]*>/.exec(markup)?.[0];
    if (!tag) throw new Error("no toggle row rendered");
    return tag;
  }

  it("dims the label of a disabled toggle and says why on the row", () => {
    const markup = toggleMarkup({ disabled: true, disabledReason: "needs a growth-curve plateau" });
    const row = rowTag(markup);
    expect(row).toMatch(/\saria-disabled="true"/);
    expect(row).toMatch(/\stitle="needs a growth-curve plateau"/);
    expect(markup).toContain('<span class="text-xs text-zinc-400 opacity-50">EE fraction</span>');
  });

  it("leaves an enabled toggle as it is", () => {
    const markup = toggleMarkup({ disabled: false });
    const row = rowTag(markup);
    expect(row).not.toContain("aria-disabled");
    expect(row).not.toContain("title=");
    expect(markup).toContain('<span class="text-xs text-zinc-400 cursor-pointer">EE fraction</span>');
  });
});

describe("SectionHeader", () => {
  it("titles the section with an h3 that keeps the header classes", () => {
    const markup = renderToStaticMarkup(createElement(SectionHeader, { icon: ICON, title: "Background Extraction" }));
    expect(markup).toBe('<div class="flex items-center gap-2 mb-1"><i data-icon=""></i><h3 class="text-sm font-semibold text-zinc-200 tracking-wide">Background Extraction</h3></div>');
  });

  it("keeps the subtitle of the existing call sites unchanged", () => {
    const markup = renderToStaticMarkup(createElement(SectionHeader, { icon: ICON, title: "Richardson-Lucy Deconvolution", subtitle: "FFT-accelerated" }));
    expect(markup).toContain('</h3><span class="text-[10px] text-zinc-500 ml-auto">FFT-accelerated</span></div>');
  });

  it("truncates a file name subtitle with the full name as its tooltip and keeps the actions after it", () => {
    const name = "jw02739-o001_t001_nircam_clear-f444w_i2d.fits";
    const markup = renderToStaticMarkup(
      createElement(SectionHeader, {
        icon: ICON,
        title: "History",
        subtitle: name,
        truncateSubtitle: true,
        actions: createElement("button", { type: "button" }, "Undo"),
      } as ComponentProps<typeof SectionHeader>),
    );
    expect(markup).toContain(`<h3 class="text-sm font-semibold text-zinc-200 tracking-wide">History</h3><span class="text-[10px] text-zinc-500 ml-auto truncate" title="${name}">${name}</span>`);
    expect(markup).toMatch(/<\/span><span class="flex items-center gap-1 shrink-0"><button type="button">Undo<\/button><\/span><\/div>$/);
  });

  it("pushes the actions to the right edge when there is no subtitle", () => {
    const markup = renderToStaticMarkup(
      createElement(SectionHeader, { icon: ICON, title: "History", subtitle: "", truncateSubtitle: true, actions: createElement("button", { type: "button" }, "Undo") } as ComponentProps<typeof SectionHeader>),
    );
    expect(markup).not.toContain("text-zinc-500");
    expect(markup).toContain('</h3><span class="flex items-center gap-1 shrink-0 ml-auto"><button type="button">Undo</button></span>');
  });
});

describe("RunButton", () => {
  it("stays focusable while running: busy and aria-disabled without native disabled, and ignores clicks", () => {
    const tag = runButtonTag({ running: true });
    expect(tag).toContain('aria-busy="true"');
    expect(tag).toContain('aria-disabled="true"');
    expect(tag).not.toMatch(/\sdisabled=""/);
    expect(tag).toMatch(/class="ab-run-btn[^"]*"[^>]*data-running="true"[^>]*data-disabled="false"/);
    expect(clickRunButton({ running: true })).not.toHaveBeenCalled();
  });

  it("stays focusable while running even when its caller also disables it", () => {
    const tag = runButtonTag({ running: true, disabled: true });
    expect(tag).not.toMatch(/\sdisabled=""/);
    expect(tag).toContain('aria-disabled="true"');
    expect(tag).toMatch(/class="ab-run-btn[^"]*"[^>]*data-running="true"[^>]*data-disabled="true"/);
    expect(clickRunButton({ running: true, disabled: true })).not.toHaveBeenCalled();
  });

  it("disables natively through the disabled prop while idle and runs on click when enabled", () => {
    const disabled = runButtonTag({ disabled: true });
    expect(disabled).toMatch(/\sdisabled=""/);
    expect(disabled).not.toContain("aria-busy");
    expect(disabled).not.toContain("aria-disabled");
    expect(disabled).toMatch(/class="ab-run-btn[^"]*"[^>]*data-running="false"[^>]*data-disabled="true"/);
    const idle = runButtonTag({});
    expect(idle).not.toMatch(/\sdisabled=""/);
    expect(idle).not.toContain("aria-busy");
    expect(clickRunButton({})).toHaveBeenCalledTimes(1);
  });
});
