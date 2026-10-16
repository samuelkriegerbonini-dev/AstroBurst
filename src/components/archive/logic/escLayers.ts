export type EscLayer = "menu" | "large" | "drawer" | "token" | "search";

export interface EscContext {
  menuOpen: boolean;
  largeOpen: boolean;
  drawerOpen: boolean;
  tokenChecking: boolean;
  searching: boolean;
}

export function innermostLayer(ctx: EscContext): EscLayer | null {
  if (ctx.menuOpen) return "menu";
  if (ctx.largeOpen) return "large";
  if (ctx.drawerOpen) return "drawer";
  if (ctx.tokenChecking) return "token";
  if (ctx.searching) return "search";
  return null;
}
