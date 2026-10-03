import { Suspense, memo, useInsertionEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Loader2 } from "lucide-react";
import {
  DOCK_TOOL_META,
  anchorOrientation,
  keptToolFileKey,
  toolMountState,
  type DockAnchor,
  type DockToolId,
} from "../../utils/dockLayout";
import { useGpuDisplay } from "../../hooks/useGpuDisplay";
import { ToolHostContext } from "../../context/ToolHostContext";

export interface ToolMountProps {
  tool: DockToolId;
  content: React.ReactNode;
  containers: React.RefObject<Record<DockAnchor, HTMLElement | null>>;
  anchor: DockAnchor;
  activeTool: DockToolId | null;
  shownTool: DockToolId | null;
  anchorMounted: boolean;
  hasFile: boolean;
  fileKey: string | null;
  groupActive: boolean;
}

type ScrollOffsets = [Element, number, number][];

function saveScrollOffsets(host: HTMLElement): ScrollOffsets {
  const saved: ScrollOffsets = [];
  const record = (el: Element) => {
    if (el.scrollTop !== 0 || el.scrollLeft !== 0) saved.push([el, el.scrollTop, el.scrollLeft]);
  };
  record(host);
  host.querySelectorAll("*").forEach(record);
  return saved;
}

function restoreScrollOffsets(saved: ScrollOffsets): void {
  for (const [el, top, left] of saved) {
    if (!el.isConnected) continue;
    el.scrollTop = top;
    el.scrollLeft = left;
  }
}

function attachHost(host: HTMLElement, container: HTMLElement | null): void {
  if (!container || host.parentElement === container) return;
  const saved = saveScrollOffsets(host);
  container.appendChild(host);
  restoreScrollOffsets(saved);
}

function createHost(tool: DockToolId): HTMLDivElement {
  const host = document.createElement("div");
  host.className = "ab-dock-host";
  host.dataset.dockHost = tool;
  return host;
}

function TabSpinner() {
  return (
    <div className="flex items-center justify-center py-8">
      <Loader2 size={16} className="animate-spin" style={{ color: "var(--ab-teal)" }} />
    </div>
  );
}

const ToolMount = memo(function ToolMount({ tool, content, containers, anchor, activeTool, shownTool, anchorMounted, hasFile, fileKey, groupActive }: ToolMountProps) {
  const meta = DOCK_TOOL_META[tool];
  const [host] = useState(() => createHost(tool));
  const [ready, setReady] = useState(false);
  const [keptFileKey, setKeptFileKey] = useState<string | null>(null);
  const nextKeptFileKey = keptToolFileKey(activeTool === tool, fileKey, keptFileKey, groupActive);
  if (nextKeptFileKey !== keptFileKey) setKeptFileKey(nextKeptFileKey);
  const state = toolMountState({
    tool,
    activeTool,
    shownTool,
    anchorMounted,
    hasFile,
    fileKey,
    keptFileKey: nextKeptFileKey,
    mount: meta.mount,
    needsFile: meta.needsFile,
  });
  const gpuDisplay = useGpuDisplay();
  const hostValue = useMemo(() => ({ active: state.active, gpuDisplay }), [state.active, gpuDisplay]);
  const orientation = anchorOrientation(anchor);
  const pendingScroll = useRef<ScrollOffsets | null>(null);
  const visible = state.visible;
  const mounted = state.mounted;

  useInsertionEffect(() => {
    if (!visible && mounted && !host.hidden) pendingScroll.current = saveScrollOffsets(host);
    host.hidden = !visible;
    host.inert = !visible;
    host.dataset.orientation = orientation;
    host.className = `ab-dock-host ${orientation === "horizontal" ? "ab-tool-fade-y" : "ab-tool-fade"}`;
    attachHost(host, containers.current?.[anchor] ?? null);
  }, [host, visible, mounted, orientation, anchor, containers]);

  useLayoutEffect(() => {
    attachHost(host, containers.current?.[anchor] ?? null);
    if (!mounted) {
      pendingScroll.current = null;
    } else if (!host.hidden && pendingScroll.current) {
      restoreScrollOffsets(pendingScroll.current);
      pendingScroll.current = null;
    }
  });

  useLayoutEffect(() => setReady(true), []);

  useLayoutEffect(() => () => host.remove(), [host]);

  if (!mounted || !ready) return null;
  return createPortal(
    <ToolHostContext.Provider value={hostValue}>
      <Suspense fallback={<TabSpinner />}>{content}</Suspense>
    </ToolHostContext.Provider>,
    host,
  );
});

export default ToolMount;
