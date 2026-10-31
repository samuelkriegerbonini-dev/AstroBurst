import { tipAnchorSelector } from "./tipAnchor";
import { useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { anchorOf, anchorSide, type DockLayout } from "../../utils/dockLayout";
import { tipPlacement, type DockTip, type TipPlacement } from "../../utils/dockOnboarding";
import { dockOnboardingStore } from "./dockOnboardingStore";
import { focusStripButton } from "./dockGeometry";
import "./dock.css";

export interface DockTipCardProps {
  tip: DockTip;
  titleId: string;
  bodyId: string;
  placement: TipPlacement | null;
  onGotIt(): void;
  onSkip(): void;
  cardRef?: React.Ref<HTMLDivElement>;
}

const UNPLACED_STYLE: React.CSSProperties = { left: 0, top: 0, visibility: "hidden" };

function placedStyle(placement: TipPlacement): React.CSSProperties {
  return {
    left: placement.left,
    top: placement.top,
    width: placement.width,
    maxHeight: placement.maxHeight,
    "--ab-tip-arrow-y": `${placement.arrowY}px`,
  } as React.CSSProperties;
}

export function DockTipCard({ tip, titleId, bodyId, placement, onGotIt, onSkip, cardRef }: DockTipCardProps): React.JSX.Element {
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    onGotIt();
  };

  return (
    <div
      ref={cardRef}
      role="dialog"
      aria-modal="false"
      aria-labelledby={titleId}
      aria-describedby={bodyId}
      data-dock-onboarding=""
      data-dock-tip={tip.id}
      data-arrow={placement?.arrow ?? "none"}
      className="ab-dock-onboarding ab-dock-tip"
      style={placement ? placedStyle(placement) : UNPLACED_STYLE}
      onKeyDown={onKeyDown}
    >
      <div className="ab-dock-onboarding-head">
        <h2 id={titleId} className="ab-dock-onboarding-title">{tip.title}</h2>
      </div>
      <div className="ab-dock-onboarding-scroll">
        <p id={bodyId} className="ab-dock-onboarding-text">{tip.body}</p>
      </div>
      <div className="ab-dock-onboarding-foot">
        <button type="button" className="ab-dock-onboarding-btn" onClick={onSkip}>
          Skip tips
        </button>
        <button type="button" className="ab-dock-onboarding-btn ab-dock-onboarding-btn-primary" onClick={onGotIt}>
          Got it
        </button>
      </div>
    </div>
  );
}


function samePlacement(a: TipPlacement, b: TipPlacement): boolean {
  return a.left === b.left && a.top === b.top && a.width === b.width && a.maxHeight === b.maxHeight && a.arrow === b.arrow && a.arrowY === b.arrowY;
}

export interface DockTipPopupProps {
  rootRef: React.RefObject<HTMLElement | null>;
  tip: DockTip;
  layout: DockLayout;
}

export function DockTipPopup({ rootRef, tip, layout }: DockTipPopupProps): React.ReactNode {
  const titleId = useId();
  const bodyId = useId();
  const cardRef = useRef<HTMLDivElement>(null);
  const [placement, setPlacement] = useState<TipPlacement | null>(null);
  const side = anchorSide(anchorOf(layout, tip.tool));

  useLayoutEffect(() => {
    const card = cardRef.current;
    const button = rootRef.current?.querySelector<HTMLElement>(tipAnchorSelector(tip.tool));
    if (!card || !button) return;
    const strip = button.closest<HTMLElement>("[data-dock-strip]");
    const place = () => {
      const next = tipPlacement(button.getBoundingClientRect(), side, { width: window.innerWidth, height: window.innerHeight }, card.offsetHeight);
      setPlacement((prev) => (prev !== null && samePlacement(prev, next) ? prev : next));
    };
    place();
    const observer = new ResizeObserver(place);
    observer.observe(card);
    if (strip) observer.observe(strip);
    strip?.addEventListener("scroll", place);
    window.addEventListener("resize", place);
    return () => {
      observer.disconnect();
      strip?.removeEventListener("scroll", place);
      window.removeEventListener("resize", place);
    };
  }, [rootRef, tip.tool, side, layout]);

  const close = (finish: () => void) => {
    const card = cardRef.current;
    const refocus = card !== null && card.contains(document.activeElement);
    finish();
    if (!refocus) return;
    requestAnimationFrame(() => {
      focusStripButton(rootRef.current?.querySelector<HTMLElement>(tipAnchorSelector(tip.tool)));
    });
  };

  return createPortal(
    <DockTipCard
      tip={tip}
      titleId={titleId}
      bodyId={bodyId}
      placement={placement}
      onGotIt={() => close(dockOnboardingStore.dismiss)}
      onSkip={() => close(dockOnboardingStore.skipAll)}
      cardRef={cardRef}
    />,
    document.body,
  );
}
