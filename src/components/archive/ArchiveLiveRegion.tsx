interface ArchiveLiveRegionProps {
  live: { text: string; seq: number };
}

export default function ArchiveLiveRegion({ live }: ArchiveLiveRegionProps) {
  return (
    <div data-testid="mast-live" aria-live="polite" aria-atomic="true" className="sr-only">
      {live.text ? <span key={live.seq}>{live.text}</span> : null}
    </div>
  );
}
