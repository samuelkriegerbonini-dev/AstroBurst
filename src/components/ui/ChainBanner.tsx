import { memo } from "react";
import { ArrowRight } from "lucide-react";

interface ChainBannerProps {
  chainedFrom: string | null | undefined;
  accent?: string;
}

function ChainBanner({ chainedFrom, accent = "teal" }: ChainBannerProps) {
  if (!chainedFrom) return null;

  return (
    <div className="ab-chain-banner" data-accent={accent}>
      <ArrowRight size={10} />
      Using output from{" "}
      <span className="font-medium">{chainedFrom}</span>
    </div>
  );
}

export default memo(ChainBanner);
