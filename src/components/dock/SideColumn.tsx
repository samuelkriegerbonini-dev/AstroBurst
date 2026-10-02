import { useCallback } from "react";
import type { SideAnchor } from "../../utils/dockLayout";

export interface SideColumnProps {
  anchor: SideAnchor;
  open: boolean;
  width: string;
  label: string;
  outerRef: React.Ref<HTMLDivElement>;
  innerRef: React.Ref<HTMLDivElement>;
  containerRef: React.Ref<HTMLDivElement>;
  onSettled(anchor: SideAnchor): void;
}

export default function SideColumn({ anchor, open, width, label, outerRef, innerRef, containerRef, onSettled }: SideColumnProps) {
  const left = anchor === "left-top";
  const handleTransitionEnd = useCallback((e: React.TransitionEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget || e.propertyName !== "width") return;
    onSettled(anchor);
  }, [anchor, onSettled]);

  return (
    <div
      ref={outerRef}
      className="shrink-0 relative overflow-hidden ab-panel-anim-w"
      style={{ width: open ? width : 0 }}
      aria-hidden={!open}
      onTransitionEnd={handleTransitionEnd}
    >
      <div
        ref={innerRef}
        inert={!open}
        role="region"
        aria-label={`${label} panel`}
        className={`absolute inset-y-0 ${left ? "right-0" : "left-0"} flex flex-col overflow-hidden`}
        style={{
          width,
          [left ? "borderRight" : "borderLeft"]: "1px solid var(--ab-border)",
          background: "rgba(5,5,16,0.55)",
        }}
      >
        <div ref={containerRef} data-dock-anchor={anchor} className="relative h-full overflow-hidden" />
      </div>
    </div>
  );
}
