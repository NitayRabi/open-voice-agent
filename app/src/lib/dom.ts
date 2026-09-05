// Typed `getElementById` helpers. The markup is shipped alongside these
// modules, so a missing id is a bug in this repo, not a runtime condition —
// hence the throw rather than a nullable return at every call site.

function byId<T extends HTMLElement>(id: string, ctor: abstract new () => T): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing element #${id}`);
  if (!(el instanceof ctor)) throw new Error(`#${id} is not a ${ctor.name}`);
  return el;
}

export const element = (id: string): HTMLElement => byId(id, HTMLElement);
export const input = (id: string): HTMLInputElement => byId(id, HTMLInputElement);
export const select = (id: string): HTMLSelectElement => byId(id, HTMLSelectElement);
export const textarea = (id: string): HTMLTextAreaElement => byId(id, HTMLTextAreaElement);
export const output = (id: string): HTMLOutputElement => byId(id, HTMLOutputElement);

/** A form control the settings screen reads a `value` from. */
export type ValueElement = HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement;

export function valueElement(id: string): ValueElement | null {
  const el = document.getElementById(id);
  if (!el) return null;
  if (el instanceof HTMLInputElement || el instanceof HTMLSelectElement || el instanceof HTMLTextAreaElement) {
    return el;
  }
  throw new Error(`#${id} is not a form control`);
}
