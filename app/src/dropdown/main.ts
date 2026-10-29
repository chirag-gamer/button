// Dropdown panel (FR-2, FR-44): the Recording Mode card, the switch list
// with sliding toggles (FR-27), and one detail page per switch for its
// off-timer (FR-28), Keep Awake durations (FR-18, D-081), the Bluetooth
// picker (FR-38), the backdrop look (FR-46) and Recording Mode membership
// (FR-40). Views are built once and patched on every state push, so the
// toggles animate and focus survives the once-a-second ticks.
import "./dropdown.css";
import "../shared/theme.css";
import "../shared/swatches.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AppState,
  BtDevice,
  OffAfter,
  Settings,
  SwitchId,
  extendRecording,
  fitDropdown,
  getSettings,
  getState,
  listBtDevices,
  onCountdown,
  onSettingsChanged,
  onStateChanged,
  openSettings,
  saveSettings,
  setBluetoothDevice,
  setKeepAwakeOptions,
  setSwitch,
  stopRecording,
  toggleRecording,
} from "../shared/ipc";
import {
  ACTION_IDS,
  BLUR_PRESETS,
  BUILTIN_BACKDROPS,
  GRADIENTS,
  LABELS,
  PRESET_COLOURS,
  UNAVAILABLE_LABELS,
  builtinThumb,
} from "../shared/catalog";
import { formatElapsed, formatLeft } from "../shared/format";
import { type IconName, icon } from "../shared/icons";

type SwitchState = AppState["switches"][number];

/** Tile glyph per switch (colours live in the CSS, D-098). */
const ICONS: Record<SwitchId, IconName> = {
  dnd: "moon",
  "desktop-icons": "squares-four",
  "other-apps": "browsers",
  taskbar: "taskbar",
  "tray-icons": "eye-slash",
  backdrop: "image",
  "keep-awake": "coffee",
  "alert-sounds": "bell",
  "dark-mode": "circle-half",
  "night-light": "sun-horizon",
  "screen-saver": "sparkle",
  microphone: "microphone",
  "lock-keyboard": "keyboard",
  bluetooth: "headphones",
};

/** One line under the detail page title: what the switch does. */
const DESCRIPTIONS: Record<SwitchId, string> = {
  dnd: "Silences notifications until you turn it off.",
  "desktop-icons": "Hides the icons on the desktop. They never move.",
  "other-apps": "Minimizes every window except the app you are on.",
  taskbar: "Turns on taskbar auto-hide.",
  "tray-icons": "Moves third-party tray icons into the overflow.",
  backdrop: "Covers the desktop behind the app you are on.",
  "keep-awake": "Stops the PC from sleeping. It sleeps as usual after.",
  "alert-sounds": "Mutes Windows system sounds; other audio keeps playing.",
  "dark-mode": "Switches Windows and apps to dark.",
  "night-light": "Warms the screen with Windows Night light.",
  "screen-saver": "Starts the screen saver now.",
  microphone: "Mutes the default microphone.",
  "lock-keyboard": "Swallows every key until you press Esc or click Unlock.",
  bluetooth: "Connects your chosen Bluetooth headphones.",
};

/** The off-timer chips (D-051); minutes go through `{ minutes }`. */
const TIMER_CHIPS: { label: string; title: string; value: OffAfter }[] = [
  { label: "5m", title: "5 minutes", value: { minutes: 5 } },
  { label: "15m", title: "15 minutes", value: "15m" },
  { label: "30m", title: "30 minutes", value: { minutes: 30 } },
  { label: "1h", title: "1 hour", value: "1h" },
  { label: "2h", title: "2 hours", value: { minutes: 120 } },
  { label: "4h", title: "4 hours", value: "4h" },
  { label: "Tomorrow", title: "Tomorrow at 8:00", value: "tomorrow" },
];

const KEEP_AWAKE_CHIPS: { label: string; minutes: number }[] = [
  { label: "5m", minutes: 5 },
  { label: "15m", minutes: 15 },
  { label: "30m", minutes: 30 },
  { label: "1h", minutes: 60 },
  { label: "2h", minutes: 120 },
  { label: "4h", minutes: 240 },
];

let state: AppState | undefined;
let settings: Settings | undefined;
let btDevices: BtDevice[] = [];
let showAll = false;
// The latest countdown tick (FR-41), shown while status is "countdown".
let countdownSeconds = 3;
// Switches with a click in flight: a second click waits for the answer.
const pending = new Set<SwitchId>();

const panel = document.getElementById("panel")!;
const viewMain = document.getElementById("view-main")!;
const viewDetail = document.getElementById("view-detail")!;
const card = document.getElementById("card")!;
const subText = document.getElementById("rm-sub-text")!;
const extendButton = document.getElementById("extend") as HTMLButtonElement;
const startRestore = document.getElementById(
  "start-restore",
) as HTMLButtonElement;
const rmConfig = document.getElementById("rm-config") as HTMLButtonElement;
const summary = document.getElementById("switch-summary")!;
const list = document.getElementById("list")!;
const showAllButton = document.getElementById("show-all") as HTMLButtonElement;
const openSettingsButton = document.getElementById(
  "open-settings",
) as HTMLButtonElement;
const switchError = document.getElementById("switch-error")!;
const hotkeyWarning = document.getElementById("hotkey-warning")!;

rmConfig.append(icon("chevron-right", 16));
openSettingsButton.append(icon("settings", 16));

let errorTimer: number | undefined;

/** FR-27's inline error surface: the message shows for 4 s. */
function showError(e: unknown): void {
  switchError.textContent = e instanceof Error ? e.message : String(e);
  switchError.hidden = false;
  window.clearTimeout(errorTimer);
  errorTimer = window.setTimeout(() => {
    switchError.hidden = true;
    fit();
  }, 4000);
  fit();
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function button(className: string, text?: string): HTMLButtonElement {
  const b = el("button", className, text);
  b.type = "button";
  return b;
}

function isAction(id: SwitchId): boolean {
  // Screen saver is the only engine action: it fires once, no on/off state.
  return id === "screen-saver";
}

/** The switch tile: a coloured rounded square with the white glyph. */
function tile(id: SwitchId, size: "row" | "hero"): HTMLElement {
  const t = el("span", `tile tile-${size} t-${id}`);
  t.setAttribute("aria-hidden", "true");
  t.append(icon(ICONS[id], size === "hero" ? 30 : 19));
  return t;
}

/** An iOS-style sliding switch: role=switch on a real button, so Enter
 * and Space work natively and the state is announced (spec 10). */
interface Toggle {
  el: HTMLButtonElement;
  set(on: boolean, disabled?: boolean): void;
}

function toggle(label: string, onClick: (next: boolean) => void): Toggle {
  const b = button("toggle");
  b.setAttribute("role", "switch");
  b.setAttribute("aria-label", label);
  b.setAttribute("aria-checked", "false");
  b.addEventListener("click", (e) => {
    e.stopPropagation(); // rows toggle on click too
    const next = b.getAttribute("aria-checked") !== "true";
    // Flip at once so the knob slides; the state push confirms or
    // flips it back.
    b.setAttribute("aria-checked", String(next));
    onClick(next);
  });
  return {
    el: b,
    set(on, disabled = false) {
      b.setAttribute("aria-checked", String(on));
      b.disabled = disabled;
    },
  };
}

function byId(id: SwitchId): SwitchState | undefined {
  return state?.switches.find((s) => s.id === id);
}

/** Recording Mode owns this switch right now (spec 7.3): timers are
 * refused until the session ends. */
function heldBySession(id: SwitchId): boolean {
  return (
    state?.session.status === "active" && state.session.applied.includes(id)
  );
}

function secondsLeft(at: string | undefined): number | undefined {
  if (!at) return undefined;
  const left = (Date.parse(at) - Date.now()) / 1000;
  return left > 0 ? left : undefined;
}

async function run(id: SwitchId, work: () => Promise<AppState>): Promise<void> {
  if (pending.has(id)) {
    render(); // undo the optimistic knob flip of the ignored click
    return;
  }
  pending.add(id);
  try {
    state = await work();
    if (id === "bluetooth") refreshBtSoon();
  } catch (e) {
    showError(e);
  } finally {
    pending.delete(id);
    render();
  }
}

/** A plain click: on/off; Keep Awake turns on for its default duration
 * (D-081); the screen saver fires. */
function flip(s: SwitchState, next: boolean): void {
  if (s.availability !== "available") return;
  void run(s.id, () => {
    if (s.id === "keep-awake" && next && settings) {
      return setSwitch(s.id, true, { minutes: settings.keepAwake.defaultMinutes });
    }
    return setSwitch(s.id, next);
  });
}

// ---- Main view: card + list ----

interface Row {
  el: HTMLElement;
  sub: HTMLElement;
  toggle?: Toggle;
  start?: HTMLButtonElement;
  chevron?: HTMLButtonElement;
}

const rows = new Map<SwitchId, Row>();
let rowOrder = "";

function makeRow(id: SwitchId): Row {
  const rowEl = el("div", "row");
  // ARIA: the row is a labelled group; the switch semantics live on the
  // toggle (or Start) at its end (spec 10).
  rowEl.setAttribute("role", "group");
  rowEl.setAttribute("aria-label", LABELS[id]);
  const text = el("span", "row-text");
  const name = el("span", "row-name", LABELS[id]);
  const sub = el("span", "row-sub");
  text.append(name, sub);
  rowEl.append(tile(id, "row"), text);

  const row: Row = { el: rowEl, sub };
  if (isAction(id)) {
    const start = button("start-chip", "Start");
    start.setAttribute("aria-label", `Start ${LABELS[id]}`);
    start.addEventListener("click", (e) => {
      e.stopPropagation();
      const s = byId(id);
      if (s) flip(s, true);
    });
    row.start = start;
    rowEl.append(start, el("span", "chevron-space"));
  } else {
    row.toggle = toggle(LABELS[id], (next) => {
      const s = byId(id);
      if (s) flip(s, next);
    });
    const chevron = button("chevron");
    chevron.id = `more-${id}`;
    chevron.setAttribute("aria-label", `${LABELS[id]} options`);
    chevron.title = "Options";
    chevron.append(icon("chevron-right", 16));
    chevron.addEventListener("click", (e) => {
      e.stopPropagation();
      openDetail(id);
    });
    row.chevron = chevron;
    rowEl.append(row.toggle.el, chevron);
  }
  // Clicking anywhere on the row toggles it (FR-27).
  rowEl.addEventListener("click", () => {
    const s = byId(id);
    if (!s || s.availability !== "available") return;
    if (isAction(id)) {
      flip(s, true);
      return;
    }
    row.toggle?.set(!s.on, false);
    flip(s, !s.on);
  });
  return row;
}

function rowSubtitle(s: SwitchState): { text: string; tone: string } {
  if (s.availability !== "available") {
    return { text: s.detail ?? UNAVAILABLE_LABELS[s.availability], tone: "warn" };
  }
  const left = secondsLeft(s.offAt);
  if (s.on && left !== undefined) {
    return { text: `Off in ${formatLeft(left)}`, tone: "" };
  }
  if (s.id === "bluetooth") {
    const chosen = settings?.bluetooth.deviceId;
    const device = btDevices.find((d) => d.id === chosen);
    if (!chosen) return { text: "Choose a device", tone: "warn" };
    if (!device) return { text: "Device not found", tone: "warn" };
    const battery = device.battery === null ? "" : ` · ${device.battery}%`;
    return { text: `${device.name}${battery}`, tone: "" };
  }
  if (s.experimental) return { text: "Experimental", tone: "" };
  return { text: "", tone: "" };
}

function updateRow(row: Row, s: SwitchState): void {
  const available = s.availability === "available";
  row.el.classList.toggle("unavailable", !available);
  const sub = rowSubtitle(s);
  row.sub.textContent = sub.text;
  row.sub.hidden = sub.text === "";
  row.sub.className = sub.tone ? `row-sub ${sub.tone}` : "row-sub";
  row.toggle?.set(s.on, !available || pending.has(s.id));
  if (row.start) row.start.disabled = !available;
}

function renderList(): void {
  if (!state || !settings) return;
  const ids = [
    ...settings.dropdown.shown,
    ...(showAll ? settings.dropdown.more : []),
  ].filter((id) => byId(id));
  for (const id of ids) {
    if (!rows.has(id)) rows.set(id, makeRow(id));
    updateRow(rows.get(id)!, byId(id)!);
  }
  // Re-insert only when the set or order changed: moving nodes on every
  // tick would cut the toggle transitions short.
  const order = ids.join(",");
  if (order !== rowOrder) {
    list.replaceChildren(...ids.map((id) => rows.get(id)!.el));
    rowOrder = order;
  }
  const more = settings.dropdown.more.filter((id) => byId(id)).length;
  showAllButton.hidden = more === 0;
  showAllButton.replaceChildren(
    icon("chevron-down", 16),
    document.createTextNode(showAll ? "Show less" : `Show ${more} more`),
  );
  showAllButton.setAttribute("aria-expanded", String(showAll));
}

function renderCard(): void {
  if (!state || !settings) return;
  const status = state.session.status;
  const active = status === "active";
  card.classList.toggle("recording", active || status === "countdown");
  extendButton.hidden = true;
  startRestore.disabled = status === "applying" || status === "restoring";
  startRestore.classList.toggle("grey", status !== "idle");

  const n = settings.recordingMode.actions.length;
  if (status === "countdown") {
    subText.textContent = `Starting in ${countdownSeconds}…`;
    startRestore.textContent = "Cancel";
  } else if (status === "applying") {
    subText.textContent = "Starting…";
    startRestore.textContent = "Start";
  } else if (status === "restoring") {
    subText.textContent = "Restoring…";
    startRestore.textContent = "Restore";
  } else if (active) {
    startRestore.textContent = "Restore";
    const left = secondsLeft(state.session.endsAt);
    if (state.session.endsAt) {
      // FR-42: remaining time and the extend chip (D-090).
      subText.textContent = `${formatElapsed(left ?? 0)} left`;
      extendButton.hidden = false;
    } else {
      const started = state.session.startedAt
        ? (Date.now() - Date.parse(state.session.startedAt)) / 1000
        : 0;
      subText.textContent = `On for ${formatElapsed(started)}`;
    }
  } else {
    subText.textContent = `${n} thing${n === 1 ? "" : "s"}, one click`;
    startRestore.textContent = "Start";
  }

  // Summary (FR-27): available, non-action switches only.
  const counted = state.switches.filter(
    (s) => s.availability === "available" && !isAction(s.id),
  );
  const on = counted.filter((s) => s.on).length;
  summary.textContent = `${on} on · ${counted.length - on} off`;

  // FR-9: warn when the hotkey could not be registered.
  hotkeyWarning.hidden = state.hotkey.registered;
  if (!state.hotkey.registered) {
    hotkeyWarning.textContent = `Hotkey ${state.hotkey.binding} could not be registered (in use by another app?)`;
  }
}

// ---- Detail view ----

interface Detail {
  id: SwitchId;
  update(): void;
}

let detail: Detail | undefined;

function chip(label: string, onClick: () => void, title?: string): HTMLButtonElement {
  const b = button("chip", label);
  if (title) b.title = title;
  b.addEventListener("click", onClick);
  return b;
}

function cardBox(...children: HTMLElement[]): HTMLElement {
  const box = el("div", "box");
  box.append(...children);
  return box;
}

function line(label: string, control: HTMLElement): HTMLElement {
  const l = el("div", "line");
  l.append(el("span", "line-label", label), control);
  return l;
}

/** The "In Recording Mode" line (FR-40): adds or removes the switch from
 * the actions, kept in apply order. */
function recordingLine(id: SwitchId): { el: HTMLElement; update(): void } | undefined {
  if (!ACTION_IDS.includes(id)) return undefined;
  const t = toggle("In Recording Mode", (on) => {
    saveChange((next) => {
      const chosen = new Set(next.recordingMode.actions);
      if (on) chosen.add(id);
      else chosen.delete(id);
      next.recordingMode.actions = ACTION_IDS.filter((a) => chosen.has(a));
    });
  });
  return {
    el: line("In Recording Mode", t.el),
    update: () => t.set(settings?.recordingMode.actions.includes(id) ?? false),
  };
}

/** Saves run one at a time, each built from the copy the previous one
 * returned, so two quick clicks never save from one stale snapshot (the
 * Settings window queues the same way). A settings-changed payload that
 * lands mid-save is newer than the save's return value and wins. */
let saveChain: Promise<void> = Promise.resolve();
let payloadRevision = 0;

function saveChange(edit: (next: Settings) => void): void {
  saveChain = saveChain.then(async () => {
    if (!settings) return;
    const next = structuredClone(settings);
    edit(next);
    const revision = payloadRevision;
    try {
      const saved = await saveSettings(next);
      if (payloadRevision === revision) settings = saved;
    } catch (e) {
      showError(e);
    }
    render();
  });
}

/** The "Turn off" chips (FR-28) for every timed switch but Keep Awake. */
function timerBox(id: SwitchId): { el: HTMLElement; update(): void } {
  const status = el("span", "line-value");
  const chips = el("div", "chips");
  // A chip turns the switch on with that timer, or moves an existing
  // timer (set_timer). There is no "no timer" chip: dropping a timer
  // entry would also drop the journaled state some switches need to
  // restore (Hide Other Apps' window placements).
  const buttons = TIMER_CHIPS.map((c) =>
    chip(
      c.label,
      () => void run(id, () => setSwitch(id, true, c.value)),
      c.value === "tomorrow" ? "Turn off tomorrow at 8:00" : `Turn off in ${c.title}`,
    ),
  );
  chips.append(...buttons);
  const head = el("div", "line");
  head.append(el("span", "line-label", "Turn off"), status);
  const box = cardBox(head, chips);
  return {
    el: box,
    update() {
      const s = byId(id);
      const held = heldBySession(id);
      const left = s?.on ? secondsLeft(s.offAt) : undefined;
      status.textContent = held
        ? "When Recording Mode stops"
        : left !== undefined
          ? `in ${formatLeft(left)}`
          : s?.on
            ? "When you turn it off"
            : "";
      const usable = s?.availability === "available" && !held;
      for (const b of buttons) b.disabled = !usable;
    },
  };
}

/** Keep Awake (FR-18, D-081): Timer / No end, the big duration readout
 * and the duration chips, plus "let the screen turn off". */
function keepAwakeBoxes(): { els: HTMLElement[]; update(): void } {
  const id: SwitchId = "keep-awake";
  let minutes = settings?.keepAwake.defaultMinutes ?? 120;
  let noEnd = false;

  const seg = el("div", "segmented");
  const timerSeg = button("seg");
  timerSeg.append(icon("timer", 16), document.createTextNode("Timer"));
  const noEndSeg = button("seg");
  noEndSeg.append(icon("timer-off", 16), document.createTextNode("No end"));
  seg.append(timerSeg, noEndSeg);

  const readout = el("div", "readout");
  const hours = el("span", "digits");
  const mins = el("span", "digits");
  const hBlock = el("div", "readout-block");
  hBlock.append(hours, el("span", "readout-unit", "Hours"));
  const mBlock = el("div", "readout-block");
  mBlock.append(mins, el("span", "readout-unit", "Min"));
  readout.append(hBlock, el("span", "readout-colon", ":"), mBlock);

  const chips = el("div", "chips centered");
  const chipButtons = KEEP_AWAKE_CHIPS.map((c) => {
    const b = chip(c.label, () => {
      minutes = c.minutes;
      noEnd = false;
      void run(id, () => setSwitch(id, true, { minutes: c.minutes }));
    });
    b.dataset.minutes = String(c.minutes);
    return b;
  });
  chips.append(...chipButtons);

  timerSeg.addEventListener("click", () => {
    noEnd = false;
    const s = byId(id);
    if (s?.on && !s.offAt) {
      void run(id, () => setSwitch(id, true, { minutes }));
    } else {
      render();
    }
  });
  noEndSeg.addEventListener("click", () => {
    noEnd = true;
    const s = byId(id);
    // On with a timer: plain "on" drops the timer (engine turn_on).
    if (s?.on && s.offAt) void run(id, () => setSwitch(id, true));
    else render();
  });

  const displayOff = toggle("Let the screen turn off", (on) => {
    setKeepAwakeOptions(on)
      .then((saved) => {
        settings = saved;
      })
      .catch(showError)
      .finally(render);
  });

  const durationBox = cardBox(seg, readout, chips);
  const optionBox = cardBox(line("Let the screen turn off", displayOff.el));
  heroToggleOverride = (next) => {
    void run(id, () =>
      next && !noEnd ? setSwitch(id, true, { minutes }) : setSwitch(id, next),
    );
  };
  return {
    els: [durationBox, optionBox],
    update() {
      const s = byId(id);
      const left = s?.on ? secondsLeft(s.offAt) : undefined;
      if (s?.on) noEnd = left === undefined;
      const held = heldBySession(id);
      timerSeg.setAttribute("aria-pressed", String(!noEnd));
      noEndSeg.setAttribute("aria-pressed", String(noEnd));
      durationBox.classList.toggle("no-end", noEnd);
      const shown = left !== undefined ? Math.ceil(left / 60) : minutes;
      hours.textContent = String(Math.floor(shown / 60)).padStart(2, "0");
      mins.textContent = String(shown % 60).padStart(2, "0");
      readout.classList.toggle("live", left !== undefined);
      for (const b of chipButtons) {
        b.setAttribute(
          "aria-pressed",
          String(!noEnd && left === undefined && Number(b.dataset.minutes) === minutes),
        );
        b.disabled = held || s?.availability !== "available";
      }
      timerSeg.disabled = held;
      noEndSeg.disabled = held;
      displayOff.set(settings?.keepAwake.allowDisplayOff ?? false);
    },
  };
}

/** The Bluetooth picker (FR-38): paired audio devices, the chosen one
 * checked. The list refreshes while the page is open. */
function bluetoothBox(): { el: HTMLElement; update(): void } {
  const listBox = el("div", "picker");
  const box = cardBox(el("span", "box-title", "Device"), listBox);
  let shownKey = "";
  void listBtDevices()
    .then((devices) => {
      btDevices = devices;
      render();
    })
    .catch(() => {
      /* keep the cached list; errors surface on the toggle click */
    });
  return {
    el: box,
    update() {
      const chosen = settings?.bluetooth.deviceId ?? null;
      const key = JSON.stringify([btDevices, chosen]);
      if (key === shownKey) return;
      shownKey = key;
      listBox.replaceChildren();
      if (btDevices.length === 0) {
        listBox.append(el("span", "empty", "No paired Bluetooth audio devices"));
      }
      for (const device of btDevices) {
        const item = button("pick");
        item.setAttribute("aria-pressed", String(device.id === chosen));
        const name = el("span", "pick-name", device.name);
        const meta = el(
          "span",
          "pick-meta",
          [device.connected ? "Connected" : "", device.battery === null ? "" : `${device.battery}%`]
            .filter(Boolean)
            .join(" · "),
        );
        const check = el("span", "pick-check");
        if (device.id === chosen) check.append(icon("checkmark", 16));
        item.append(name, meta, check);
        item.addEventListener("click", () => {
          setBluetoothDevice(device.id)
            .then((saved) => {
              settings = saved;
            })
            .catch(showError)
            .finally(render);
        });
        listBox.append(item);
      }
    },
  };
}

type Look = "wallpaper" | "builtin" | "color" | "gradient";

/** The backdrop look (FR-46): Wallpaper / Built-in / Colour / Gradient,
 * the matching tiles, and the blur presets. Pictures and dim stay in
 * Settings. Every pick saves through save_settings (the single writer),
 * which restyles a backdrop on screen. */
function backdropBox(): { el: HTMLElement; update(): void } {
  const looks: { id: Look; label: string }[] = [
    { id: "wallpaper", label: "Wallpaper" },
    { id: "builtin", label: "Built-in" },
    { id: "color", label: "Colour" },
    { id: "gradient", label: "Gradient" },
  ];
  const seg = el("div", "segmented");
  const segButtons = looks.map((look) => {
    const b = button("seg", look.label);
    b.addEventListener("click", () => {
      saveChange((next) => {
        next.backdrop.style = look.id;
        if (look.id === "builtin") next.backdrop.builtin ??= BUILTIN_BACKDROPS[0].id;
        if (look.id === "gradient") next.backdrop.gradient ??= GRADIENTS[0].id;
      });
    });
    seg.append(b);
    return b;
  });
  const tiles = el("div", "tiles");
  const note = el("p", "box-note");
  const blurSeg = el("div", "segmented small");
  const blurButtons = BLUR_PRESETS.map((p) => {
    const b = button("seg", p.label);
    b.title = `${p.px} px`;
    b.addEventListener("click", () =>
      saveChange((next) => {
        next.backdrop.blur = p.px;
      }),
    );
    blurSeg.append(b);
    return b;
  });
  const blurLine = line("Blur", blurSeg);
  const box = cardBox(seg, tiles, note, blurLine);
  let tilesFor = "";

  function pickTile(className: string, label: string, pressed: boolean, onClick: () => void): HTMLButtonElement {
    const b = button(`look-tile ${className}`);
    b.setAttribute("aria-label", label);
    b.title = label;
    b.setAttribute("aria-pressed", String(pressed));
    b.addEventListener("click", onClick);
    return b;
  }

  return {
    el: box,
    update() {
      if (!settings) return;
      const b = settings.backdrop;
      const look = (["wallpaper", "builtin", "color", "gradient"] as Look[]).find(
        (l) => l === b.style,
      );
      segButtons.forEach((sb, i) =>
        sb.setAttribute("aria-pressed", String(looks[i].id === look)),
      );
      const key = JSON.stringify([b.style, b.builtin, b.color, b.gradient]);
      if (key !== tilesFor) {
        tilesFor = key;
        tiles.replaceChildren();
        tiles.className = `tiles tiles-${b.style}`;
        if (b.style === "builtin") {
          for (const pic of BUILTIN_BACKDROPS) {
            const t = pickTile("pic", pic.name, b.builtin === pic.id, () =>
              saveChange((next) => {
                next.backdrop.style = "builtin";
                next.backdrop.builtin = pic.id;
              }),
            );
            const img = el("img");
            img.src = builtinThumb(pic.id);
            img.alt = "";
            img.draggable = false;
            t.append(img);
            tiles.append(t);
          }
        } else if (b.style === "color") {
          for (const c of PRESET_COLOURS) {
            tiles.append(
              pickTile(`swatch ${c.className}`, c.name, b.color.toLowerCase() === c.value, () =>
                saveChange((next) => {
                  next.backdrop.style = "color";
                  next.backdrop.color = c.value;
                }),
              ),
            );
          }
        } else if (b.style === "gradient") {
          for (const g of GRADIENTS) {
            tiles.append(
              pickTile(`gradient-tile gradient-${g.id}`, `${g.name} gradient`, b.gradient === g.id, () =>
                saveChange((next) => {
                  next.backdrop.style = "gradient";
                  next.backdrop.gradient = g.id;
                }),
              ),
            );
          }
        }
      }
      tiles.hidden = tiles.childElementCount === 0;
      note.textContent =
        b.style === "wallpaper"
          ? "Your Windows wallpaper, without the desktop icons."
          : b.style === "picture"
            ? "Your own picture. Change it in Settings."
            : "";
      note.hidden = note.textContent === "";
      // Blur softens pictures only; colours and gradients are flat.
      const picture = b.style === "wallpaper" || b.style === "builtin" || b.style === "picture";
      blurLine.hidden = !picture;
      blurButtons.forEach((bb, i) =>
        bb.setAttribute("aria-pressed", String(BLUR_PRESETS[i].px === b.blur)),
      );
    },
  };
}

// Keep Awake's hero toggle honours the page's Timer / No end choice.
let heroToggleOverride: ((next: boolean) => void) | undefined;

function buildDetail(id: SwitchId): Detail {
  heroToggleOverride = undefined;
  const back = button("back");
  back.id = "detail-back";
  back.append(icon("chevron-left", 16), document.createTextNode("button"));
  back.setAttribute("aria-label", "Back to the switches");
  back.addEventListener("click", closeDetail);

  const title = el("h2", "hero-title", LABELS[id]);
  title.id = "detail-title";
  const hero = el("div", "hero");
  const heroToggle = toggle(LABELS[id], (next) => {
    const s = byId(id);
    if (!s || s.availability !== "available") return;
    if (heroToggleOverride) heroToggleOverride(next);
    else flip(s, next);
  });
  hero.append(tile(id, "hero"), title, heroToggle.el);
  const desc = el("p", "hero-desc", DESCRIPTIONS[id]);
  const reason = el("p", "hero-reason");

  const parts: { update(): void }[] = [];
  const boxes: HTMLElement[] = [];
  if (id === "keep-awake") {
    const ka = keepAwakeBoxes();
    boxes.push(...ka.els);
    parts.push(ka);
  } else if (id === "bluetooth") {
    const bt = bluetoothBox();
    boxes.push(bt.el);
    parts.push(bt);
  } else if (id === "backdrop") {
    const bd = backdropBox();
    boxes.push(bd.el);
    parts.push(bd);
  }
  // Levers groups the timer and "In Recording Mode" in one card; Keep
  // Awake's timer is its duration card, so the line joins its options.
  let last: HTMLElement;
  if (id === "keep-awake") {
    last = boxes[boxes.length - 1];
  } else {
    const timer = timerBox(id);
    boxes.push(timer.el);
    parts.push(timer);
    last = timer.el;
  }
  const rec = recordingLine(id);
  if (rec) {
    last.append(el("div", "divider"), rec.el);
    parts.push(rec);
  }

  const settingsLink = button("foot-link");
  settingsLink.append(document.createTextNode("More in Settings"), icon("chevron-right", 16));
  // The settings that go deeper than this page: the backdrop picture and
  // dim, the Stays on Screen apps, the shelves and per-switch options.
  const section =
    id === "backdrop" ? "backdrop" : id === "other-apps" ? "recording" : "switches";
  settingsLink.addEventListener("click", () => {
    void openSettings(section);
  });
  const foot = el("footer", "footer");
  foot.append(el("span", "spacer"), settingsLink);

  viewDetail.replaceChildren(back, hero, desc, reason, ...boxes, foot);
  viewDetail.setAttribute("aria-labelledby", "detail-title");

  return {
    id,
    update() {
      const s = byId(id);
      if (!s) return;
      const available = s.availability === "available";
      heroToggle.set(s.on, !available || pending.has(id));
      reason.hidden = available;
      reason.textContent = available
        ? ""
        : (s.detail ?? UNAVAILABLE_LABELS[s.availability]);
      for (const p of parts) p.update();
    },
  };
}

function slideIn(view: HTMLElement, from: "left" | "right"): void {
  view.classList.remove("enter-left", "enter-right");
  view.classList.add(from === "right" ? "enter-right" : "enter-left");
  void view.offsetWidth; // start from the offset position
  view.classList.remove("enter-left", "enter-right");
}

function openDetail(id: SwitchId): void {
  detail = buildDetail(id);
  viewMain.hidden = true;
  viewDetail.hidden = false;
  viewDetail.scrollTop = 0;
  render();
  slideIn(viewDetail, "right");
  document.getElementById("detail-back")?.focus();
}

function closeDetail(): void {
  if (!detail) return;
  const id = detail.id;
  detail = undefined;
  heroToggleOverride = undefined;
  viewDetail.hidden = true;
  viewDetail.replaceChildren();
  viewMain.hidden = false;
  render();
  slideIn(viewMain, "left");
  document.getElementById(`more-${id}`)?.focus();
}

// ---- Render + window fit ----

function render(): void {
  if (!state || !settings) return;
  renderCard();
  renderList();
  detail?.update();
  fit();
}

let fittedTo = 0;

/** Size the window to the open view's natural height (fit_dropdown keeps
 * the taskbar-side edge in place). Measured with the scroll areas
 * unclamped; no paint happens in between. */
function fit(): void {
  document.body.classList.add("measuring");
  const height = Math.ceil(panel.getBoundingClientRect().height);
  document.body.classList.remove("measuring");
  if (Math.abs(height - fittedTo) < 1) return;
  fittedTo = height;
  fitDropdown(height).catch((e: unknown) => console.error(e));
}

// ---- Wiring ----

startRestore.addEventListener("click", async () => {
  try {
    state =
      state?.session.status === "countdown"
        ? await stopRecording() // FR-41: cancelling the countdown is a Stop
        : await toggleRecording();
  } catch (e) {
    // The state changed under us (e.g. session started elsewhere):
    // refresh instead of erroring (FR-2 has no error UI).
    console.error(e);
    state = await getState();
  }
  render();
});

// FR-42: push the time limit back 15 minutes.
extendButton.addEventListener("click", async () => {
  try {
    state = await extendRecording(15);
  } catch (e) {
    console.error(e);
    state = await getState();
  }
  render();
});

rmConfig.addEventListener("click", () => {
  void openSettings("recording");
});

showAllButton.addEventListener("click", () => {
  showAll = !showAll;
  render();
});

// FR-39: the settings window opens next to the dropdown; the dropdown
// hides on the blur that follows.
openSettingsButton.addEventListener("click", () => {
  void openSettings();
});

// Drag the panel by its background, like the Win+V panel: anything that
// is not a control starts a window move (data-tauri-drag-region would
// need the attribute on every child).
panel.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  const target = e.target as Element;
  if (target.closest("button, a, input, select, label, .row, .picker")) return;
  void getCurrentWindow().startDragging();
});

/** Connecting settles over seconds: refresh the device list a few times
 * after a connect/disconnect click (mirrors the core's row refreshes). */
function refreshBtSoon(): void {
  for (const delay of [3000, 8000]) {
    window.setTimeout(() => {
      void listBtDevices()
        .then((devices) => {
          btDevices = devices;
          render();
        })
        .catch(() => {
          /* the list keeps its last value; errors surfaced on click */
        });
    }, delay);
  }
}

// FR-41: the countdown ticks arrive as events.
void onCountdown((secondsLeft) => {
  countdownSeconds = secondsLeft;
  if (state?.session.status === "countdown") render();
});

// Blur hides the whole dropdown (tray.rs): the next open starts on the
// switch list, like a fresh menu.
window.addEventListener("blur", () => {
  if (detail) closeDetail();
});

// Escape leaves a detail page first; on the list it hides the dropdown
// (standard popover behaviour, spec 10).
window.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (detail) {
    closeDetail();
    return;
  }
  void getCurrentWindow().hide();
});

Promise.all([getState(), getSettings(), listBtDevices().catch(() => [])]).then(
  ([initialState, initialSettings, devices]) => {
    state = initialState;
    settings = initialSettings;
    btDevices = devices;
    render();
  },
);
void onStateChanged((next) => {
  state = next;
  render();
});
void onSettingsChanged((next) => {
  payloadRevision += 1;
  settings = next;
  render();
});
