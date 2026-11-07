import { isValidElement, type ReactElement, type ReactNode } from "react";

type AnyProps = Record<string, unknown> & { children?: ReactNode };

export function elementsOf<P = AnyProps>(node: unknown): ReactElement<P>[] {
  if (Array.isArray(node)) return node.flatMap((child) => elementsOf<P>(child));
  if (!isValidElement<P & AnyProps>(node)) return [];
  return [node, ...elementsOf<P>(node.props.children)];
}

export function findElement<P = AnyProps>(node: unknown, predicate: (element: ReactElement<P>) => boolean): ReactElement<P> | undefined {
  return elementsOf<P>(node).find(predicate);
}

export function textOf(node: unknown): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<AnyProps>(node) ? textOf(node.props.children) : "";
}
