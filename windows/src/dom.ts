// mimic · © 2026 Jay · https://github.com/ag0nyy-ops/mimic · MIT, see LICENSE
//
// A tiny hyperscript helper plus the icons. No framework needed for this ui.

type Child = Node | string | number | null | false | undefined;
type Props = Record<string, unknown> | null;

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props?: Props,
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props ?? {})) {
    if (value === undefined || value === null || value === false) continue;
    if (key === "class") el.className = String(value);
    else if (key === "html") el.innerHTML = String(value); // only ever used with the static icons below
    else if (key.startsWith("on") && typeof value === "function") {
      el.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
    } else if (value === true) el.setAttribute(key, "");
    else if (key in el && key !== "list" && key !== "type") (el as unknown as Record<string, unknown>)[key] = value;
    else el.setAttribute(key, String(value));
  }
  append(el, children);
  return el;
}

export function append(el: Element, children: Child[]): void {
  for (const c of children) {
    if (c === null || c === false || c === undefined) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
}

export function clear(el: Element): void {
  while (el.firstChild) el.removeChild(el.firstChild);
}

const svg = (inner: string, size = 18) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${inner}</svg>`;

export const icons = {
  play: (s?: number) => svg('<polygon points="7 4 20 12 7 20 7 4" fill="currentColor"/>', s),
  stop: (s?: number) => svg('<rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor"/>', s),
  plus: (s?: number) => svg('<path d="M12 5v14M5 12h14"/>', s),
  copy: (s?: number) => svg('<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V6a2 2 0 0 1 2-2h9"/>', s),
  trash: (s?: number) => svg('<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v5M14 11v5"/>', s),
  sliders: (s?: number) =>
    svg('<path d="M4 6h8M18 6h2M4 12h2M12 12h8M4 18h10M20 18h0"/><circle cx="15" cy="6" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="17" cy="18" r="2"/>', s),
  search: (s?: number) => svg('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>', s),
  x: (s?: number) => svg('<path d="M6 6l12 12M18 6 6 18"/>', s),
  info: (s?: number) => svg('<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>', s),
  refresh: (s?: number) => svg('<path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7"/>', s),
  swap: (s?: number) => svg('<path d="M7 4 3 8l4 4M3 8h14M17 20l4-4-4-4M21 16H7"/>', s),
  alert: (s?: number) => svg('<path d="M12 4 2.5 20h19L12 4zM12 10v4M12 17h.01"/>', s),
  check: (s?: number) => svg('<path d="m5 12 5 5 9-10"/>', s),
  link: (s?: number) => svg('<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>', s),
  gamepad: (s?: number) =>
    svg('<path d="M6 11h4M8 9v4"/><circle cx="15.5" cy="12" r=".7" fill="currentColor"/><circle cx="18" cy="10" r=".7" fill="currentColor"/><path d="M17.3 5H6.7a4 4 0 0 0-3.9 3.2L2 15a3 3 0 0 0 5.2 2.3L9 15h6l1.8 2.3A3 3 0 0 0 22 15l-.8-6.8A4 4 0 0 0 17.3 5z"/>', s),
};

export const icon = (name: keyof typeof icons, size?: number): HTMLSpanElement =>
  h("span", { class: "icon", html: icons[name](size) });
