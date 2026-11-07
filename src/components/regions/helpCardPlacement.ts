export interface HelpCardPlacement {
  left: number;
  top: number;
  width: number;
}

const HELP_CARD_WIDTH_PX = 280;
const EDGE_GAP_PX = 8;

export function placeHelpCard(
  buttonRect: Pick<DOMRect, "left" | "top" | "bottom">,
  cardHeight: number,
  viewport: { width: number; height: number },
): HelpCardPlacement {
  const width = Math.min(HELP_CARD_WIDTH_PX, viewport.width - 2 * EDGE_GAP_PX);
  const left = Math.max(EDGE_GAP_PX, Math.min(buttonRect.left - EDGE_GAP_PX, viewport.width - width - EDGE_GAP_PX));
  const below = buttonRect.bottom + EDGE_GAP_PX;
  const top = below + cardHeight <= viewport.height - EDGE_GAP_PX
    ? below
    : Math.max(EDGE_GAP_PX, buttonRect.top - cardHeight - EDGE_GAP_PX);
  return { left, top, width };
}
