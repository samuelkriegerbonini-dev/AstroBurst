import { memo } from "react";

interface ResultItem {
  label: string;
  value: string | number | undefined | null;
}

interface ResultGridProps {
  items: ResultItem[];
  columns?: 2 | 3 | 4;
}

const COL_CLASS: Record<number, string> = {
  2: "grid-cols-2",
  3: "grid-cols-3",
  4: "grid-cols-4",
};

function ResultGrid({ items, columns = 3 }: ResultGridProps) {
  return (
    <div className={`grid ${COL_CLASS[columns]} gap-2 text-xs`}>
      {items.map((item) => (
        <div key={item.label} className="ab-metric-card">
          <div className="text-zinc-500 text-[10px]">{item.label}</div>
          <div className="text-zinc-200 font-mono text-[11px]">{item.value ?? "--"}</div>
        </div>
      ))}
    </div>
  );
}

export default memo(ResultGrid);
