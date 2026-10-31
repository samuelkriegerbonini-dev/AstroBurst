import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ProcessedFile } from "../../../shared/types";
import type { PipelineMasterOutput, PipelineRequest, PipelineResult, PipelineStats } from "../../../shared/types/stacking";
import type { CalibrationState } from "../StackingTab";

const { captured, runPipelineMock, ingest, progress } = vi.hoisted(() => ({
  captured: { onRun: null as null | (() => void) },
  runPipelineMock: vi.fn(),
  ingest: { showWhenLoaded: vi.fn(), ingestUnlessLoaded: vi.fn() },
  progress: { events: new Set<string>(), state: { current: 0, total: 0, percent: 0, stage: "", active: false } },
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../../hooks/useFileIngest", () => ingest);
vi.mock("../../../hooks/useProgress", () => ({
  useProgress: (event: string) => {
    progress.events.add(event);
    return { ...progress.state, reset: () => {} };
  },
}));
vi.mock("../../../services/progress", () => ({ cancelProgress: vi.fn() }));
vi.mock("../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui")>();
  return {
    ...actual,
    RunButton: (props: { onClick: () => void }) => {
      captured.onRun = props.onClick;
      return null;
    },
  };
});
vi.mock("../../../services/stacking", () => ({ runCalibrationPipeline: runPipelineMock }));
vi.mock("../../../infrastructure/tauri", () => ({ getOutputDir: vi.fn() }));

import { open } from "@tauri-apps/plugin-dialog";
import { hooks } from "../../../test/hookRuntime";
import { cancelProgress } from "../../../services/progress";
import { ErrorAlert } from "../../ui";
import { DEFAULT_STACK_SETTINGS } from "../../../utils/stackingRejection";
import PipelinePanel, { COSMETIC_AUTO_NOTE, CosmeticAutoNote, PipelineMasterLines } from "../PipelinePanel";

const RED_FILTER: Record<string, string> = { FILTER: "Red" };

function light(name: string): ProcessedFile {
  return {
    id: name,
    name,
    path: `C:/r6/a1/calibset/${name}`,
    sourcePath: `C:/r6/a1/calibset/${name}`,
    imageRef: null,
    size: 0,
    status: "done",
    result: { header: RED_FILTER } as ProcessedFile["result"],
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function sessionFrame(name: string, header: Record<string, string>): ProcessedFile {
  return { ...light(name), result: { header } as ProcessedFile["result"] };
}

const LIGHTS = ["light_120s.fits", "light_120s_2.fits"].map(light);
const CALIBRATION: CalibrationState = {
  calibratedPath: null,
  calibratedFitsPath: null,
  hasBias: true,
  hasDark: true,
  hasFlat: false,
  darkPaths: ["C:/r6/a1/calibset/dark_300s_1.fits"],
  flatPaths: [],
  biasPaths: ["C:/r6/a1/calibset/bias_1.fits"],
  flatDarkPaths: [],
};

function stats(extra: Partial<PipelineStats>): PipelineStats {
  return { darks_combined: 4, flats_combined: 4, bias_combined: 4, channels: [], ...extra };
}

function linesOf(s: PipelineStats): string[] {
  const html = renderToStaticMarkup(createElement(PipelineMasterLines, { stats: s }));
  return [...html.matchAll(/<div>([^<]*)<\/div>/g)].map((m) => m[1]);
}

describe("PipelinePanel flat-dark row and request", () => {
  beforeEach(() => {
    captured.onRun = null;
    runPipelineMock.mockReset();
    runPipelineMock.mockResolvedValue({ stats: stats({}), channel_previews: [] });
  });

  it("shows an empty Flat darks row", () => {
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files: LIGHTS, calibration: CALIBRATION }));
    expect(html).toMatch(/<div[^>]*data-testid="pipeline-flatdark-row"[^>]*><span[^>]*>Flat darks: 0<\/span>/);
  });

  it("always sends flat_dark_paths and leaves cosmetic correction off by default", async () => {
    renderToStaticMarkup(createElement(PipelinePanel, { files: LIGHTS, calibration: CALIBRATION }));
    captured.onRun?.();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(1));
    const request = runPipelineMock.mock.calls[0][0] as PipelineRequest;
    expect(request.flat_dark_paths).toEqual([]);
    expect(request.dark_paths).toEqual(CALIBRATION.darkPaths);
    expect(request.bias_paths).toEqual(CALIBRATION.biasPaths);
    expect(request.channels).toEqual([{ label: "R", paths: LIGHTS.map((f) => f.path) }]);
    expect(request.cosmetic).toBeNull();
  });

  it("seeds the Flat darks row and the request with the Calibration tab flat-darks", async () => {
    const flatDarkPaths = ["C:/r6/a1/calibset/flatdark_2s_1.fits", "C:/r6/a1/calibset/flatdark_2s_2.fits"];
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files: LIGHTS, calibration: { ...CALIBRATION, flatDarkPaths } }));
    expect(html).toMatch(/<div[^>]*data-testid="pipeline-flatdark-row"[^>]*><span[^>]*>Flat darks: 2<\/span>/);
    captured.onRun?.();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(1));
    expect((runPipelineMock.mock.calls[0][0] as PipelineRequest).flat_dark_paths).toEqual(flatDarkPaths);
  });
});

describe("PipelinePanel frame types", () => {
  beforeEach(() => {
    captured.onRun = null;
    runPipelineMock.mockReset();
    runPipelineMock.mockResolvedValue({ stats: stats({}), channel_previews: [] });
  });

  async function runRequest(): Promise<PipelineRequest> {
    captured.onRun?.();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(1));
    return runPipelineMock.mock.calls[0][0] as PipelineRequest;
  }

  it("an IMAGETYP=FLAT file with FILTER=R seeds the Flats row, not the R lane", async () => {
    const lights = ["light_R_1.fits", "light_R_2.fits"].map((n) => sessionFrame(n, { IMAGETYP: "LIGHT", FILTER: "Red" }));
    const flat = sessionFrame("flat_R_1.fits", { IMAGETYP: "FLAT", FILTER: "Red" });
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files: [lights[0], flat, lights[1]] }));
    expect(html).toMatch(/>R<\/span><span[^>]*>2 files</);
    expect(html).toContain(">Flats: 1<");
    const request = await runRequest();
    expect(request.channels).toEqual([{ label: "R", paths: lights.map((f) => f.path) }]);
    expect(request.flat_paths).toEqual([flat.path]);
  });

  it("seeds the Darks, Flat darks and Bias rows from their kinds after the Calibrate tab's frames", async () => {
    const lights = ["light_R_1.fits", "light_R_2.fits"].map((n) => sessionFrame(n, { IMAGETYP: "Light Frame", FILTER: "Red" }));
    const dark = sessionFrame("dark_300s_2.fits", { IMAGETYP: "Dark Frame", FILTER: "Red" });
    const frametypDark = sessionFrame("dark_300s_3.fits", { FRAMETYP: "Dark" });
    const flatDark = sessionFrame("flatdark_2s_1.fits", { IMAGETYP: "Flat Dark", FILTER: "Red" });
    const bias = sessionFrame("bias_2.fits", { IMAGETYP: "ZERO" });
    const repeatedDark = sessionFrame("dark_300s_1.fits", { IMAGETYP: "DARK" });
    const files = [lights[0], dark, frametypDark, flatDark, bias, repeatedDark, lights[1]];
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files, calibration: CALIBRATION }));
    expect(html).toContain(">Darks: 3<");
    expect(html).toContain(">Flat darks: 1<");
    expect(html).toContain(">Bias: 2<");
    expect(html).toContain(">Flats: 0<");
    expect(html).toMatch(/>R<\/span><span[^>]*>2 files</);
    const request = await runRequest();
    expect(request.channels).toEqual([{ label: "R", paths: lights.map((f) => f.path) }]);
    expect(request.dark_paths).toEqual([...CALIBRATION.darkPaths, dark.path, frametypDark.path]);
    expect(request.flat_dark_paths).toEqual([flatDark.path]);
    expect(request.bias_paths).toEqual([...CALIBRATION.biasPaths, bias.path]);
    expect(request.flat_paths).toEqual([]);
  });

  it("keeps an Unknown frame, such as a WFPC2 IMAGETYP='EXT' exposure, in its light lane", async () => {
    const ext = sessionFrame("u2kc0101t_c0f.fits", { IMAGETYP: "EXT", FILTER: "Red" });
    renderToStaticMarkup(createElement(PipelinePanel, { files: [ext] }));
    const request = await runRequest();
    expect(request.channels).toEqual([{ label: "R", paths: [ext.path] }]);
  });
});

describe("PipelineMasterLines", () => {
  it("names the master flat-dark and the dark temperature groups", () => {
    expect(linesOf(stats({ flat_darks_combined: 4, dark_groups: [{ temp_c: -10, frames: 4 }] }))).toEqual([
      "Master dark: 4 frames",
      "Master flat: 4 frames",
      "Master flat-dark: 4 frames",
      "dark groups: -10.0 °C (4)",
    ]);
  });

  it("lists every dark group with one decimal and an ASCII minus", () => {
    const lines = linesOf(stats({ dark_groups: [{ temp_c: -20.06, frames: 12 }, { temp_c: -10, frames: 8 }, { temp_c: 0.04, frames: 5 }] }));
    expect(lines).toContain("dark groups: -20.1 °C (12), -10.0 °C (8), 0.0 °C (5)");
    expect(lines.join("\n")).not.toContain("\u2212");
  });

  it("leaves out the flat-dark line without flat-darks and the group line without temperatures", () => {
    expect(linesOf(stats({ flat_darks_combined: 0, dark_groups: [{ temp_c: null, frames: 4 }] }))).toEqual([
      "Master dark: 4 frames",
      "Master flat: 4 frames",
    ]);
    expect(linesOf(stats({}))).toEqual(["Master dark: 4 frames", "Master flat: 4 frames"]);
  });

  it("marks a group without a temperature when other groups have one", () => {
    expect(linesOf(stats({ dark_groups: [{ temp_c: -10, frames: 6 }, { temp_c: null, frames: 5 }] }))).toContain(
      "dark groups: -10.0 °C (6), unknown (5)",
    );
  });
});

describe("CosmeticAutoNote", () => {
  it("points to the Cosmetic dry run while Auto detect is on", () => {
    expect(COSMETIC_AUTO_NOTE).toBe(
      "Auto detect replaces pixels in every light without a preview; check one light first with Cosmetic > Dry run.",
    );
    const html = renderToStaticMarkup(createElement(CosmeticAutoNote, { enabled: true }));
    expect(html).toContain(COSMETIC_AUTO_NOTE.replace(">", "&gt;"));
  });

  it("says nothing while Auto detect is off", () => {
    expect(renderToStaticMarkup(createElement(CosmeticAutoNote, { enabled: false }))).toBe("");
  });
});

describe("PipelinePanel Clear buttons", () => {
  it("render as polite live regions that ask for a second press", () => {
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files: LIGHTS, calibration: CALIBRATION }));
    const clears = [...html.matchAll(/<button([^>]*)>Clear<\/button>/g)].map((m) => m[1]);
    expect(clears).toHaveLength(3);
    for (const attrs of clears) {
      expect(attrs).toContain('aria-live="polite"');
      expect(attrs).toContain('type="button"');
    }
  });
});

type AnyProps = Record<string, unknown> & { children?: ReactNode };
type PanelProps = Parameters<typeof PipelinePanel>[0];

const panel = PipelinePanel as (props: PanelProps) => ReactNode;

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

function laneRows(tree: ReactNode): ReactElement<AnyProps>[] {
  return nodes(tree).filter((el) => el.props["data-testid"] === "pipeline-lane");
}

function lanes(tree: ReactNode): string[] {
  const found: string[] = [];
  for (const el of nodes(tree)) {
    const kids = (Array.isArray(el.props.children) ? el.props.children : [el.props.children]).filter((kid): kid is ReactElement<AnyProps> => isValidElement(kid));
    kids.forEach((kid, i) => {
      const count = textOf(kid.props.children);
      if (i > 0 && kid.type === "span" && /^\d+ files/.test(count)) found.push(`${textOf(kids[i - 1].props.children)} ${count}`);
    });
  }
  return found;
}

function laneRow(tree: ReactNode, label: string): ReactElement<AnyProps> {
  const row = laneRows(tree).find((el) => nodes(el.props.children).some((s) => s.type === "span" && textOf(s.props.children) === label));
  expect(row, `lane ${label}`).toBeDefined();
  return row as ReactElement<AnyProps>;
}

function buttonNamed(tree: ReactNode, name: string): ReactElement<AnyProps> {
  const found = nodes(tree).find((el) => typeof el.props.onClick === "function" && textOf(el.props.children).trim() === name);
  expect(found, name).toBeDefined();
  return found as ReactElement<AnyProps>;
}

const press = (el: ReactElement<AnyProps>) => (el.props.onClick as () => unknown)();

function staticLanes(html: string): string[] {
  return [...html.matchAll(/<span class="text-xs text-zinc-300">([^<]*)<\/span><span[^>]*>(\d+ files[^<]*)<\/span>/g)].map((m) => `${m[1]} ${m[2]}`);
}

function leftOutText(tree: ReactNode): string | null {
  const line = nodes(tree).find((el) => el.props["data-testid"] === "pipeline-filter-left-out");
  return line ? textOf(line.props.children) : null;
}

const putSelect = (tree: ReactNode, filter: string) => nodes(tree).find((el) => el.props["aria-label"] === `Lane for the ${filter} lights`);
const putOptions = (tree: ReactNode, filter: string) => nodes(putSelect(tree, filter)?.props.children).map((o) => o.props.value);
const choosePutLane = (tree: ReactNode, filter: string, lane: string) =>
  (putSelect(tree, filter)?.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: lane } });

function putButton(tree: ReactNode, filter: string): ReactElement<AnyProps> {
  const found = nodes(tree).find((el) => typeof el.props.onClick === "function" && String(el.props["aria-label"] ?? "").startsWith(`Put the ${filter} lights in the `));
  expect(found, `Put in lane for ${filter}`).toBeDefined();
  return found as ReactElement<AnyProps>;
}

function laneAdd(tree: ReactNode, label: string): ReactElement<AnyProps> {
  const found = nodes(laneRow(tree, label).props.children).find((el) => el.props["aria-label"] === `Add ${label} lights`);
  expect(found, `+ Add of ${label}`).toBeDefined();
  return found as ReactElement<AnyProps>;
}

function filtered(name: string, filter: string | null): ProcessedFile {
  return sessionFrame(name, filter === null ? { IMAGETYP: "LIGHT" } : { IMAGETYP: "LIGHT", FILTER: filter });
}

const NARROWBAND = [
  ...[1, 2, 3, 4].map((n) => filtered(`frame_ha_${n}.fits`, "Hα")),
  ...[1, 2, 3, 4].map((n) => filtered(`frame_oiii_${n}.fits`, "OIII")),
  ...[1, 2, 3, 4].map((n) => filtered(`frame_sii_${n}.fits`, "SII")),
];

describe("PipelinePanel lanes follow the filters found", () => {
  beforeEach(() => {
    hooks.mount();
    captured.onRun = null;
    runPipelineMock.mockReset();
    runPipelineMock.mockResolvedValue({ stats: stats({}), channel_previews: [] });
    vi.mocked(open).mockReset();
  });

  afterEach(() => hooks.mount());

  it("Hα files make an Ha lane, and OIII and SII files their own lanes, with no empty R, G or B lane", async () => {
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files: NARROWBAND }));
    expect(staticLanes(html)).toEqual(["Ha 4 files", "OIII 4 files", "SII 4 files"]);
    captured.onRun?.();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(1));
    const request = runPipelineMock.mock.calls[0][0] as PipelineRequest;
    expect(request.channels.map((c) => [c.label, c.paths.length])).toEqual([["Ha", 4], ["OIII", 4], ["SII", 4]]);
  });

  it("an unfiltered light makes a Mono lane", async () => {
    const mono = filtered("frame_0001.fits", null);
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files: [mono] }));
    expect(staticLanes(html)).toEqual(["Mono 1 files"]);
    captured.onRun?.();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(1));
    expect((runPipelineMock.mock.calls[0][0] as PipelineRequest).channels).toEqual([{ label: "Mono", paths: [mono.path] }]);
  });

  it("lists the lanes in filter order, L first", () => {
    const files = [filtered("frame_ha.fits", "Ha"), filtered("frame_r.fits", "Red"), filtered("frame_l.fits", "Luminance"), filtered("frame_b.fits", "Blue")];
    expect(staticLanes(renderToStaticMarkup(createElement(PipelinePanel, { files })))).toEqual(["L 1 files", "R 1 files", "B 1 files", "Ha 1 files"]);
  });

  it("names the lights that no lane takes by their filter and keeps them out of every lane", () => {
    const files = [
      filtered("frame_r.fits", "Red"),
      filtered("frame_nii_1.fits", "NII"),
      filtered("frame_nii_2.fits", "NII"),
      filtered("light_Hb_1.fits", "Hb"),
      filtered("light_0001.fits", "3"),
    ];
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files }));
    expect(staticLanes(html)).toEqual(["R 1 files"]);
    expect(/<p[^>]*data-testid="pipeline-filter-left-out"[^>]*>([^<]*)<\/p>/.exec(html)?.[1]).toBe(
      "4 lights left out by filter: NII (2), Hb (1), wheel position 3 (1)",
    );
  });

  it("says nothing about filters when every light has a lane", () => {
    expect(renderToStaticMarkup(createElement(PipelinePanel, { files: NARROWBAND }))).not.toContain("pipeline-filter-left-out");
  });

  it("counts only the left-out lights that no lane holds, as Put in lane and Clear move them", () => {
    const nii = [filtered("frame_nii_1.fits", "NII"), filtered("frame_nii_2.fits", "NII")];
    const props: PanelProps = { files: [filtered("frame_ha.fits", "Ha"), ...nii] };
    const view = () => hooks.render(panel, props) as ReactNode;
    expect(leftOutText(view())).toBe("2 lights left out by filter: NII (2)");
    press(putButton(view(), "NII"));
    expect(lanes(view())).toEqual(["Ha 1 files", "Mono 2 files"]);
    expect(leftOutText(view())).toBeNull();
    const clear = () => nodes(laneRow(view(), "Mono").props.children).find((el) => typeof el.props.onPress === "function");
    (clear()?.props.onPress as () => void)();
    (clear()?.props.onPress as () => void)();
    expect(lanes(view())).toEqual(["Ha 1 files", "Mono 0 files"]);
    expect(leftOutText(view())).toBe("2 lights left out by filter: NII (2)");
  });

  it("puts the lights of a filter no lane takes in a lane picked by hand, offering only lanes free of other filters", async () => {
    const red = [filtered("light_r_1.fits", "Red"), filtered("light_r_2.fits", "Red")];
    const nii = [filtered("frame_nii_1.fits", "NII"), filtered("frame_nii_2.fits", "NII")];
    const [wheel1, wheel2] = [filtered("light_0001.fits", "1"), filtered("light_0002.fits", "2")];
    const props: PanelProps = { files: [...red, ...nii, wheel1, wheel2] };
    const view = () => hooks.render(panel, props) as ReactNode;
    expect(lanes(view())).toEqual(["R 2 files"]);
    expect(leftOutText(view())).toBe("4 lights left out by filter: NII (2), wheel position 1 (1), wheel position 2 (1)");
    expect(putOptions(view(), "NII")).toEqual(["L", "G", "B", "Ha", "OIII", "SII", "Mono"]);
    expect(putSelect(view(), "NII")?.props.value).toBe("Mono");
    press(putButton(view(), "NII"));
    expect(lanes(view())).toEqual(["R 2 files", "Mono 2 files"]);
    expect(leftOutText(view())).toBe("2 lights left out by filter: wheel position 1 (1), wheel position 2 (1)");
    expect(putOptions(view(), "wheel position 1")).toEqual(["L", "G", "B", "Ha", "OIII", "SII"]);
    choosePutLane(view(), "wheel position 1", "G");
    expect(putButton(view(), "wheel position 1").props["aria-label"]).toBe("Put the wheel position 1 lights in the G lane");
    press(putButton(view(), "wheel position 1"));
    expect(lanes(view())).toEqual(["R 2 files", "G 1 files", "Mono 2 files"]);
    expect(putOptions(view(), "wheel position 2")).toEqual(["L", "B", "Ha", "OIII", "SII"]);
    expect(leftOutText(view())).toBe("1 light left out by filter: wheel position 2 (1)");
    void (nodes(view()).find((el) => "runningLabel" in el.props)?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(1));
    expect((runPipelineMock.mock.calls[0][0] as PipelineRequest).channels).toEqual([
      { label: "R", paths: red.map((f) => f.path) },
      { label: "G", paths: [wheel1.path] },
      { label: "Mono", paths: nii.map((f) => f.path) },
    ]);
  });

  it("+ Add puts no light of another filter in a lane, says so, and skips lights a lane already holds", async () => {
    const ha = filtered("frame_ha.fits", "Ha");
    const nii = [filtered("frame_nii_1.fits", "NII"), filtered("frame_nii_2.fits", "NII")];
    const props: PanelProps = { files: [ha, ...nii] };
    const view = () => hooks.render(panel, props) as ReactNode;
    vi.mocked(open).mockResolvedValueOnce([nii[0].path, ha.path, "C:/r6/a1/calibset/ha_dialog_1.fits"]);
    press(laneAdd(view(), "Ha"));
    await vi.waitFor(() => expect(lanes(view())).toEqual(["Ha 2 files"]));
    expect(textOf(nodes(view()).find((el) => el.props["data-testid"] === "pipeline-lane-note"))).toBe("1 light not added to the Ha lane: NII (1); a lane holds one filter");
    expect(leftOutText(view())).toBe("2 lights left out by filter: NII (2)");
  });

  it("never adds lights loaded later to a lane that holds another filter, and names them as left out", () => {
    const nii = [filtered("frame_nii_1.fits", "NII"), filtered("frame_nii_2.fits", "NII")];
    const props: PanelProps = { files: nii };
    const view = () => hooks.render(panel, props) as ReactNode;
    choosePutLane(view(), "NII", "R");
    press(putButton(view(), "NII"));
    expect(lanes(view())).toEqual(["R 2 files"]);
    props.files = [...nii, ...LIGHTS];
    view();
    expect(lanes(view())).toEqual(["R 2 files"]);
    expect(leftOutText(view())).toBe("2 lights left out by filter: R (2)");
    expect(putOptions(view(), "R")).not.toContain("R");
  });

  it("names every + Add button after its lane or calibration row", () => {
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files: [...LIGHTS, ...NARROWBAND.slice(0, 4)], calibration: CALIBRATION }));
    expect([...html.matchAll(/<button[^>]*aria-label="([^"]*)"[^>]*>\+ Add<\/button>/g)].map((m) => m[1])).toEqual([
      "Add R lights",
      "Add Ha lights",
      "Add dark frames",
      "Add flat frames",
      "Add flat-dark frames",
      "Add bias frames",
    ]);
  });

  it("calibration + Add skips the frames the row already holds", async () => {
    const props: PanelProps = { files: LIGHTS, calibration: CALIBRATION };
    const view = () => hooks.render(panel, props) as ReactNode;
    const darksRow = () => nodes(view()).find((el) => el.props.label === "Darks");
    vi.mocked(open).mockResolvedValueOnce([CALIBRATION.darkPaths[0], "C:/r6/a1/calibset/dark_300s_2.fits"]);
    void (darksRow()?.props.onAdd as () => Promise<void>)();
    await vi.waitFor(() => expect(darksRow()?.props.count).not.toBe(1));
    expect(darksRow()?.props.count).toBe(2);
  });

  it("says there is no lane yet, not that no lights are loaded, when every light is left out by filter", () => {
    const files = [filtered("frame_nii_1.fits", "NII"), filtered("light_0001.fits", "3")];
    const html = renderToStaticMarkup(createElement(PipelinePanel, { files }));
    expect(staticLanes(html)).toEqual([]);
    expect(html).not.toContain("No lights loaded yet");
    expect(html).toContain("No lanes yet. Add a lane and pick its lights.");
    expect(html).toContain("2 lights left out by filter: NII (1), wheel position 3 (1)");
  });

  it("adds an empty lane in filter order with + Lane, fills it with + Add, and drops it again with Remove", async () => {
    const props: PanelProps = { files: LIGHTS };
    const view = () => hooks.render(panel, props) as ReactNode;
    let tree = view();
    expect(lanes(tree)).toEqual(["R 2 files"]);
    const pick = nodes(tree).find((el) => el.props["aria-label"] === "Lane to add");
    expect(nodes(pick?.props.children).map((o) => o.props.value)).toEqual(["L", "G", "B", "Ha", "OIII", "SII", "Mono"]);
    (pick?.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: "L" } });
    press(buttonNamed(view(), "+ Lane"));
    tree = view();
    expect(lanes(tree)).toEqual(["L 0 files", "R 2 files"]);
    expect(nodes(laneRow(tree, "L").props.children).some((el) => typeof el.props.onPress === "function")).toBe(false);

    vi.mocked(open).mockResolvedValueOnce(["C:/r6/a1/calibset/lum_1.fits", "C:/r6/a1/calibset/lum_2.fits"]);
    press(buttonNamed(laneRow(tree, "L").props.children as ReactNode, "+ Add"));
    await vi.waitFor(() => expect(lanes(view())).toEqual(["L 2 files", "R 2 files"]));
    expect(vi.mocked(open).mock.calls[0][0]).toMatchObject({ title: "Select L lights" });

    const clear = () => nodes(laneRow(view(), "L").props.children).find((el) => typeof el.props.onPress === "function");
    (clear()?.props.onPress as () => void)();
    (clear()?.props.onPress as () => void)();
    tree = view();
    expect(lanes(tree)).toEqual(["L 0 files", "R 2 files"]);
    press(buttonNamed(laneRow(tree, "L").props.children as ReactNode, "Remove"));
    expect(lanes(view())).toEqual(["R 2 files"]);
  });

  it("gives a filter loaded later its own lane and keeps the lanes already there", () => {
    const props: PanelProps = { files: LIGHTS };
    const view = () => hooks.render(panel, props) as ReactNode;
    expect(lanes(view())).toEqual(["R 2 files"]);
    props.files = [...LIGHTS, ...NARROWBAND.slice(0, 4)];
    view();
    expect(lanes(view())).toEqual(["R 2 files", "Ha 4 files"]);
  });

  it("keeps a cleared lane and a cleared Darks row empty when other files load, such as opened masters", () => {
    const dark = sessionFrame("dark_300s_2.fits", { IMAGETYP: "DARK" });
    const props: PanelProps = { files: [...LIGHTS, dark] };
    const view = () => hooks.render(panel, props) as ReactNode;
    const darksRow = () => nodes(view()).find((el) => el.props.label === "Darks");
    const laneClear = () => nodes(laneRow(view(), "R").props.children).find((el) => typeof el.props.onPress === "function");
    expect(darksRow()?.props.count).toBe(1);
    (darksRow()?.props.onClear as () => void)();
    (darksRow()?.props.onClear as () => void)();
    (laneClear()?.props.onPress as () => void)();
    (laneClear()?.props.onPress as () => void)();
    expect(lanes(view())).toEqual(["R 0 files"]);
    expect(darksRow()?.props.count).toBe(0);
    props.files = [...LIGHTS, dark, ...NARROWBAND.slice(0, 4)];
    view();
    expect(lanes(view())).toEqual(["R 0 files", "Ha 4 files"]);
    expect(darksRow()?.props.count).toBe(0);
  });

  it("keeps opened stacked masters and rejection maps out of the lanes and says so", () => {
    const masterCards = { IMAGETYP: "LIGHT", FILTER: "Red", ABCOMB: "mean", NCOMBINE: "2" };
    const master = { ...sessionFrame("light_120s_pipeline2_20261030-101500-000_R.fits", masterCards), path: "C:/out/light_120s_pipeline2_20261030-101500-000_R.fits" };
    const rejectionMap = { ...sessionFrame("light_120s_stack2_rejhigh.fits", { IMAGETYP: "LIGHT", FILTER: "Red", ABPROC: "rejection" }), path: "C:/out/light_120s_stack2_rejhigh.fits" };
    const props: PanelProps = { files: LIGHTS };
    const view = () => hooks.render(panel, props) as ReactNode;
    view();
    props.files = [...LIGHTS, master, rejectionMap];
    view();
    expect(lanes(view())).toEqual(["R 2 files"]);
    expect(textOf(nodes(view()).find((el) => el.props["data-testid"] === "pipeline-stacked-left-out"))).toBe("2 stacked results left out of the lanes");
    expect(staticLanes(renderToStaticMarkup(createElement(PipelinePanel, { files: [...LIGHTS, master, rejectionMap] })))).toEqual(["R 2 files"]);
  });
});

const MASTERS: PipelineMasterOutput[] = ["R", "G", "B"].map((label) => ({
  label,
  png_path: `C:/out/light_120s_pipeline6_${label}.png`,
  fits_path: `C:/out/light_120s_pipeline6_${label}.fits`,
  dimensions: [8, 8],
  input_path: `C:/r6/a1/calibset/light_${label}_1.fits`,
}));

describe("PipelinePanel results", () => {
  const RGB_LIGHTS = ["R", "G", "B"].flatMap((c) => [1, 2].map((n) => filtered(`light_${c}_${n}.fits`, { R: "Red", G: "Green", B: "Blue" }[c] as string)));

  beforeEach(() => {
    hooks.mount();
    runPipelineMock.mockReset();
    ingest.showWhenLoaded.mockReset();
    ingest.ingestUnlessLoaded.mockReset();
  });

  afterEach(() => hooks.mount());

  async function ran(reply: Partial<PipelineResult>, extra: Partial<PanelProps> = {}): Promise<() => ReactNode> {
    runPipelineMock.mockResolvedValue({ stats: stats({}), channel_previews: [], ...reply });
    const props: PanelProps = { files: RGB_LIGHTS, ...extra };
    const view = () => hooks.render(panel, props) as ReactNode;
    const run = nodes(view()).find((el) => "runningLabel" in el.props);
    void (run?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(nodes(view()).some((el) => el.type === "span" && textOf(el.props.children) === "Results")).toBe(true));
    return view;
  }

  const pills = (tree: ReactNode) => nodes(tree).filter((el) => String(el.props.className ?? "").startsWith("ab-pill")).map((el) => textOf(el.props.children));
  const composeButton = (tree: ReactNode) => nodes(tree).find((el) => el.props["data-testid"] === "pipeline-compose-masters");

  it("names the narrowband preview after the SHO palette the backend used", async () => {
    const onShow = vi.fn();
    const previews = ["Ha", "OIII", "SII"].map((label) => ({ label, pixels_b64: "", width: 1, height: 1 }));
    const view = await ran({ channel_previews: previews, rgb_preview: "AAAA", rgb_dimensions: [1, 1], rgb_palette: "SHO" }, { onShow });
    expect(pills(view())).toEqual(["Ha", "OIII", "SII", "SHO"]);
    expect(onShow.mock.calls[0][1]).toBe("RGB");
  });

  it("keeps the RGB pill when the backend names no palette", async () => {
    const view = await ran({ channel_previews: [], rgb_preview: "AAAA", rgb_dimensions: [1, 1] });
    expect(pills(view())).toEqual(["RGB"]);
  });

  it("Open masters adds every master to Files and shows the first", async () => {
    const view = await ran({ masters: MASTERS });
    press(buttonNamed(view(), "Open masters"));
    expect(ingest.showWhenLoaded.mock.calls).toEqual([[MASTERS[0].fits_path]]);
    expect(ingest.ingestUnlessLoaded.mock.calls).toEqual([[MASTERS[1].fits_path], [MASTERS[2].fits_path]]);
  });

  it("hands the masters to Compose at once when the wizard has no progress", async () => {
    const onComposeMasters = vi.fn();
    const view = await ran({ masters: MASTERS }, { onComposeMasters, composeHasProgress: false });
    press(buttonNamed(view(), "Compose these masters"));
    expect(onComposeMasters.mock.calls).toEqual([[MASTERS]]);
  });

  it("the button arms when the wizard has progress and hands the masters over on the second press", async () => {
    const onComposeMasters = vi.fn();
    const view = await ran({ masters: MASTERS }, { onComposeMasters, composeHasProgress: true });
    press(buttonNamed(view(), "Compose these masters"));
    expect(onComposeMasters).not.toHaveBeenCalled();
    const armed = composeButton(view());
    expect(textOf(armed?.props.children)).toBe("Replace Compose channels?");
    expect(armed?.props["aria-label"]).toBe("Confirm replacing the Compose channels with these masters");
    expect(armed?.props["aria-live"]).toBe("polite");
    press(armed as ReactElement<AnyProps>);
    expect(onComposeMasters.mock.calls).toEqual([[MASTERS]]);
    expect(textOf(composeButton(view())?.props.children)).toBe("Compose these masters");
  });

  it("disarms the Compose button when it loses focus", async () => {
    const onComposeMasters = vi.fn();
    const view = await ran({ masters: MASTERS }, { onComposeMasters, composeHasProgress: true });
    press(buttonNamed(view(), "Compose these masters"));
    (composeButton(view())?.props.onBlur as () => void)();
    expect(textOf(composeButton(view())?.props.children)).toBe("Compose these masters");
    press(buttonNamed(view(), "Compose these masters"));
    expect(onComposeMasters).not.toHaveBeenCalled();
  });

  it("offers neither hand-off when the run wrote no master", async () => {
    const view = await ran({ masters: [] }, { onComposeMasters: vi.fn() });
    const names = nodes(view()).filter((el) => typeof el.props.onClick === "function").map((el) => textOf(el.props.children).trim());
    expect(names).not.toContain("Open masters");
    expect(names).not.toContain("Compose these masters");
  });

  it("says how long the run took", async () => {
    const view = await ran({ elapsed_ms: 83_000 });
    expect(textOf(view())).toContain("Finished in 1m 23s");
  });
});

describe("PipelinePanel progress and cancel", () => {
  const IDLE = { current: 0, total: 0, percent: 0, stage: "", active: false };

  beforeEach(() => {
    hooks.mount();
    Object.assign(progress.state, IDLE);
    progress.events.clear();
    runPipelineMock.mockReset();
    vi.mocked(cancelProgress).mockReset().mockResolvedValue(true);
  });

  afterEach(() => hooks.mount());

  const byTestId = (tree: ReactNode, id: string) => nodes(tree).find((el) => el.props["data-testid"] === id);
  const runButton = (tree: ReactNode) => nodes(tree).find((el) => "runningLabel" in el.props);
  const errorShown = (tree: ReactNode) => nodes(tree).find((el) => el.type === ErrorAlert)?.props.message;

  async function started(): Promise<{ view: () => ReactNode; fail: (error: Error) => void }> {
    let fail: (error: Error) => void = () => {};
    runPipelineMock.mockReturnValue(new Promise<PipelineResult>((_, reject) => { fail = reject; }));
    const props: PanelProps = { files: LIGHTS };
    const view = () => hooks.render(panel, props) as ReactNode;
    void (runButton(view())?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(1));
    return { view, fail };
  }

  it("a progress event replaces the static label with the stage and k/N", async () => {
    const { view } = await started();
    expect(textOf(byTestId(view(), "pipeline-progress-label"))).toBe("Starting the pipeline...");
    expect([...progress.events]).toEqual(["pipeline-progress"]);
    Object.assign(progress.state, { current: 4, total: 8, percent: 50, stage: "stacking:1500:2/3:14:R", active: true });
    const tree = view();
    expect(textOf(byTestId(tree, "pipeline-progress-label"))).toBe("R 2/3: stacking 14 lights");
    expect(runButton(tree)?.props.runningLabel).not.toBe("Building calibration masters...");
    expect(byTestId(tree, "pipeline-progress-bar")?.props.style).toMatchObject({ width: "50%" });
    expect(textOf(byTestId(tree, "pipeline-elapsed"))).toMatch(/^(\d+ms|\d+s|\d+m \d+s)$/);
  });

  it.each(["Operation cancelled", "dark: Operation cancelled"])("the X cancels pipeline-progress, and %s reads Pipeline cancelled with no error", async (message) => {
    const { view, fail } = await started();
    Object.assign(progress.state, { current: 3, total: 8, percent: 38, stage: "loading:900:2/3:6:G", active: true });
    const cancel = byTestId(view(), "pipeline-cancel");
    expect(cancel?.props["aria-label"]).toBe("Cancel pipeline");
    press(cancel as ReactElement<AnyProps>);
    expect(vi.mocked(cancelProgress).mock.calls).toEqual([["pipeline-progress"]]);
    fail(new Error(message));
    await vi.waitFor(() => expect(textOf(view())).toContain("Pipeline cancelled"));
    const tree = view();
    expect(errorShown(tree)).toBeNull();
    expect(byTestId(tree, "pipeline-cancel")).toBeUndefined();
    runPipelineMock.mockReturnValue(new Promise<PipelineResult>(() => {}));
    void (runButton(tree)?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(2));
    expect(textOf(view())).not.toContain("Pipeline cancelled");
  });

  it("a run that fails for another reason still shows its error", async () => {
    const { view, fail } = await started();
    fail(new Error("Channel 'R': frame 1 has shape [8, 8] but frame 0 has [4, 4]. All frames must match."));
    await vi.waitFor(() => expect(String(errorShown(view()))).toContain("All frames must match"));
    expect(textOf(view())).not.toContain("Pipeline cancelled");
  });
});

describe("PipelinePanel integration settings", () => {
  const REQUEST_KEYS = [
    "channels", "dark_paths", "flat_paths", "bias_paths", "flat_dark_paths",
    "rejection", "sigma_low", "sigma_high", "max_iterations", "winsor_cutoff", "percentile_low", "percentile_high", "minmax_low", "minmax_high",
    "combine", "normalize", "align", "cosmetic", "dark_optimize",
  ];

  beforeEach(() => {
    hooks.mount();
    runPipelineMock.mockReset();
    runPipelineMock.mockResolvedValue({ stats: stats({}), channel_previews: [] });
  });

  afterEach(() => hooks.mount());

  const rejectionControls = (tree: ReactNode) => nodes(tree).find((el) => el.props.settings !== undefined && typeof el.props.onChange === "function");
  const touch = (tree: ReactNode, change: Record<string, unknown>) => (rejectionControls(tree)?.props.onChange as (c: Record<string, unknown>) => void)(change);
  const normalizeToggle = (tree: ReactNode) => nodes(tree).find((el) => typeof el.props.label === "string" && String(el.props.label).startsWith("Normalize"));
  const normalizeNote = (tree: ReactNode) => nodes(tree).find((el) => el.props["data-testid"] === "pipeline-normalize-note");

  async function requestOf(view: () => ReactNode): Promise<PipelineRequest> {
    void (nodes(view()).find((el) => "runningLabel" in el.props)?.props.onClick as () => Promise<void>)();
    await vi.waitFor(() => expect(runPipelineMock).toHaveBeenCalledTimes(1));
    return runPipelineMock.mock.calls[0][0] as PipelineRequest;
  }

  it("the request carries minmax_low: 2 after the Stack tab sets 2/2", async () => {
    const props: PanelProps = { files: LIGHTS, stackConfig: DEFAULT_STACK_SETTINGS };
    const view = () => hooks.render(panel, props) as ReactNode;
    view();
    props.stackConfig = { ...DEFAULT_STACK_SETTINGS, rejection: "min_max", minmaxLow: 2, minmaxHigh: 2 };
    view();
    expect(rejectionControls(view())?.props).toMatchObject({ frameCount: 2, settings: { rejection: "min_max", minmaxLow: 2, minmaxHigh: 2 } });
    const request = await requestOf(view);
    expect(Object.keys(request).sort()).toEqual([...REQUEST_KEYS].sort());
    expect(request).toMatchObject({
      rejection: "min_max",
      sigma_low: 4,
      sigma_high: 3,
      max_iterations: 5,
      winsor_cutoff: 5,
      percentile_low: 0.2,
      percentile_high: 0.1,
      minmax_low: 2,
      minmax_high: 2,
    });
  });

  it("seeds every rejection parameter from the Stack tab, a sigma above 5 included, until a Pipeline control is touched", async () => {
    const props: PanelProps = {
      files: LIGHTS,
      stackConfig: { ...DEFAULT_STACK_SETTINGS, rejection: "percentile_clip", sigmaLow: 5.5, maxIterations: 9, winsorCutoff: 2.5, percentileLow: 0.3, percentileHigh: 0.15, minmaxLow: 2, minmaxHigh: 3, combine: "median" },
    };
    const view = () => hooks.render(panel, props) as ReactNode;
    expect(rejectionControls(view())?.props.settings).toEqual({
      rejection: "percentile_clip",
      sigmaLow: 5.5,
      sigmaHigh: 3,
      maxIterations: 9,
      winsorCutoff: 2.5,
      percentileLow: 0.3,
      percentileHigh: 0.15,
      minmaxLow: 2,
      minmaxHigh: 3,
    });
    touch(view(), { percentileHigh: 0.05 });
    props.stackConfig = { ...DEFAULT_STACK_SETTINGS, rejection: "min_max", minmaxLow: 4, minmaxHigh: 4 };
    view();
    expect(rejectionControls(view())?.props.settings).toMatchObject({ rejection: "percentile_clip", percentileLow: 0.3, percentileHigh: 0.05, minmaxLow: 2 });
    const request = await requestOf(view);
    expect(request).toMatchObject({
      rejection: "percentile_clip",
      sigma_low: 5.5,
      max_iterations: 9,
      winsor_cutoff: 2.5,
      percentile_low: 0.3,
      percentile_high: 0.05,
      minmax_low: 2,
      minmax_high: 3,
      combine: "median",
    });
  });

  it("without the Stack tab keeps its own 2.5/3.0 sigmas and the backend's other defaults, and shows no hint without lights", async () => {
    const view = () => hooks.render(panel, { files: LIGHTS }) as ReactNode;
    expect(rejectionControls(view())?.props.frameCount).toBe(2);
    expect(await requestOf(view)).toMatchObject({
      rejection: "sigma_clip",
      sigma_low: 2.5,
      sigma_high: 3,
      max_iterations: 5,
      winsor_cutoff: 5,
      percentile_low: 0.2,
      percentile_high: 0.1,
      minmax_low: 1,
      minmax_high: 1,
      normalize: true,
    });
    hooks.mount();
    expect(rejectionControls(hooks.render(panel, { files: [] }) as ReactNode)?.props.frameCount).toBeNull();
  });

  it("says that its normalization divides by the sky background and names a Stack tab method that differs", () => {
    const props: PanelProps = { files: LIGHTS, stackConfig: DEFAULT_STACK_SETTINGS };
    const view = () => hooks.render(panel, props) as ReactNode;
    const toggle = normalizeToggle(view());
    expect(toggle?.props.label).toBe("Normalize to sky background (divide)");
    expect(toggle?.props.checked).toBe(true);
    expect(textOf(normalizeNote(view()))).toBe("The Stack tab normalization is Additive with scaling; this divides each light by its own sky background instead.");
    expect(toggle?.props.describedBy).toBe(normalizeNote(view())?.props.id);
    (toggle?.props.onChange as (on: boolean) => void)(false);
    expect(textOf(normalizeNote(view()))).toBe("The Stack tab normalization is Additive with scaling; the Pipeline stacks these lights without normalization.");
    props.stackConfig = { ...DEFAULT_STACK_SETTINGS, normalization: "none" };
    expect(normalizeNote(view())).toBeUndefined();
    expect(normalizeToggle(view())?.props.describedBy).toBeUndefined();
    (normalizeToggle(view())?.props.onChange as (on: boolean) => void)(true);
    expect(textOf(normalizeNote(view()))).toBe("The Stack tab normalization is None; this divides each light by its own sky background instead.");
    hooks.mount();
    expect(normalizeNote(hooks.render(panel, { files: LIGHTS }) as ReactNode)).toBeUndefined();
  });
});
