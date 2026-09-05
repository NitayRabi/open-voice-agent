/** `Error.message` when there is one, otherwise whatever the value stringifies to. */
export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
