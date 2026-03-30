declare module "lucide-react" {
  import { FC, SVGAttributes } from "react";
  interface IconProps extends SVGAttributes<SVGElement> {
    size?: number | string;
    strokeWidth?: number | string;
    className?: string;
  }
  type Icon = FC<IconProps>;
  export type LucideIcon = Icon;
  export const Activity: Icon;
  export const AlertCircle: Icon;
  export const AlertTriangle: Icon;
  export const ArrowDown: Icon;
  export const ArrowRight: Icon;
  export const BarChart3: Icon;
  export const Box: Icon;
  export const Check: Icon;
  export const CheckCircle2: Icon;
  export const ChevronDown: Icon;
  export const ChevronRight: Icon;
  export const ChevronUp: Icon;
  export const Compass: Icon;
  export const Copy: Icon;
  export const Cpu: Icon;
  export const Crosshair: Icon;
  export const Database: Icon;
  export const Download: Icon;
  export const Eye: Icon;
  export const EyeOff: Icon;
  export const FileDown: Icon;
  export const FileText: Icon;
  export const Film: Icon;
  export const FlaskConical: Icon;
  export const FolderOpen: Icon;
  export const Globe: Icon;
  export const Grid3X3: Icon;
  export const GripVertical: Icon;
  export const Home: Icon;
  export const Image: Icon;
  export const ImageIcon: Icon;
  export const Info: Icon;
  export const Key: Icon;
  export const Keyboard: Icon;
  export const Layers: Icon;
  export const Layers2: Icon;
  export const Link: Icon;
  export const Loader2: Icon;
  export const Maximize2: Icon;
  export const Minimize2: Icon;
  export const PackageOpen: Icon;
  export const Palette: Icon;
  export const Pause: Icon;
  export const Play: Icon;
  export const Plus: Icon;
  export const RefreshCw: Icon;
  export const RotateCcw: Icon;
  export const Save: Icon;
  export const Search: Icon;
  export const Settings: Icon;
  export const SkipBack: Icon;
  export const SkipForward: Icon;
  export const SlidersHorizontal: Icon;
  export const Sparkles: Icon;
  export const Star: Icon;
  export const Telescope: Icon;
  export const Unlink: Icon;
  export const Upload: Icon;
  export const Wand2: Icon;
  export const X: Icon;
  export const Zap: Icon;
  export const ZoomIn: Icon;
  export const ZoomOut: Icon;
}

declare module "@tauri-apps/plugin-dialog" {
  interface OpenDialogOptions {
    multiple?: boolean;
    directory?: boolean;
    filters?: { name: string; extensions: string[] }[];
    defaultPath?: string;
    title?: string;
  }
  interface SaveDialogOptions {
    filters?: { name: string; extensions: string[] }[];
    defaultPath?: string;
    title?: string;
  }
  export function open(options?: OpenDialogOptions): Promise<string | string[] | null>;
  export function save(options?: SaveDialogOptions): Promise<string | null>;
}

declare module "@tauri-apps/plugin-fs" {
  interface DirEntry {
    name: string;
    isDirectory: boolean;
    isFile: boolean;
    isSymlink: boolean;
    children?: DirEntry[];
  }
  export function readDir(path: string): Promise<DirEntry[]>;
  export function readFile(path: string): Promise<Uint8Array>;
  export function readTextFile(path: string): Promise<string>;
  export function writeFile(path: string, contents: Uint8Array): Promise<void>;
  export function writeTextFile(path: string, contents: string): Promise<void>;
  export function exists(path: string): Promise<boolean>;
  export function mkdir(path: string, options?: { recursive?: boolean }): Promise<void>;
}

declare module "@tauri-apps/plugin-opener" {
  export function revealItemInDir(path: string): Promise<void>;
}

declare module "@tauri-apps/plugin-shell" {
  export function open(url: string): Promise<void>;
}

declare module "framer-motion" {
  import { ComponentType, ReactNode, HTMLAttributes } from "react";
  export const AnimatePresence: ComponentType<{ children?: ReactNode; mode?: string; initial?: boolean }>;
  export const motion: {
    div: ComponentType<HTMLAttributes<HTMLDivElement> & Record<string, any>>;
    span: ComponentType<HTMLAttributes<HTMLSpanElement> & Record<string, any>>;
    [key: string]: ComponentType<any>;
  };
}

declare module "jszip" {
  class JSZip {
    file(name: string, data: any, options?: any): this;
    generateAsync(options: { type: string; compression?: string }, onUpdate?: (meta: { percent: number }) => void): Promise<Blob>;
  }
  export default JSZip;
}

declare module "file-saver" {
  export function saveAs(blob: Blob, filename: string): void;
}

declare module "openseadragon" {
  const OSD: any;
  export default OSD;
}
