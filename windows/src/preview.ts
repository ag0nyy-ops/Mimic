// The live preview: a card that imitates discord's "Playing a game" activity.

import type { Preset } from "./api";
import { h, icon } from "./dom";

export function elapsedStart(preset: Preset, baseUnix: number): number | null {
  switch (preset.timer.mode) {
    case "none":
      return null;
    case "now":
      return baseUnix;
    case "offset":
      return Math.max(0, baseUnix - Math.max(0, Math.floor(preset.timer.offsetSeconds)));
  }
}

export function formatElapsed(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const pad = (n: number) => String(n).padStart(2, "0");
  const hours = Math.floor(s / 3600);
  const minutes = Math.floor((s % 3600) / 60);
  const seconds = s % 60;
  return hours > 0 ? `${pad(hours)}:${pad(minutes)}:${pad(seconds)}` : `${pad(minutes)}:${pad(seconds)}`;
}

const text = (s: string) => s.trim();

export class PreviewCard {
  readonly el = h("div", { class: "activity-card" });
  private start: number | null = null;
  private timeEl: HTMLElement | null = null;
  private timer: number | undefined;

  constructor() {
    this.timer = window.setInterval(() => this.tick(), 1000);
  }

  destroy(): void {
    window.clearInterval(this.timer);
  }

  /** `baseUnix` anchors "start now" and "offset" timers (when the preset went live). */
  update(preset: Preset, baseUnix: number): void {
    const details = text(preset.details);
    const state = text(preset.state);
    const party = preset.party && preset.party.max >= 1 ? ` (${preset.party.current} of ${preset.party.max})` : "";
    const hasLarge = text(preset.assets.largeImage) !== "";
    const hasSmall = text(preset.assets.smallImage) !== "";
    this.start = elapsedStart(preset, baseUnix);

    const art = h(
      "div",
      { class: "ac-art" },
      h(
        "div",
        {
          class: "ac-large" + (hasLarge ? " has-key" : ""),
          title: hasLarge ? `large image key: ${preset.assets.largeImage.trim()}` : "no large image key set",
        },
        icon("gamepad", 28),
      ),
      hasSmall
        ? h("div", { class: "ac-small", title: `small image key: ${preset.assets.smallImage.trim()}` }, icon("gamepad", 11))
        : null,
    );

    this.timeEl = this.start !== null ? h("div", { class: "ac-time" }, h("span", { class: "ac-dot" }), h("span")) : null;

    const lines = h(
      "div",
      { class: "ac-text" },
      h("div", { class: "ac-name" }, preset.game.name),
      details ? h("div", { class: "ac-line" }, details) : null,
      state || party ? h("div", { class: "ac-line" }, `${state}${party}`.trim()) : null,
      this.timeEl,
    );

    const buttons = preset.buttons.filter((b) => text(b.label) !== "").slice(0, 2);

    this.el.replaceChildren(
      h("div", { class: "ac-head" }, "Playing a game"),
      h("div", { class: "ac-body" }, art, lines),
      ...(buttons.length ? [h("div", { class: "ac-buttons" }, ...buttons.map((b) => h("div", { class: "ac-btn" }, text(b.label))))] : []),
    );
    this.tick();
  }

  private tick(): void {
    if (this.start === null || !this.timeEl) return;
    const label = this.timeEl.lastElementChild;
    if (label) label.textContent = `${formatElapsed(Date.now() / 1000 - this.start)} elapsed`;
  }
}
