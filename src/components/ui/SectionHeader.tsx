import { memo } from "react";

interface SectionHeaderProps {
  icon: React.ReactNode;
  title: string;
  subtitle?: string;
  truncateSubtitle?: boolean;
  actions?: React.ReactNode;
}

function SectionHeader({ icon, title, subtitle, truncateSubtitle = false, actions }: SectionHeaderProps) {
  return (
    <div className="flex items-center gap-2 mb-1">
      {icon}
      <h3 className="text-sm font-semibold text-zinc-200 tracking-wide">{title}</h3>
      {subtitle && (truncateSubtitle ? (
        <span className="text-[10px] text-zinc-500 ml-auto truncate" title={subtitle}>{subtitle}</span>
      ) : (
        <span className="text-[10px] text-zinc-500 ml-auto">{subtitle}</span>
      ))}
      {actions && <span className={subtitle ? "flex items-center gap-1 shrink-0" : "flex items-center gap-1 shrink-0 ml-auto"}>{actions}</span>}
    </div>
  );
}

export default memo(SectionHeader);
