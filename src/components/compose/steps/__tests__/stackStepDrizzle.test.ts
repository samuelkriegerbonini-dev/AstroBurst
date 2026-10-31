import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { WizardState } from "../../../../utils/wizard";
import type { StackResult } from "../../../../shared/types/stacking";

const wizard = vi.hoisted(() => ({ state: null as WizardState | null }));

vi.mock("react", async (importOriginal) => (await import("../../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../../context/ComposeWizardContext", () => ({
  useComposeWizardContext: () => ({
    stackRun: null,
    setStackRun: vi.fn(),
    finishStackRun: vi.fn(),
    getStackGeneration: () => 0,
    getState: () => wizard.state,
    stackDiscarded: {},
    recordStackOutcome: vi.fn(),
  }),
}));
vi.mock("../../../../services/stacking", () => ({ stackFrames: vi.fn(), drizzleFrames: vi.fn() }));
vi.mock("../../../../services/analysis", () => ({ analyzeSubframes: vi.fn() }));
vi.mock("../../../../services/progress", () => ({ cancelProgress: vi.fn() }));
vi.mock("../../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));
vi.mock("../../../../hooks/useProgress", () => ({
  useProgress: () => ({ active: false, percent: 0, stage: "", current: 0, total: 0, reset: () => {} }),
}));
vi.mock("../../../../hooks/useFileStore", () => ({ resolveEffectivePath: (path: string) => path }));

import { hooks } from "../../../../test/hookRuntime";
import { drizzleFrames } from "../../../../services/stacking";
import { getOutputDir } from "../../../../infrastructure/tauri";
import { claimStackProgress, releaseStackProgress, stackProgressHolder } from "../../../../utils/stackProgressLock";
import { INITIAL_STATE } from "../../../../utils/wizard";
import StackStep from "../StackStep";

type AnyProps = Record<string, unknown> & { children?: ReactNode };
type StepProps = { state: WizardState; dispatch: () => void; onStacked: () => void };
const step = StackStep as unknown as (props: StepProps) => ReactNode;

const R = ["/sx13/r_0001.fits", "/sx13/r_0002.fits", "/sx13/r_0003.fits"];

function nodes(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...nodes(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<AnyProps>(node) ? textOf(node.props.children) : "";
}

const labelled = (tree: ReactNode, label: string) => nodes(tree).find((el) => el.props.label === label);
const byTestId = (tree: ReactNode, id: string) => nodes(tree).find((el) => el.props["data-testid"] === id);
const stackButton = (tree: ReactNode) => nodes(tree).find((el) => el.type === "button" && textOf(el.props.children).trim() === "Stack");
const kernelSelect = (tree: ReactNode) => nodes(tree).find((el) => el.type === "select" && el.props["data-testid"] === "stack-drizzle-kernel");

function viewWithDrizzle(): () => ReactNode {
  const state: WizardState = { ...INITIAL_STATE, bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: b.id === "r" ? R : [] })) };
  wizard.state = state;
  const view = () => hooks.render(step, { state, dispatch: vi.fn(), onStacked: vi.fn() }) as ReactNode;
  (labelled(view(), "Drizzle")?.props.onChange as (on: boolean) => void)(true);
  return view;
}

describe("compose Stack step drizzle", () => {
  beforeEach(() => {
    hooks.mount();
    vi.mocked(drizzleFrames).mockReset();
    vi.mocked(getOutputDir).mockReset().mockResolvedValue("C:/out");
  });

  afterEach(() => {
    for (const run of ["stack", "drizzle", "compose"] as const) releaseStackProgress(run);
    hooks.mount();
  });

  it("shows pixfrac and kernel next to a 1-4x scale and sends all three", async () => {
    let finish: (result: StackResult) => void = () => {};
    vi.mocked(drizzleFrames).mockReturnValue(new Promise<StackResult>((resolve) => { finish = resolve; }));
    const view = viewWithDrizzle();
    const scale = labelled(view(), "Scale");
    const pixfrac = labelled(view(), "Pixfrac");
    expect([scale?.props.min, scale?.props.max, scale?.props.value]).toEqual([1, 4, 2]);
    expect([pixfrac?.props.min, pixfrac?.props.max, pixfrac?.props.value]).toEqual([0.1, 1, 0.7]);
    expect(kernelSelect(view())?.props.value).toBe("square");
    (scale?.props.onChange as (v: number) => void)(3.5);
    (labelled(view(), "Pixfrac")?.props.onChange as (v: number) => void)(0.5);
    (kernelSelect(view())?.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "lanczos3" } });
    (stackButton(view())?.props.onClick as () => void)();
    await vi.waitFor(() => expect(vi.mocked(drizzleFrames)).toHaveBeenCalledTimes(1));
    const [paths, outputDir, options] = vi.mocked(drizzleFrames).mock.calls[0];
    expect(paths).toEqual(R);
    expect(outputDir).toBe("C:/out");
    expect(options).toMatchObject({ scale: 3.5, pixfrac: 0.5, kernel: "lanczos3", align: true });
    expect(stackProgressHolder()).toBe("compose");
    finish({ png_path: "C:/out/drizzle_r.png", fits_path: "C:/out/drizzle_r.fits", dimensions: [8, 8], elapsed_ms: 5, frame_count: 3 });
    await vi.waitFor(() => expect(stackProgressHolder()).toBeNull());
  });

  it("waits while Stacking › Drizzle RGB (mono) runs", () => {
    claimStackProgress("drizzle");
    const tree = viewWithDrizzle()();
    const line = byTestId(tree, "compose-stack-progress-busy");
    expect(labelled(tree, "Stack All")?.props.disabled).toBe(true);
    expect(labelled(tree, "Stack All")?.props.describedBy).toBe(line?.props.id);
    expect(stackButton(tree)?.props.disabled).toBe(true);
    expect(line?.props.children).toBe("Stacking › Drizzle RGB (mono) is running; wait until it ends, as both use the same progress bar and Cancel.");
  });

  it("waits for a Compose stack it is no longer showing, such as one still ending after a Reset", () => {
    claimStackProgress("compose");
    const tree = viewWithDrizzle()();
    expect(labelled(tree, "Stack All")?.props.disabled).toBe(true);
    expect(stackButton(tree)?.props.disabled).toBe(true);
    expect(byTestId(tree, "compose-stack-progress-busy")?.props.children).toBe("Compose › Stack is running; wait until it ends, as both use the same progress bar and Cancel.");
  });
});
