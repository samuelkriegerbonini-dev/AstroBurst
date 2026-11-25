export type EscLayer = "menu" | "large" | "drawer" | "downloads" | "token" | "search";

export interface EscContext {
  menuOpen: boolean;
  largeOpen: boolean;
  drawerOpen: boolean;
  downloadsOpen?: boolean;
  tokenChecking: boolean;
  searching: boolean;
}

export function innermostLayer(ctx: EscContext): EscLayer | null {
  if (ctx.menuOpen) return "menu";
  if (ctx.largeOpen) return "large";
  if (ctx.drawerOpen) return "drawer";
  if (ctx.downloadsOpen) return "downloads";
  if (ctx.tokenChecking) return "token";
  if (ctx.searching) return "search";
  return null;
}
