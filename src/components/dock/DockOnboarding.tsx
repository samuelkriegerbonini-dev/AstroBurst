import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import {
  DOCK_ONBOARDING_DELAY_MS,
  DOCK_ONBOARDING_STEPS,
  DOCK_ONBOARDING_TIP,
  DOCK_ONBOARDING_TITLE,
  onboardingPlacement,
  type DockOnboardingDemo,
  type DockOnboardingStep,
  type OnboardingPlacement,
} from "../../utils/dockOnboarding";
import { dockOnboardingStore, useDockOnboarding } from "./dockOnboardingStore";
import { focusStripButton } from "./dockGeometry";
import "./dock.css";

export interface DockOnboardingCardProps {
  step: number;
  titleId: string;
  bodyId: string;
  placement: OnboardingPlacement | null;
  onNext(): void;
  onBack(): void;
  onClose(): void;
  cardRef?: React.Ref<HTMLDivElement>;
  primaryRef?: React.Ref<HTMLButtonElement>;
}

interface DemoBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface DemoScene {
  rightSlots: number[];
  source: number;
  target: number;
  preview: DemoBox;
  pill: { x: number; y: number; w: number };
  arrow: { line: string; head: string };
}

const STRIP_SLOT_X = { left: 3.5, right: 228.5 };
const LEFT_SLOTS = [5, 16, 91];
const SLOT_SIZE = 8;
const PILL_HEIGHT = 12;

const DEMO_SCENES: Record<DockOnboardingDemo, DemoScene> = {
  vertical: {
    rightSlots: [5, 16, 27, 91],
    source: 80,
    target: 38,
    preview: { x: 165.5, y: 0.5, w: 60, h: 103 },
    pill: { x: 132, y: 41, w: 82 },
    arrow: { line: "M 219 84 L 219 45", head: "M 215.5 48.5 L 219 44.5 L 222.5 48.5" },
  },
  horizontal: {
    rightSlots: [5, 16, 91],
    source: 27,
    target: 80,
    preview: { x: 120, y: 70, w: 105.5, h: 33.5 },
    pill: { x: 120, y: 53, w: 94 },
    arrow: { line: "M 219 31 L 219 83", head: "M 215.5 79.5 L 219 83.5 L 222.5 79.5" },
  },
};

function Slot({ x, y, className = "ab-dock-onboarding-slot" }: { x: number; y: number; className?: string }) {
  return <rect className={className} x={x} y={y} width={SLOT_SIZE} height={SLOT_SIZE} rx={1.5} />;
}

function DockOnboardingDemoFrame({ step }: { step: DockOnboardingStep }): React.JSX.Element {
  const scene = DEMO_SCENES[step.demo];
  const { preview, pill, arrow } = scene;
  return (
    <svg
      className="ab-dock-onboarding-demo"
      data-onboarding-demo={step.demo}
      viewBox="0 0 240 104"
      aria-hidden="true"
      focusable="false"
    >
      <rect className="ab-dock-onboarding-frame" x={0.5} y={0.5} width={239} height={103} rx={4} />
      <rect className="ab-dock-onboarding-viewer" x={14.5} y={0.5} width={211} height={69.5} />
      <rect className="ab-dock-onboarding-bottom" x={14.5} y={70} width={105.5} height={33.5} />
      <rect className="ab-dock-onboarding-bottom" x={120} y={70} width={105.5} height={33.5} />
      <rect className="ab-dock-onboarding-strip" x={0.5} y={0.5} width={14} height={103} />
      <rect className="ab-dock-onboarding-strip" x={225.5} y={0.5} width={14} height={103} />
      {LEFT_SLOTS.map((y) => (
        <Slot key={`l${y}`} x={STRIP_SLOT_X.left} y={y} />
      ))}
      {scene.rightSlots.map((y) => (
        <Slot key={`r${y}`} x={STRIP_SLOT_X.right} y={y} />
      ))}
      <Slot x={STRIP_SLOT_X.right} y={scene.source} className="ab-dock-onboarding-source" />
      <rect className="ab-dock-onboarding-preview" x={preview.x} y={preview.y} width={preview.w} height={preview.h} rx={1.5} />
      <g className="ab-dock-onboarding-static-arrow">
        <path d={arrow.line} />
        <path d={arrow.head} />
      </g>
      <g className="ab-dock-onboarding-pill">
        <rect x={pill.x} y={pill.y} width={pill.w} height={PILL_HEIGHT} rx={PILL_HEIGHT / 2} />
        <text x={pill.x + pill.w / 2} y={pill.y + 8.5} textAnchor="middle">{step.label}</text>
      </g>
      <g className="ab-dock-onboarding-mover" data-path={step.demo} transform={`translate(${STRIP_SLOT_X.right} ${scene.target})`}>
        <rect className="ab-dock-onboarding-mover-icon" width={SLOT_SIZE} height={SLOT_SIZE} rx={1.5} />
        <path className="ab-dock-onboarding-cursor" d="M 5 5 L 5 13 L 7.2 10.8 L 9 14.5 L 10.4 13.8 L 8.7 10.2 L 11.5 10.2 Z" />
      </g>
    </svg>
  );
}

export function DockOnboardingCard({ step, titleId, bodyId, placement, onNext, onBack, onClose, cardRef, primaryRef }: DockOnboardingCardProps): React.JSX.Element {
  const index = Math.min(Math.max(0, step), DOCK_ONBOARDING_STEPS.length - 1);
  const current = DOCK_ONBOARDING_STEPS[index];
  const total = DOCK_ONBOARDING_STEPS.length;
  const last = index === total - 1;

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    onClose();
  };

  return (
    <div
      ref={cardRef}
      role="dialog"
      aria-modal="false"
      aria-labelledby={titleId}
      aria-describedby={bodyId}
      data-dock-onboarding=""
      data-arrow={placement?.arrow ?? "none"}
      className="ab-dock-onboarding"
      style={placement ? { left: placement.left, top: placement.top, width: placement.width, maxHeight: placement.maxHeight } : undefined}
      onKeyDown={onKeyDown}
    >
      <div className="ab-dock-onboarding-head">
        <h2 id={titleId} className="ab-dock-onboarding-title">{DOCK_ONBOARDING_TITLE}</h2>
        <button type="button" aria-label="Close" title="Close" className="ab-dock-onboarding-close" onClick={onClose}>
          <X size={12} aria-hidden="true" />
        </button>
      </div>
      <div className="ab-dock-onboarding-scroll">
        <DockOnboardingDemoFrame key={current.demo} step={current} />
        <div className="ab-dock-onboarding-step">
          <span className="ab-dock-onboarding-count">{`Step ${index + 1} of ${total}`}</span>
          <h3 className="ab-dock-onboarding-step-title">{current.title}</h3>
        </div>
        <p id={bodyId} className="ab-dock-onboarding-text">{current.body}</p>
        <p className="ab-dock-onboarding-tip">{DOCK_ONBOARDING_TIP}</p>
      </div>
      <div className="ab-dock-onboarding-foot">
        <div className="ab-dock-onboarding-dots" aria-hidden="true">
          {DOCK_ONBOARDING_STEPS.map((s, i) => (
            <span key={s.demo} className="ab-dock-onboarding-dot" data-onboarding-dot={i === index ? "active" : "idle"} />
          ))}
          <span className="ab-dock-onboarding-progress">{`${index + 1}/${total}`}</span>
        </div>
        <div className="ab-dock-onboarding-actions">
          {index > 0 && (
            <button key="back" type="button" className="ab-dock-onboarding-btn" onClick={onBack}>
              Back
            </button>
          )}
          <button
            key="primary"
            ref={primaryRef}
            type="button"
            className="ab-dock-onboarding-btn ab-dock-onboarding-btn-primary"
            onClick={last ? onClose : onNext}
          >
            {last ? "Got it" : "Next"}
          </button>
        </div>
      </div>
    </div>
  );
}

interface DockOnboardingProps {
  rootRef: React.RefObject<HTMLElement | null>;
}

function DockOnboardingPopup({ rootRef, step }: DockOnboardingProps & { step: number }): React.ReactNode {
  const titleId = useId();
  const bodyId = useId();
  const cardRef = useRef<HTMLDivElement>(null);
  const primaryRef = useRef<HTMLButtonElement>(null);
  const [placement, setPlacement] = useState<OnboardingPlacement | null>(null);

  useLayoutEffect(() => {
    const viewer = rootRef.current?.querySelector<HTMLElement>("[data-dock-viewer]");
    if (!viewer) return;
    const place = () => {
      setPlacement(onboardingPlacement(viewer.getBoundingClientRect(), { width: window.innerWidth, height: window.innerHeight }));
    };
    place();
    const observer = new ResizeObserver(place);
    observer.observe(viewer);
    window.addEventListener("resize", place);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
    };
  }, [rootRef]);

  const focusInside = () => {
    const card = cardRef.current;
    return card !== null && card.contains(document.activeElement);
  };

  const close = () => {
    const refocus = focusInside();
    dockOnboardingStore.dismiss();
    if (!refocus) return;
    requestAnimationFrame(() => {
      focusStripButton(rootRef.current?.querySelector<HTMLElement>('[data-dock-strip="right"] [data-tool-id]'));
    });
  };

  const back = () => {
    const refocus = focusInside();
    dockOnboardingStore.setStep(step - 1);
    if (!refocus) return;
    requestAnimationFrame(() => primaryRef.current?.focus({ preventScroll: true }));
  };

  if (placement === null) return null;
  return createPortal(
    <DockOnboardingCard
      step={step}
      titleId={titleId}
      bodyId={bodyId}
      placement={placement}
      onNext={() => dockOnboardingStore.setStep(step + 1)}
      onBack={back}
      onClose={close}
      cardRef={cardRef}
      primaryRef={primaryRef}
    />,
    document.body,
  );
}

export default function DockOnboarding({ rootRef }: DockOnboardingProps): React.ReactNode {
  const { open, step } = useDockOnboarding();

  useEffect(() => {
    const timer = window.setTimeout(() => dockOnboardingStore.autoShow(), DOCK_ONBOARDING_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, []);

  if (!open) return null;
  return <DockOnboardingPopup rootRef={rootRef} step={step} />;
}
