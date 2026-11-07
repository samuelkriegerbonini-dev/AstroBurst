import { describe, expect, it, vi } from "vitest";
import { createElement, type ComponentProps, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ErrorAlert from "../ErrorAlert";
import SectionHeader from "../SectionHeader";
import RunButton from "../RunButton";

type RunButtonProps = ComponentProps<typeof RunButton>;

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
