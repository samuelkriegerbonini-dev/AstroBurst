import { useAnnouncement } from "./useDockDrag";

export default function LiveRegion(): React.JSX.Element {
  const { text, seq } = useAnnouncement();
  return (
    <div data-dock-live="" aria-live="polite" aria-atomic="true" className="sr-only">
      {text && <span key={seq}>{text}</span>}
    </div>
  );
}
