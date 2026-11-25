export type PanelNavigation =
  | 'previous-record'
  | 'next-record'
  | 'first-record'
  | 'last-record'
  | 'previous-page'
  | 'next-page'
  | 'previous-kind'
  | 'next-kind'
  | 'previous-group'
  | 'next-group';
const plainNavigation: Readonly<Record<string, PanelNavigation>> = {
  ArrowUp: 'previous-record',
  ArrowDown: 'next-record',
  Home: 'first-record',
  End: 'last-record',
  PageUp: 'previous-page',
  PageDown: 'next-page',
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
