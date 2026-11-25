import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement, ReactNode } from "react";
import type { ContourResult } from "../../../shared/types/contours";

const backend = vi.hoisted(() => ({ contours: vi.fn() }));
const dq = vi.hoisted(() => ({ excludeDq: false }));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../services/contours", () => ({ contourLines: backend.contours }));
vi.mock("../../../context/PreviewContext", () => ({ useDqContext: () => dq }));
vi.mock("../../../hooks/useAnalysisTarget", () => ({ useMeasurementSource: () => null }));
vi.mock("../MeasurementBadge", () => ({ default: () => null }));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, findElement } from "../../../test/elementTree";
import { DecimalField, RunButton } from "../../ui";
import ContourPanel from "../ContourPanel";
import StaleResultNote from "../StaleResultNote";

type Props = Record<string, unknown>;

const RESULT: ContourResult = { levels: [], bin: 1, n_points: 0, background_median: 1, background_sigma: 0.5, notes: [], masked: false, elapsed_ms: 2 };

const PROPS = { filePath: "C:/data/a.fits", overlayKey: null, imageWidth: 64, imageHeight: 64, measureKey: "C:/data/a.fits" };
const panel = (ContourPanel as unknown as { type: (props: Props) => ReactNode }).type;
const show = () => hooks.render(panel, PROPS) as ReactNode;
const isStale = (tree: ReactNode) => elementsOf(tree).some((el) => el.type === StaleResultNote);

async function computed(): Promise<void> {
  const run = findElement<Props>(show(), (el) => el.type === RunButton) as ReactElement<Props>;
  (run.props.onClick as () => void)();
  await vi.waitFor(() => expect(backend.contours).toHaveBeenCalled());
  await vi.waitFor(() => expect(elementsOf<Props>(show()).some((el) => el.props["data-testid"] === "contour-background")).toBe(true));
}

describe("ContourPanel result validity", () => {
  beforeEach(() => {
    hooks.mount();
    dq.excludeDq = false;
    backend.contours.mockReset().mockResolvedValue(RESULT);
  });

  afterEach(() => hooks.mount());

  it("marks the contours as stale after DQ exclusion or the smoothing changes", async () => {
    await computed();
    expect(isStale(show())).toBe(false);
    dq.excludeDq = true;
    expect(isStale(show())).toBe(true);
    dq.excludeDq = false;
    expect(isStale(show())).toBe(false);
    const tree = show();
    const label = findElement<Props>(tree, (el) => el.type === "label" && String(el.props.children).trim() === "Smoothing sigma (px)");
    const smooth = findElement<Props>(tree, (el) => el.type === DecimalField && el.props.id === label?.props.htmlFor);
    (smooth!.props.onChange as (text: string) => void)("2");
    expect(isStale(show())).toBe(true);
  });
});
