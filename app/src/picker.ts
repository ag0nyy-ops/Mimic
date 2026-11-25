// mimic · © 2026 Jay · https://github.com/ag0nyy-ops/mimic · source viewable, not reusable: see LICENSE
//
// The searchable game picker, shown as a modal.

import type { Game } from "./api";
import { h, icon } from "./dom";

export interface PickerOptions {
  title: string;
  total: number;
  search: (query: string) => Promise<Game[]>;
}

export function pickGame(opts: PickerOptions): Promise<Game | null> {
  return new Promise((resolve) => {
    let results: Game[] = [];
    let active = 0;
    let seq = 0;
    let debounce: number | undefined;

    const list = h("div", { class: "pick-list", role: "listbox" });
    const count = h("div", { class: "label pick-count" });
    const input = h("input", {
      class: "input pick-input",
      type: "text",
      placeholder: "Search games",
      "aria-label": "Search games",
      autocomplete: "off",
      spellcheck: false,
    });

    const close = (game: Game | null) => {
      document.removeEventListener("keydown", onKey, true);
      overlay.remove();
      resolve(game);
    };

    const paint = () => {
      list.replaceChildren(
        ...(results.length
          ? results.map((g, i) =>
              h(
                "button",
                {
                  class: "pick-row" + (i === active ? " active" : ""),
                  type: "button",
                  role: "option",
                  onclick: () => close(g),
                  onmousemove: () => {
                    if (active !== i) {
                      active = i;
                      list.querySelector(".active")?.classList.remove("active");
                      list.children[i]?.classList.add("active");
                    }
                  },
                },
                h("span", { class: "pick-name" }, g.name),
                h("span", { class: "pick-id" }, g.id),
              ),
            )
          : [h("div", { class: "pick-empty" }, "Nothing on file matches that. Try fewer words, or refresh the list in settings.")]),
      );
      list.scrollTop = 0;
    };

    const run = async () => {
      const mine = ++seq;
      const found = await opts.search(input.value);
      if (mine !== seq) return; // a newer search is already on its way
      results = found;
      active = 0;
      count.textContent = input.value.trim()
        ? `${found.length}${found.length >= 50 ? "+" : ""} match${found.length === 1 ? "" : "es"}`
        : `${opts.total.toLocaleString()} games on file`;
      paint();
    };

    const scrollActive = () => list.children[active]?.scrollIntoView({ block: "nearest" });
    const move = (delta: number) => {
      if (!results.length) return;
      list.children[active]?.classList.remove("active");
      active = (active + delta + results.length) % results.length;
      list.children[active]?.classList.add("active");
      scrollActive();
    };

    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close(null);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        move(1);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        move(-1);
      } else if (e.key === "Enter" && results[active]) {
        e.preventDefault();
        close(results[active]);
      }
    }

    input.addEventListener("input", () => {
      window.clearTimeout(debounce);
      debounce = window.setTimeout(run, 60);
    });
    document.addEventListener("keydown", onKey, true);

    const overlay = h(
      "div",
      {
        class: "overlay",
        onmousedown: (e: MouseEvent) => {
          if (e.target === overlay) close(null);
        },
      },
      h(
        "div",
        { class: "modal picker", role: "dialog", "aria-label": opts.title },
        h(
          "div",
          { class: "modal-head" },
          h("div", null, h("div", { class: "label" }, "File search"), h("h2", null, opts.title)),
          h("button", { class: "icon-btn", type: "button", title: "Close", "aria-label": "Close", onclick: () => close(null) }, icon("x")),
        ),
        h("div", { class: "pick-search" }, icon("search", 16), input),
        count,
        list,
        h(
          "div",
          { class: "pick-foot" },
          "Application ids come from discord's own list, mimic never makes them up.",
        ),
      ),
    );
    document.body.append(overlay);
    input.focus();
    void run();
  });
}
