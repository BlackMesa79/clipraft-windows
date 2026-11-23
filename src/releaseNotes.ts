const MAX_SUMMARY_LENGTH = 4_000;
const UPDATE_HEADING = '更新内容';
const HEADING = /^ {0,3}(#{1,6})[ \t]+(.+?)[ \t]*#*[ \t]*$/;
const FENCE = /^ {0,3}(`{3,}|~{3,})/;

function* visibleLines(markdown: string): Generator<string> {
  let fence = '';
  for (const line of markdown.split(/\r?\n/)) {
    const marker = FENCE.exec(line)?.[1];
    if (!marker) {
      if (!fence) yield line;
      continue;
    }
    if (!fence) fence = marker;
    else if (marker.startsWith(fence) && line.trim() === marker) fence = '';
  }
}

export function releaseSummary(markdown: string): string {
  const summary: string[] = [];
  let sectionLevel = 0;
  for (const line of visibleLines(markdown)) {
    const heading = HEADING.exec(line);
    if (sectionLevel) {
      if (heading && heading[1].length <= sectionLevel) break;
      summary.push(line);
      continue;
    }
    if (heading?.[2] === UPDATE_HEADING) sectionLevel = heading[1].length;
  }
  const result = summary.join('\n').trim();
  return result.length > MAX_SUMMARY_LENGTH ? result.slice(0, MAX_SUMMARY_LENGTH) + '…' : result;
}
