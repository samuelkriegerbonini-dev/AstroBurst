import { memo } from "react";

interface SectionHeaderProps {
  icon: React.ReactNode;
  title: string;
  subtitle?: string;
}

function SectionHeader({ icon, title, subtitle }: SectionHeaderProps) {
  return (
    <div className="flex items-center gap-2 mb-1">
      {icon}
      <span className="text-sm font-semibold text-zinc-200 tracking-wide">{title}</span>
      {subtitle && <span className="text-[10px] text-zinc-500 ml-auto">{subtitle}</span>}
    </div>
  );
}

export default memo(SectionHeader);
