import { describe, expect, it } from "vitest";
import ts from "typescript";

const SOURCES = import.meta.glob<string>("../../**/*.tsx", { query: "?raw", import: "default", eager: true });
const ESCAPE = /\\u[0-9a-fA-F]{4}|\\u\{[0-9a-fA-F]+\}/;

function literalEscapesIn(fileName: string, text: string): string[] {
  const source = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const found: string[] = [];
  const report = (node: ts.Node) => {
    const line = source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1;
    found.push(`${fileName}:${line}`);
  };
  const visit = (node: ts.Node) => {
    if (ts.isJsxText(node) && ESCAPE.test(node.getText(source))) report(node);
    if (ts.isJsxAttribute(node) && node.initializer && ts.isStringLiteral(node.initializer) && ESCAPE.test(node.initializer.getText(source))) {
      report(node);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return found;
}

describe("JSX text and attribute strings", () => {
  it("scans the component sources", () => {
    expect(Object.keys(SOURCES).length).toBeGreaterThan(100);
  });

  it("contain no \\u escapes, which JSX shows literally instead of the character", () => {
    const offenders = Object.entries(SOURCES).flatMap(([name, text]) => literalEscapesIn(name, text));
    expect(offenders).toEqual([]);
  });
});
