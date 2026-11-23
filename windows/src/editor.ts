// mimic · © 2026 Jay · https://github.com/ag0nyy-ops/mimic · MIT, see LICENSE
//
// The preset editor form. It edits one draft preset in place and tells the
// caller whenever something changed.

import type { ButtonInput, Preset, Timer } from "./api";
import { h, icon } from "./dom";

export interface EditorHooks {
  onChange: (draft: Preset) => void;
  onChangeGame: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
}

/** How many button rows the form always shows. */
const BUTTON_ROWS = 2;

export function padButtons(buttons: ButtonInput[]): ButtonInput[] {
  const rows = buttons.slice(0, BUTTON_ROWS).map((b) => ({ ...b }));
  while (rows.length < BUTTON_ROWS) rows.push({ label: "", url: "" });
  return rows;
}

/**
 * The form always shows two button rows, but only filled rows are saved (the
 * shared schema doesn't allow blank ones). `rowMap[i]` is the form row that
 * saved button `i` came from, so errors can be mapped back to the right inputs.
 */
export function serializeForSave(draft: Preset): { preset: Preset; rowMap: number[] } {
  const rowMap: number[] = [];
  const buttons: ButtonInput[] = [];
  draft.buttons.forEach((b, row) => {
    if (b.label.trim() === "" && b.url.trim() === "") return;
    rowMap.push(row);
    buttons.push({ label: b.label, url: b.url });
  });
  return { preset: { ...draft, buttons }, rowMap };
}

export function mapErrorField(field: string | null, rowMap: number[]): string | null {
  if (!field) return null;
  const m = /^buttons\.(\d+)\.(label|url)$/.exec(field);
  if (!m) return field;
  return `buttons.${rowMap[Number(m[1])] ?? Number(m[1])}.${m[2]}`;
}

interface FieldRef {
  group: HTMLElement;
  error: HTMLElement;
  input?: HTMLElement;
}

export class Editor {
  readonly head = h("div", { class: "editor-head" });
  readonly body = h("div", { class: "editor-body" });

  private draft: Preset | null = null;
  private fields = new Map<string, FieldRef>();
  private loaders: Array<(p: Preset) => void> = [];
  private nameInput!: HTMLInputElement;
  private gameName!: HTMLElement;
  private gameChip!: HTMLElement;
  private caseLabel = h("div", { class: "label" });

  constructor(private hooks: EditorHooks) {
    this.buildHead();
    this.buildBody();
  }

  load(preset: Preset, caseNo: number): void {
    this.draft = preset;
    this.caseLabel.textContent = `Case ${String(caseNo).padStart(3, "0")} / Preset`;
    for (const fn of this.loaders) fn(preset);
    this.setError(null, null);
  }

  /** Mark one field (e.g. `buttons.1.url`) as invalid, or clear all marks. */
  setError(field: string | null, message: string | null): void {
    for (const ref of this.fields.values()) {
      ref.group.classList.remove("invalid");
      ref.error.textContent = "";
      ref.input?.classList.remove("invalid");
    }
    if (!field || !message) return;
    const ref = this.fields.get(field) ?? this.fields.get(field.replace(/\.(label|url)$/, ""));
    if (!ref) return;
    ref.group.classList.add("invalid");
    ref.error.textContent = message;
    ref.input?.classList.add("invalid");
  }

  // ---- building blocks

  private touch(mutate: (p: Preset) => void): void {
    if (!this.draft) return;
    mutate(this.draft);
    this.hooks.onChange(this.draft);
  }

  private register(key: string, group: HTMLElement, error: HTMLElement, input?: HTMLElement): void {
    this.fields.set(key, { group, error, input });
  }

  private text(opts: {
    key: string;
    label: string;
    placeholder?: string;
    max: number;
    get: (p: Preset) => string;
    set: (p: Preset, v: string) => void;
    counter?: boolean;
    type?: string;
  }): HTMLElement {
    const input = h("input", {
      class: "input",
      type: opts.type ?? "text",
      placeholder: opts.placeholder ?? "",
      maxLength: opts.max,
      autocomplete: "off",
      spellcheck: false,
    });
    const counter = h("span", { class: "counter" });
    const error = h("div", { class: "field-error" });
    const group = h(
      "div",
      { class: "field" },
      h("label", null, h("span", null, opts.label), opts.counter ? counter : null),
      input,
      error,
    );
    const paint = () => {
      counter.textContent = `${input.value.length}/${opts.max}`;
    };
    input.addEventListener("input", () => {
      paint();
      this.touch((p) => opts.set(p, input.value));
    });
    this.loaders.push((p) => {
      input.value = opts.get(p);
      paint();
    });
    this.register(opts.key, group, error, input);
    return group;
  }

  /** `label` is the small mono tag; the "FILE 0n /" number in front of it comes from css. */
  private card(label: string, title: string, subtitle: string | null, ...children: Array<HTMLElement | null>): HTMLElement {
    return h(
      "section",
      { class: "card", "aria-label": title },
      h("div", { class: "card-head" }, h("div", { class: "label" }, label), h("h2", null, title), subtitle ? h("p", null, subtitle) : null),
      ...children,
    );
  }

  private notice(...content: Array<string | Node>): HTMLElement {
    return h("div", { class: "notice" }, icon("info", 16), h("div", null, ...content));
  }

  // ---- head: preset name + game

  private buildHead(): void {
    this.nameInput = h("input", {
      class: "name-input",
      type: "text",
      maxLength: 64,
      placeholder: "Preset name",
      spellcheck: false,
      "aria-label": "Preset name",
    });
    this.nameInput.addEventListener("input", () => this.touch((p) => (p.name = this.nameInput.value)));

    this.gameName = h("span", { class: "game-chip-name" });
    this.gameChip = h(
      "button",
      { class: "game-chip", type: "button", onclick: () => this.hooks.onChangeGame() },
      icon("gamepad", 15),
      this.gameName,
      h("span", { class: "game-chip-swap" }, icon("swap", 12), "Change"),
    );

    this.head.append(
      h(
        "div",
        { class: "head-main" },
        this.caseLabel,
        this.nameInput,
        this.gameChip,
      ),
      h(
        "div",
        { class: "head-actions" },
        h("button", { class: "icon-btn", type: "button", title: "Duplicate preset", "aria-label": "Duplicate preset", onclick: () => this.hooks.onDuplicate() }, icon("copy")),
        h("button", { class: "icon-btn danger", type: "button", title: "Delete preset", "aria-label": "Delete preset", onclick: () => this.hooks.onDelete() }, icon("trash")),
      ),
    );

    this.loaders.push((p) => {
      this.nameInput.value = p.name;
      this.gameName.textContent = p.game.name;
      this.gameChip.title = `Change the game (discord application id ${p.game.id})`;
    });
  }

  // ---- body sections

  private buildBody(): void {
    this.body.append(this.textSection(), this.imageSection(), this.timerSection(), this.partySection(), this.buttonSection());
  }

  private textSection(): HTMLElement {
    return this.card(
      "Details",
      "What you're doing",
      null,
      this.text({
        key: "details",
        label: "Details",
        placeholder: "e.g. Competitive, ranked match",
        max: 128,
        counter: true,
        get: (p) => p.details,
        set: (p, v) => (p.details = v),
      }),
      this.text({
        key: "state",
        label: "State",
        placeholder: "e.g. In queue",
        max: 128,
        counter: true,
        get: (p) => p.state,
        set: (p, v) => (p.state = v),
      }),
    );
  }

  private imageSection(): HTMLElement {
    const row = (a: HTMLElement, b: HTMLElement) => h("div", { class: "row2" }, a, b);
    return this.card(
      "Images",
      "Images",
      "Asset keys and their hover text",
      this.notice(
        h("strong", null, "Image keys only show up if this game's discord application owns an asset with that exact name."),
        " For games from discord's list that application belongs to the game's developer, so made-up keys usually show nothing. Whether a full image url works as a key over ipc is untested.",
      ),
      row(
        this.text({
          key: "assets.largeImage",
          label: "Large image key",
          placeholder: "asset key",
          max: 256,
          get: (p) => p.assets.largeImage,
          set: (p, v) => (p.assets.largeImage = v),
        }),
        this.text({
          key: "assets.largeText",
          label: "Large image hover text",
          max: 128,
          get: (p) => p.assets.largeText,
          set: (p, v) => (p.assets.largeText = v),
        }),
      ),
      row(
        this.text({
          key: "assets.smallImage",
          label: "Small image key",
          placeholder: "asset key",
          max: 256,
          get: (p) => p.assets.smallImage,
          set: (p, v) => (p.assets.smallImage = v),
        }),
        this.text({
          key: "assets.smallText",
          label: "Small image hover text",
          max: 128,
          get: (p) => p.assets.smallText,
          set: (p, v) => (p.assets.smallText = v),
        }),
      ),
    );
  }

  private timerSection(): HTMLElement {
    const modes: Array<{ mode: Timer["mode"]; label: string }> = [
      { mode: "none", label: "No timer" },
      { mode: "now", label: "Start now" },
      { mode: "offset", label: "Custom offset" },
    ];
    const buttons = modes.map((m) =>
      h("button", { class: "seg-btn", type: "button", "data-mode": m.mode, onclick: () => choose(m.mode) }, m.label),
    );
    const num = (label: string, max: number) => {
      const input = h("input", { class: "input num", type: "number", min: 0, max, step: 1, inputMode: "numeric" });
      return { input, group: h("div", { class: "field narrow" }, h("label", null, h("span", null, label)), input) };
    };
    const hours = num("Hours", 9999);
    const minutes = num("Minutes", 59);
    const seconds = num("Seconds", 59);
    const offsetRow = h(
      "div",
      { class: "offset-row" },
      hours.group,
      minutes.group,
      seconds.group,
      h("p", { class: "hint" }, "Shows this much time as already elapsed, e.g. 1h 30m reads 01:30:00 straight away."),
    );

    const readOffset = () => {
      const n = (el: HTMLInputElement) => Math.max(0, Math.floor(Number(el.value) || 0));
      return n(hours.input) * 3600 + Math.min(59, n(minutes.input)) * 60 + Math.min(59, n(seconds.input));
    };
    const paintMode = (mode: Timer["mode"]) => {
      for (const b of buttons) b.classList.toggle("active", b.dataset.mode === mode);
      offsetRow.hidden = mode !== "offset";
    };
    const choose = (mode: Timer["mode"]) => {
      paintMode(mode);
      this.touch((p) => {
        p.timer = mode === "offset" ? { mode, offsetSeconds: readOffset() } : { mode };
      });
    };
    for (const el of [hours.input, minutes.input, seconds.input]) {
      el.addEventListener("input", () => this.touch((p) => (p.timer = { mode: "offset", offsetSeconds: readOffset() })));
    }
    this.loaders.push((p) => {
      paintMode(p.timer.mode);
      const total = p.timer.mode === "offset" ? p.timer.offsetSeconds : 0;
      hours.input.value = String(Math.floor(total / 3600));
      minutes.input.value = String(Math.floor((total % 3600) / 60));
      seconds.input.value = String(total % 60);
    });

    return this.card("Timer", "Elapsed timer", null, h("div", { class: "seg" }, ...buttons), offsetRow);
  }

  private partySection(): HTMLElement {
    const toggle = h("input", { type: "checkbox", id: "party-toggle" });
    const current = h("input", { class: "input num", type: "number", min: 1, step: 1 });
    const max = h("input", { class: "input num", type: "number", min: 1, step: 1 });
    const error = h("div", { class: "field-error" });
    const fields = h(
      "div",
      { class: "party-fields" },
      h("div", { class: "field narrow" }, h("label", null, h("span", null, "In party")), current),
      h("span", { class: "of" }, "of"),
      h("div", { class: "field narrow" }, h("label", null, h("span", null, "Max size")), max),
    );
    const group = h("div", { class: "party" }, fields, error);

    const read = () => ({
      current: Math.max(0, Math.floor(Number(current.value) || 0)),
      max: Math.max(0, Math.floor(Number(max.value) || 0)),
    });
    const paint = (on: boolean) => {
      fields.hidden = !on;
    };
    toggle.addEventListener("change", () => {
      paint(toggle.checked);
      if (toggle.checked && !current.value) {
        current.value = "1";
        max.value = "4";
      }
      this.touch((p) => (p.party = toggle.checked ? read() : null));
    });
    for (const el of [current, max]) el.addEventListener("input", () => this.touch((p) => (p.party = read())));
    this.loaders.push((p) => {
      toggle.checked = p.party !== null;
      current.value = p.party ? String(p.party.current) : "";
      max.value = p.party ? String(p.party.max) : "";
      paint(p.party !== null);
    });
    this.register("party", group, error);

    return this.card(
      "Party",
      "Party size",
      null,
      h("label", { class: "switch-row", for: "party-toggle" }, toggle, h("span", { class: "switch" }), h("span", null, "Show a party size, like “2 of 4”")),
      group,
    );
  }

  private buttonSection(): HTMLElement {
    const rows = Array.from({ length: BUTTON_ROWS }, (_, i) => {
      const label = h("input", { class: "input", type: "text", placeholder: "Button label", maxLength: 32, autocomplete: "off", spellcheck: false });
      const url = h("input", { class: "input", type: "text", placeholder: "https://…", maxLength: 512, autocomplete: "off", spellcheck: false });
      const labelError = h("div", { class: "field-error" });
      const urlError = h("div", { class: "field-error" });
      const labelGroup = h("div", { class: "field" }, label, labelError);
      const urlGroup = h("div", { class: "field" }, url, urlError);
      const sync = () =>
        this.touch((p) => {
          p.buttons[i] = { label: label.value, url: url.value };
        });
      label.addEventListener("input", sync);
      url.addEventListener("input", sync);
      this.loaders.push((p) => {
        label.value = p.buttons[i]?.label ?? "";
        url.value = p.buttons[i]?.url ?? "";
      });
      this.register(`buttons.${i}.label`, labelGroup, labelError, label);
      this.register(`buttons.${i}.url`, urlGroup, urlError, url);
      return h("div", { class: "button-row" }, h("span", { class: "button-num" }, String(i + 1)), labelGroup, urlGroup);
    });

    const generalError = h("div", { class: "field-error" });
    const group = h("div", null, generalError);
    this.register("buttons", group, generalError);

    return this.card(
      "Buttons",
      "Buttons",
      "Up to two link buttons",
      this.notice(
        h("strong", null, "Buttons are only visible to other people."),
        " Discord never shows them on your own profile, so don't worry when you can't see them yourself.",
      ),
      ...rows,
      group,
    );
  }
}
