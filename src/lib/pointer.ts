/**
 * macOS turns Control+click into a secondary ("right") click. With a
 * Control-based shortcut held, every click on Sedjem's controls would become
 * a context click and WebKit never fires `click`. Treat it as a primary press.
 */
export function isPrimaryPress(event: { button: number; ctrlKey: boolean }): boolean {
  return event.button === 0 || (event.button === 2 && event.ctrlKey);
}
