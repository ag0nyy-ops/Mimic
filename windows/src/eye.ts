// mimic · © 2026 Jay · https://github.com/ag0nyy-ops/mimic · MIT, see LICENSE
//
// The eye is built entirely in css (see .eye in style.css). This file only
// creates the markup and feeds css the numbers it can't know: the cursor
// position, where each eye sits, and whether the visitor has gone idle.
// It animates nothing itself.

import { h } from "./dom";

/** One eye. `blink` is its blink period in seconds, `delay` offsets it so no two eyes blink together. */
export function eye(variant: string, blink: number, delay: number, lashes = true): HTMLElement {
  const el = h(
    "span",
    { class: `eye ${variant}`, "aria-hidden": "true" },
    h(
      "span",
      { class: "eye__ball" },
      h("span", { class: "eye__iris" }, h("span", { class: "eye__fibers" }), h("span", { class: "eye__pupil" })),
      h("span", { class: "eye__lid eye__lid--top" }),
      h("span", { class: "eye__lid eye__lid--bottom" }),
    ),
    lashes ? h("span", { class: "eye__lashes" }) : null,
  );
  el.style.setProperty("--jdx-blink", `${blink}s`);
  el.style.setProperty("--jdx-blink-delay", `${delay}s`);
  return el;
}

const IDLE_AFTER_MS = 25_000;

export function startEyes(): () => void {
  const root = document.documentElement;
  const calm = window.matchMedia("(prefers-reduced-motion: reduce)");
  let raf = 0;
  let x = 0;
  let y = 0;
  let idle: number | undefined;

  const publish = () => {
    raf = 0;
    root.style.setProperty("--mx", String(Math.round(x)));
    root.style.setProperty("--my", String(Math.round(y)));
    for (const el of document.querySelectorAll<HTMLElement>(".eye")) {
      const r = el.getBoundingClientRect();
      el.style.setProperty("--jdx-ex", String(Math.round(r.left + r.width / 2)));
      el.style.setProperty("--jdx-ey", String(Math.round(r.top + r.height / 2)));
    }
  };

  const wake = () => {
    delete root.dataset.idle;
    window.clearTimeout(idle);
    idle = window.setTimeout(() => {
      root.dataset.idle = "asleep";
    }, IDLE_AFTER_MS);
  };

  const onMove = (e: PointerEvent) => {
    if (calm.matches) return;
    x = e.clientX;
    y = e.clientY;
    wake();
    if (!raf) raf = requestAnimationFrame(publish);
  };
  const onKey = () => wake();

  window.addEventListener("pointermove", onMove, { passive: true });
  window.addEventListener("keydown", onKey);
  wake();

  return () => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("keydown", onKey);
    window.clearTimeout(idle);
  };
}
