export default function HeaderTable({ header }) {
  if (!header) return null;

  const entries = Object.entries(header).filter(
    ([key]) => key !== "FILENAME"
  );

  return (
    <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 animate-fade-in">
      {entries.map(([key, value]) => (
        <div key={key} className="contents">
          <span className="text-xs font-mono text-zinc-500 text-right">
            {key}
          </span>
          <span className="text-xs text-zinc-300 truncate">{value}</span>
        </div>
      ))}
    </div>
  );
}
