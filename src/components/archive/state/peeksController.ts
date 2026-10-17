import type { MastMissionId, MastPeek, MastProduct } from "../../../shared/types/mast";
import { mastErrorOf, mastPeek } from "../../../services/mast";
import { peekKey, peekRequestId, peekUri } from "../logic/peekView";
import { peeksStore, type PeeksStore } from "./peeksStore";

export interface PeekDeps {
  peek: (requestId: string, mission: MastMissionId, datasetId: string, filename: string, uri: string | null) => Promise<MastPeek>;
}

export interface PeeksController {
  fetch(mission: MastMissionId, datasetId: string, product: MastProduct): Promise<void>;
}

export function createPeeksController(store: PeeksStore, deps: PeekDeps): PeeksController {
  let attempts = 0;
  return {
    async fetch(mission, datasetId, product) {
      const key = peekKey(mission, datasetId, product.filename);
      const attempt = ++attempts;
      store.dispatch({ type: "started", key, mission, datasetId, filename: product.filename, attempt });
      try {
        const peek = await deps.peek(peekRequestId(datasetId, product.filename), mission, product.datasetId, product.filename, peekUri(product));
        store.dispatch({ type: "loaded", key, attempt, peek });
      } catch (err) {
        const error = mastErrorOf(err);
        store.dispatch(error.code === "mast_cancelled" ? { type: "dropped", key, attempt } : { type: "failed", key, attempt, error });
      }
    },
  };
}

export const peeksController = createPeeksController(peeksStore, { peek: mastPeek });
