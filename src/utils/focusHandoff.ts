export interface FocusHandoff {
  leaving: (node: HTMLElement | null) => (() => void) | undefined;
  arriving: (node: HTMLElement | null) => void;
}

type FocusDocument = Pick<Document, "activeElement" | "body">;

const currentDocument = (): FocusDocument | null => (typeof document === "undefined" ? null : document);

export function createFocusHandoff(doc: () => FocusDocument | null = currentDocument): FocusHandoff {
  let pending = false;
  return {
    leaving: (node) => {
      if (node === null) return undefined;
      return () => {
        if (doc()?.activeElement === node) pending = true;
      };
    },
    arriving: (node) => {
      if (node === null || !pending) return;
      pending = false;
      const current = doc();
      if (current !== null && (current.activeElement === null || current.activeElement === current.body)) node.focus();
    },
  };
}
