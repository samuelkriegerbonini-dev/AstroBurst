const AD_HOC_ERROR_TEXT = /\btext-red-400(?:\/\d+)?(?![\w-])/;
const AD_HOC_ERROR_FILL = /\bbg-red-900\/(?:15|20)(?![\w-])/;
const LOCAL_SMALL_BUTTON_CLASS = /^\s*const SMALL_BUTTON_CLASS\s*=/;

function matchingLines(source: string, offends: (line: string) => boolean): number[] {
  return source.split(/\r?\n/).flatMap((line, i) => (offends(line) ? [i + 1] : []));
}

export function adHocErrorBoxLines(source: string): number[] {
  return matchingLines(source, (line) => AD_HOC_ERROR_TEXT.test(line) && AD_HOC_ERROR_FILL.test(line));
}

export function localSmallButtonLines(source: string): number[] {
  return matchingLines(source, (line) => LOCAL_SMALL_BUTTON_CLASS.test(line));
}
