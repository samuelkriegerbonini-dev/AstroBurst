import { AlertTriangle, Clock, X } from "lucide-react";
import type { Banner as BannerModel } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";

interface BannerProps {
  banner: BannerModel;
  ctl: Pick<ArchiveController, "retryThumbs" | "resumeThumbs" | "search" | "loadMore" | "dismissBanner" | "setFilters">;
}

function retryLabel(banner: BannerModel): string | null {
  switch (banner.retry) {
    case "thumbs":
      return "Retry failed";
    case "resume":
      return "Resume now";
    case "search":
      return banner.kind === "rate-limited" ? "Retry now" : "Try again";
    case "page":
      return "Load again";
    default:
      return null;
  }
}

export default function Banner({ banner, ctl }: BannerProps) {
  const label = retryLabel(banner);
  const onRetry = () => {
    if (banner.retry === "thumbs") ctl.retryThumbs();
    else if (banner.retry === "resume") ctl.resumeThumbs();
    else if (banner.retry === "search") void ctl.search();
    else if (banner.retry === "page") void ctl.loadMore();
  };
  const Icon = banner.kind === "rate-limited" ? Clock : AlertTriangle;
  return (
    <div data-testid="mast-banner" data-kind={banner.kind} className="ab-mast-banner" role="status">
      <Icon size={13} aria-hidden="true" />
      <span className="ab-mast-banner-text">{banner.text}</span>
      {label ? (
        <button type="button" className="ab-mast-link-btn" onClick={onRetry}>
          {label}
          {banner.retry === "thumbs" || banner.retry === "search" ? <kbd className="ab-cmdp-kbd">R</kbd> : null}
        </button>
      ) : null}
      {banner.allLevels ? (
        <button type="button" data-testid="mast-banner-all-levels" className="ab-mast-link-btn" onClick={() => ctl.setFilters({ level: "all" }, { rerun: true })}>
          Search all levels
        </button>
      ) : null}
      <button type="button" className="ab-mast-icon-btn" aria-label="Dismiss" title="Dismiss" onClick={() => ctl.dismissBanner(banner.kind)}>
        <X size={12} aria-hidden="true" />
      </button>
    </div>
  );
}
