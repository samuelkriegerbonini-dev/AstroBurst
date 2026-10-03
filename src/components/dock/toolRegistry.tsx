import { lazy } from "react";
import {
  Aperture,
  Box,
  Download,
  FileText,
  FlaskConical,
  FolderOpen,
  ImageIcon,
  Info,
  Layers,
  Layers2,
  ScrollText,
  Settings,
  Sparkles,
  Telescope,
  type LucideIcon,
} from "lucide-react";
import type { DockToolId } from "../../utils/dockLayout";
import { InfoPanel } from "../file/SidebarPanels";

const ComposeWizard = lazy(() => import("../compose/ComposeWizard"));
const HeadersTab = lazy(() => import("../header/HeadersTab"));
const ProcessingTab = lazy(() => import("../processing/ProcessingTab"));
const StackingTab = lazy(() => import("../stacking/StackingTab"));
const SynthPanel = lazy(() => import("../synth/SynthPanel"));
const ExportTab = lazy(() => import("../export/ExportTab"));
const ConfigTab = lazy(() => import("../preview/ConfigTab"));
const ImageTool = lazy(() => import("../analysis/ImageTool"));
const AstrometryTool = lazy(() => import("../analysis/AstrometryTool"));
const PhotometryTool = lazy(() => import("../analysis/PhotometryTool"));
const CubeTool = lazy(() => import("../analysis/CubeTool"));
const LogTool = lazy(() => import("../analysis/LogTool"));

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
  image: { id: "image", icon: ImageIcon, render: () => <ImageTool /> },
  astrometry: { id: "astrometry", icon: Telescope, render: () => <AstrometryTool /> },
  photometry: { id: "photometry", icon: Aperture, render: () => <PhotometryTool /> },
  cube: { id: "cube", icon: Box, render: () => <CubeTool /> },
  processing: { id: "processing", icon: Sparkles, render: () => <ProcessingTab /> },
  stacking: { id: "stacking", icon: Layers2, render: () => <StackingTab /> },
  synth: { id: "synth", icon: FlaskConical, render: () => <SynthPanel /> },
  export: { id: "export", icon: Download, render: () => <ExportTab /> },
  config: { id: "config", icon: Settings, render: () => <ConfigTab /> },
  log: { id: "log", icon: ScrollText, render: () => <LogTool /> },
};
