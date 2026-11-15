export type PanelNavigation =
  | 'previous-record'
  | 'next-record'
  | 'previous-kind'
  | 'next-kind'
  | 'previous-group'
  | 'next-group';
const plainNavigation: Readonly<Record<string, PanelNavigation>> = {
  ArrowUp: 'previous-record',
  ArrowDown: 'next-record',
  ArrowLeft: 'previous-kind',
  ArrowRight: 'next-kind',
};
const groupNavigation: Readonly<Record<string, PanelNavigation>> = {
  ArrowUp: 'previous-group',
  ArrowDown: 'next-group',
};
export function panelNavigation(event: KeyboardEvent): PanelNavigation | undefined {
  if (event.shiftKey || event.altKey || event.metaKey) return undefined;
  return (event.ctrlKey ? groupNavigation : plainNavigation)[event.key];
}
export function cycleFilter<T>(values: readonly T[], current: T, direction: number): T {
  return values[(values.indexOf(current) + direction + values.length) % values.length];
}
