export function containDialogFocus(
  element: HTMLDialogElement | undefined,
  event: KeyboardEvent,
): void {
  if (event.key !== 'Tab') return;
  const controls = Array.from(
    element?.querySelectorAll<HTMLElement>('input:not(:disabled), button:not(:disabled)') ?? [],
  );
  const first = controls[0];
  const last = controls.at(-1);
  if (!first || !last) {
    event.preventDefault();
    return;
  }
  const boundary = event.shiftKey ? first : last;
  if (
    document.activeElement === boundary ||
    !controls.includes(document.activeElement as HTMLElement)
  ) {
    event.preventDefault();
    (event.shiftKey ? last : first).focus();
  }
}
