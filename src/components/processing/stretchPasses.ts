import type { MaskedStretchChannelStats } from "../../shared/types/processing";

export const ITERATIONS_LABEL = "Iterations";
export const ITERATIONS_HINT = "More steps reach the same background with stronger star protection";

const CONVERGED = "(converged)";
const SHORT_OF_TARGET = "(stopped before the target)";

export interface StretchPassesInput {
  iterations_run?: number;
  converged?: boolean;
  channels?: { r: MaskedStretchChannelStats; g: MaskedStretchChannelStats; b: MaskedStretchChannelStats };
}

export interface StretchPassesSummary {
  text: string;
  converged: boolean;
}

export function stretchPassesSummary(res: StretchPassesInput): StretchPassesSummary | null {
  if (res.channels) {
    const channels = [["R", res.channels.r], ["G", res.channels.g], ["B", res.channels.b]] as const;
    const counts = channels.map(([label, c]) => `${label} ${c.iterations_run}`).join(" · ");
    const short = channels.filter(([, c]) => !c.converged).map(([label]) => label);
    const status = short.length === 0
      ? CONVERGED
      : short.length === channels.length
        ? SHORT_OF_TARGET
        : `(stopped before the target: ${short.join(", ")})`;
    return { text: `Stretch passes: ${counts} ${status}`, converged: short.length === 0 };
  }
  if (res.iterations_run == null) return null;
  const converged = res.converged === true;
  return { text: `Stretch passes: ${res.iterations_run} ${converged ? CONVERGED : SHORT_OF_TARGET}`, converged };
}
