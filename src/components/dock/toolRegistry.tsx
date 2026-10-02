import { lazy } from "react";
import {
  BarChart3,
  Download,
  FileText,
  FlaskConical,
  FolderOpen,
  Info,
  Layers,
  Layers2,
  Settings,
  Sparkles,
  type LucideIcon,
} from "lucide-react";
import type { DockToolId } from "../../utils/dockLayout";
import { InfoPanel } from "../file/SidebarPanels";
import AnalysisTool from "./tools/AnalysisTool";

const ComposeWizard = lazy(() => import("../compose/ComposeWizard"));
const HeadersTab = lazy(() => import("../header/HeadersTab"));
const ProcessingTab = lazy(() => import("../processing/ProcessingTab"));
const StackingTab = lazy(() => import("../stacking/StackingTab"));
const SynthPanel = lazy(() => import("../synth/SynthPanel"));
const ExportTab = lazy(() => import("../export/ExportTab"));
const ConfigTab = lazy(() => import("../preview/ConfigTab"));

export interface DockToolDef {
  id: DockToolId;
  icon: LucideIcon;
  render: () => React.ReactNode;
}

export const DOCK_TOOLS: Record<DockToolId, DockToolDef> = {
  files: { id: "files", icon: FolderOpen, render: () => null },
  info: { id: "info", icon: Info, render: () => <InfoPanel /> },
  compose: { id: "compose", icon: Layers, render: () => <ComposeWizard /> },
  headers: { id: "headers", icon: FileText, render: () => <HeadersTab /> },
  analysis: { id: "analysis", icon: BarChart3, render: () => <AnalysisTool /> },
  processing: { id: "processing", icon: Sparkles, render: () => <ProcessingTab /> },
  stacking: { id: "stacking", icon: Layers2, render: () => <StackingTab /> },
  synth: { id: "synth", icon: FlaskConical, render: () => <SynthPanel /> },
  export: { id: "export", icon: Download, render: () => <ExportTab /> },
  config: { id: "config", icon: Settings, render: () => <ConfigTab /> },
};
