// The Settings window (FR-39..FR-46, spec 5.1 and 6.1). One nav plus one
// section per area. Every control change builds the FULL next settings
// object from the current copy, saves it through `save_settings`, and
// replaces the copy with the returned (normalised) settings — the core is
// the single writer, so a partial object is never sent.
import "./settings.css";
import "../shared/theme.css";
import { getVersion } from "@tauri-apps/api/app";
import { eventToAccelerator } from "../shared/accel";
import {
  AppInfo,
  AppState,
  Availability,
  BtDevice,
  GradientPreset,
  IpcError,
  Settings,
  SwitchId,
  TrayIconInfo,
  getSettings,
  getState,
  listApps,
  listBtDevices,
  listTrayIcons,
  onSettingsChanged,
  onStateChanged,
  openOnboarding,
  pickBackdropPicture,
  saveSettings,
} from "../shared/ipc";
import { addApp, isExeName, moveApp, removeApp } from "./apps";
import { moveBetween, reorder, Shelves } from "./shelves";

// Display names mirror the dropdown's LABELS (spec 7.5 catalogue).
const LABELS: Record<SwitchId, string> = {
  dnd: "Do Not Disturb",
  "desktop-icons": "Hide Desktop Icons",
  "other-apps": "Hide Other Apps",
  taskbar: "Hide Taskbar",
  "tray-icons": "Hide Tray Icons",
  backdrop: "Backdrop",
  "keep-awake": "Keep Awake",
  "alert-sounds": "Mute Alert Sounds",
  "dark-mode": "Dark Mode",
  "night-light": "Night Light",
  "screen-saver": "Screen Saver",
  microphone: "Mute Microphone",
  "lock-keyboard": "Lock Keyboard",
  bluetooth: "Bluetooth Headphones",
};

const UNAVAILABLE_LABELS: Record<Availability, string> = {
  available: "",
  "needs-windows-11": "Needs Windows 11",
  "probe-failed": "Unavailable",
  "not-built": "Not built yet",
};

/** The Recording Mode actions in spec 7.1 apply order (FR-40). */
const ACTION_IDS: SwitchId[] = [
  "dnd",
  "alert-sounds",
  "keep-awake",
  "other-apps",
  "backdrop",
  "desktop-icons",
  "taskbar",
  "tray-icons",
  "dark-mode",
  "night-light",
];

/** Every switch in the spec 5.1 catalogue order (FR-44, FR-45). */
const ALL_SWITCHES: SwitchId[] = [
  "dnd",
  "desktop-icons",
  "other-apps",
  "taskbar",
  "tray-icons",
  "backdrop",
  "keep-awake",
  "alert-sounds",
  "dark-mode",
  "night-light",
  "screen-saver",
  "microphone",
  "lock-keyboard",
  "bluetooth",
];

/** Time-limit choices (FR-42), label + minutes; "Off" is null. */
const TIME_LIMITS: { minutes: number; label: string }[] = [
  { minutes: 1, label: "1 minute" },
  { minutes: 5, label: "5 minutes" },
  { minutes: 15, label: "15 minutes" },
  { minutes: 30, label: "30 minutes" },
  { minutes: 60, label: "1 hour" },
  { minutes: 90, label: "90 minutes" },
  { minutes: 120, label: "2 hours" },
];

/** Gradient presets (D-087) with the names the tiles announce. */
const GRADIENTS: { id: GradientPreset; name: string }[] = [
  { id: "dusk", name: "Dusk" },
  { id: "ocean", name: "Ocean" },
  { id: "ember", name: "Ember" },
  { id: "forest", name: "Forest" },
  { id: "violet", name: "Violet" },
  { id: "slate", name: "Slate" },
];

/** Preset colours (FR-46) plus the custom picker. The fills live in
 * settings.css: inline styles are banned by the CSP (D-082). */
const PRESET_COLOURS: { value: string; name: string; className: string }[] = [
  { value: "#202020", name: "Charcoal", className: "swatch-charcoal" },
  { value: "#000000", name: "Black", className: "swatch-black" },
  { value: "#ffffff", name: "White", className: "swatch-white" },
  { value: "#3a3d45", name: "Slate", className: "swatch-slate" },
  { value: "#1e4d3b", name: "Forest", className: "swatch-forest" },
  { value: "#4b2a80", name: "Plum", className: "swatch-plum" },
];

const SHELF_LABELS: Record<keyof Shelves, string> = {
  shown: "Shown",
  more: "Show all",
  hidden: "Hidden",
};

type AppShelf = keyof Settings["apps"];
type BackdropStyle = Settings["backdrop"]["style"];

let settings: Settings | undefined;
let state: AppState | undefined;
let btDevices: BtDevice[] = [];
let runningApps: AppInfo[] = [];
let trayIcons: TrayIconInfo[] | undefined;
let trayIconsError: string | undefined;

/** The binding row listening for a keypress (FR-45): "recordingMode" or
 * "switch:<id>". Undefined when no capture is running. */
let capturing: string | undefined;

function byId<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`missing element #${id}`);
  return found as T;
}

function make<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}

/** A button with an id (focus survives a re-render), a class and an
 * accessible name (spec 10). */
function makeButton(
  id: string,
  label: string,
  className: string,
  ariaLabel: string,
): HTMLButtonElement {
  const button = make("button", className, label);
  button.id = id;
  button.type = "button";
  button.setAttribute("aria-label", ariaLabel);
  return button;
}

function message(e: unknown): string {
  if (e instanceof IpcError) return e.message;
  return e instanceof Error ? e.message : String(e);
}

// ---- Window chrome ----

const navItems = Array.from(
  document.querySelectorAll<HTMLButtonElement>(".nav-item"),
);
const sections = Array.from(document.querySelectorAll<HTMLElement>(".section"));

/** Switch sections (FR-39): the open nav item carries aria-current. */
function showSection(sectionId: string): void {
  for (const item of navItems) {
    if (item.getAttribute("aria-controls") === sectionId) {
      item.setAttribute("aria-current", "page");
    } else {
      item.removeAttribute("aria-current");
    }
  }
  for (const section of sections) {
    section.hidden = section.id !== sectionId;
  }
}

for (const item of navItems) {
  item.addEventListener("click", () => {
    const target = item.getAttribute("aria-controls");
    if (target) showSection(target);
  });
}

// ---- The one error region (role=alert) ----

const errorBox = byId<HTMLDivElement>("error");

function showError(text: string | undefined): void {
  errorBox.hidden = text === undefined;
  errorBox.textContent = text ?? "";
}

// ---- Saving ----

/** Saves run one at a time, and a settings-changed payload from another
 * writer (dropdown, tray, onboarding) always wins over a save's own
 * return value: `payloadRevision` counts those payloads. */
let saveChain: Promise<void> = Promise.resolve();
let payloadRevision = 0;

/** Edit the current settings, save the whole object, keep the returned
 * (normalised) copy. The edit runs when its turn comes, built from the copy
 * the previous save returned; returning `false` refuses the save (an
 * invalid entry) and the control re-reads the stored value. A failed save
 * shows the message and re-renders from the last good copy, so the UI never
 * drifts from the file. */
function saveChange(edit: (next: Settings) => void | false): Promise<void> {
  saveChain = saveChain.then(async () => {
    if (!settings) return;
    const next = JSON.parse(JSON.stringify(settings)) as Settings;
    if (edit(next) === false) {
      render();
      return;
    }
    const revision = payloadRevision;
    try {
      const saved = await saveSettings(next);
      // A payload that landed mid-save is newer: keep it.
      if (payloadRevision === revision) settings = saved;
      showError(undefined);
    } catch (e) {
      showError(message(e));
    }
    render();
  });
  return saveChain;
}

// ---- Elements ----

const startWithWindows = byId<HTMLInputElement>("start-with-windows");
const checkUpdates = byId<HTMLInputElement>("check-updates");
const onboardingButtons = [
  byId<HTMLButtonElement>("show-onboarding-general"),
  byId<HTMLButtonElement>("show-onboarding-about"),
];

const actionsBox = byId<HTMLDivElement>("actions");
const countdownBox = byId<HTMLInputElement>("countdown");
const timeLimit = byId<HTMLSelectElement>("time-limit");
const appLists: Record<
  AppShelf,
  { list: HTMLUListElement; pick: HTMLSelectElement; manual: HTMLInputElement }
> = {
  staysOnScreen: {
    list: byId<HTMLUListElement>("apps-stays"),
    pick: byId<HTMLSelectElement>("apps-stays-pick"),
    manual: byId<HTMLInputElement>("apps-stays-manual"),
  },
  hidden: {
    list: byId<HTMLUListElement>("apps-hidden"),
    pick: byId<HTMLSelectElement>("apps-hidden-pick"),
    manual: byId<HTMLInputElement>("apps-hidden-manual"),
  },
};

const switchRows = byId<HTMLDivElement>("switch-rows");
const keepAwakeMinutes = byId<HTMLInputElement>("keep-awake-minutes");
const allowDisplayOff = byId<HTMLInputElement>("allow-display-off");
const btSelect = byId<HTMLSelectElement>("bt-device");
const trayBox = byId<HTMLDivElement>("tray-icons");

const hotkeyRows = byId<HTMLDivElement>("hotkey-rows");

const styleRadios = Array.from(
  document.querySelectorAll<HTMLInputElement>('input[name="backdrop-style"]'),
);
const monitorRadios = Array.from(
  document.querySelectorAll<HTMLInputElement>('input[name="backdrop-monitors"]'),
);
const colourBlock = byId<HTMLDivElement>("colour-block");
const gradientBlock = byId<HTMLDivElement>("gradient-block");
const pictureBlock = byId<HTMLDivElement>("picture-block");
const swatchBox = byId<HTMLDivElement>("swatches");
const gradientBox = byId<HTMLDivElement>("gradients");
const customColour = byId<HTMLInputElement>("custom-colour");
const choosePictureButton = byId<HTMLButtonElement>("choose-picture");
const picturePath = byId<HTMLParagraphElement>("picture-path");
const blurRange = byId<HTMLInputElement>("backdrop-blur");
const dimRange = byId<HTMLInputElement>("backdrop-dim");
const blurValue = byId<HTMLSpanElement>("blur-value");
const dimValue = byId<HTMLSpanElement>("dim-value");

const versionLabel = byId<HTMLSpanElement>("app-version");

// ---- Render ----

function render(): void {
  // Rows are rebuilt, so a focused control inside one loses focus; put it
  // back on the replacement (same id) when the render detached it.
  const focused = document.activeElement;
  renderAll();
  if (
    focused instanceof HTMLElement &&
    focused.id !== "" &&
    !focused.isConnected
  ) {
    document.getElementById(focused.id)?.focus();
  }
}

function renderAll(): void {
  if (!settings) return;
  startWithWindows.checked = settings.general.startWithWindows;
  checkUpdates.checked = settings.general.checkForUpdates;
  countdownBox.checked = settings.recordingMode.countdown;
  renderActions();
  renderTimeLimit();
  renderApps();
  renderSwitchRows();
  keepAwakeMinutes.value = String(settings.keepAwake.defaultMinutes);
  allowDisplayOff.checked = settings.keepAwake.allowDisplayOff;
  renderBluetooth();
  renderTrayIcons();
  renderHotkeys();
  renderBackdrop();
}

function switchRow(id: SwitchId): AppState["switches"][number] | undefined {
  return state?.switches.find((row) => row.id === id);
}

/** The action rows read the state only through availability and its
 * reason, and the core ticks state-changed every second during a session
 * (spec 7.7): compare these before rebuilding the rows. */
function availabilitySignature(): string {
  return ACTION_IDS.map((id) => {
    const row = switchRow(id);
    return `${id}:${row?.availability ?? "available"}:${row?.detail ?? ""}`;
  }).join("|");
}

let lastAvailability = "";

/** One checkbox per action (FR-40). An unavailable action keeps its row
 * with the reason, disabled (spec 7.6). */
function renderActions(): void {
  if (!settings) return;
  actionsBox.replaceChildren();

  for (const id of ACTION_IDS) {
    const availability = switchRow(id);
    const reason =
      availability === undefined || availability.availability === "available"
        ? undefined
        : (availability.detail ?? UNAVAILABLE_LABELS[availability.availability]);

    const row = make("div", "row");
    const checkbox = make("input", "");
    checkbox.type = "checkbox";
    checkbox.id = `action-${id}`;
    checkbox.checked = settings.recordingMode.actions.includes(id);
    checkbox.disabled = reason !== undefined;

    const label = make("label", "name", LABELS[id]);
    label.htmlFor = checkbox.id;

    // The stored list keeps the spec 7.1 apply order.
    checkbox.addEventListener("change", () => {
      const on = checkbox.checked;
      void saveChange((next) => {
        const chosen = new Set(next.recordingMode.actions);
        if (on) chosen.add(id);
        else chosen.delete(id);
        next.recordingMode.actions = ACTION_IDS.filter((entry) =>
          chosen.has(entry),
        );
      });
    });

    row.append(checkbox, label);
    if (reason !== undefined) row.append(make("span", "detail", reason));
    actionsBox.append(row);
  }
  lastAvailability = availabilitySignature();
}

function renderAppShelf(shelf: AppShelf, listed: string[]): void {
  if (!settings) return;
  const other: AppShelf = shelf === "staysOnScreen" ? "hidden" : "staysOnScreen";
  const otherName = other === "hidden" ? "Hidden" : "Stays on Screen";
  const { list, pick } = appLists[shelf];
  list.replaceChildren();

  for (const name of settings.apps[shelf]) {
    const row = make("li", "row");
    row.append(make("span", "name", name));

    const move = makeButton(
      `app-${shelf}-move-${name}`,
      "Move",
      "icon",
      `Move ${name} to ${otherName}`,
    );
    move.addEventListener("click", () => {
      // The lists come from the draft, so a click that lands before the
      // previous save's re-render never works from a stale copy.
      void saveChange((next) => {
        const moved = moveApp(next.apps[shelf], next.apps[other], name);
        next.apps[shelf] = moved.from;
        next.apps[other] = moved.to;
      });
    });

    const drop = makeButton(
      `app-${shelf}-remove-${name}`,
      "Remove",
      "icon",
      `Remove ${name}`,
    );
    drop.addEventListener("click", () => {
      void saveChange((next) => {
        next.apps[shelf] = removeApp(next.apps[shelf], name);
      });
    });

    row.append(move, drop);
    list.append(row);
  }

  // Running apps that are on neither shelf (spec 5.1: case-insensitive).
  const previous = pick.value;
  pick.replaceChildren(new Option("Running app…", ""));
  for (const app of runningApps) {
    if (listed.includes(app.exe.toLowerCase())) continue;
    pick.append(new Option(app.exe, app.exe));
  }
  if (previous !== "" && !listed.includes(previous.toLowerCase())) {
    pick.value = previous;
  }
}

function renderTimeLimit(): void {
  if (!settings) return;
  const minutes = settings.recordingMode.timeLimitMinutes;
  timeLimit.replaceChildren(new Option("Off", ""));
  for (const limit of TIME_LIMITS) {
    timeLimit.append(new Option(limit.label, String(limit.minutes)));
  }
  // A limit set outside the presets (CLI, a hand-edited file) is shown as
  // itself instead of silently reading "Off".
  if (minutes !== null && !TIME_LIMITS.some((limit) => limit.minutes === minutes)) {
    timeLimit.append(new Option(`${minutes} minutes`, String(minutes)));
  }
  timeLimit.value = minutes === null ? "" : String(minutes);
}

timeLimit.addEventListener("change", () => {
  const value = timeLimit.value;
  void saveChange((next) => {
    next.recordingMode.timeLimitMinutes = value === "" ? null : Number(value);
  });
});

countdownBox.addEventListener("change", () => {
  const checked = countdownBox.checked;
  void saveChange((next) => {
    next.recordingMode.countdown = checked;
  });
});

// ---- Apps shelves (FR-43) ----

function renderApps(): void {
  if (!settings) return;
  const listed = [
    ...settings.apps.staysOnScreen,
    ...settings.apps.hidden,
  ].map((name) => name.toLowerCase());

  for (const shelf of ["staysOnScreen", "hidden"] as AppShelf[]) {
    renderAppShelf(shelf, listed);
  }
}

/** Add the picked or typed app. Validation runs inside the save queue
 * against the current copy, so two quick adds cannot fight over one
 * snapshot; a refusal shows the reason and keeps the typed text. */
function addFromShelf(shelf: AppShelf): void {
  const { pick, manual } = appLists[shelf];
  const other: AppShelf = shelf === "staysOnScreen" ? "hidden" : "staysOnScreen";
  const otherName = other === "hidden" ? "Hidden" : "Stays on Screen";
  const typed = manual.value.trim();
  const name = typed !== "" ? typed : pick.value;

  if (name === "") {
    showError("Pick a running app or type an executable file name like Zoom.exe");
    return;
  }
  void saveChange((next) => {
    if (
      next.apps[other].some(
        (entry) => entry.toLowerCase() === name.toLowerCase(),
      )
    ) {
      showError(`'${name}' is already on ${otherName}`);
      return false;
    }
    if (!isExeName(name)) {
      showError(`'${name}' is not an executable file name like Zoom.exe`);
      return false;
    }
    const withApp = addApp(next.apps[shelf], name);
    if (withApp === null) {
      // Shape is validated above, so a refusal here is a duplicate on
      // this shelf (the other-shelf case was refused before it).
      showError(`'${name}' is already on the list`);
      return false;
    }
    next.apps[shelf] = withApp;
    manual.value = "";
  });
}

for (const shelf of ["staysOnScreen", "hidden"] as AppShelf[]) {
  byId<HTMLButtonElement>(
    shelf === "staysOnScreen" ? "apps-stays-add" : "apps-hidden-add",
  ).addEventListener("click", () => addFromShelf(shelf));
  appLists[shelf].manual.addEventListener("keydown", (e) => {
    if (e.key === "Enter") addFromShelf(shelf);
  });
}

// ---- Switches: the three dropdown shelves (FR-44) ----

function shelfOf(dropdown: Shelves, id: SwitchId): keyof Shelves {
  if (dropdown.shown.includes(id)) return "shown";
  if (dropdown.more.includes(id)) return "more";
  return "hidden";
}

function renderSwitchRows(): void {
  if (!settings) return;
  switchRows.replaceChildren();

  for (const id of ALL_SWITCHES) {
    const shelf = shelfOf(settings.dropdown, id);
    const index = settings.dropdown[shelf].indexOf(id);
    const shelfLabel = SHELF_LABELS[shelf];

    const row = make("div", "row");
    row.append(make("span", "name", LABELS[id]));

    const select = make("select", "");
    select.id = `shelf-${id}`;
    select.setAttribute("aria-label", `Dropdown shelf for ${LABELS[id]}`);
    for (const name of ["shown", "more", "hidden"] as (keyof Shelves)[]) {
      select.append(new Option(SHELF_LABELS[name], name));
    }
    select.value = shelf;
    select.addEventListener("change", () => {
      const target = select.value as keyof Shelves;
      void saveChange((next) => {
        next.dropdown = moveBetween(next.dropdown, id, target);
      });
    });

    // The index is read from the draft when the click lands, so a row that
    // shifted after the last save still moves the right switch.
    const step = (delta: number) => {
      void saveChange((next) => {
        const from = shelfOf(next.dropdown, id);
        const list = next.dropdown[from];
        next.dropdown[from] = reorder(list, list.indexOf(id), delta);
      });
    };

    const up = makeButton(
      `shelf-${id}-up`,
      "↑",
      "icon",
      `Move ${LABELS[id]} up on ${shelfLabel}`,
    );
    up.disabled = index <= 0;
    up.addEventListener("click", () => step(-1));

    const down = makeButton(
      `shelf-${id}-down`,
      "↓",
      "icon",
      `Move ${LABELS[id]} down on ${shelfLabel}`,
    );
    down.disabled = index === settings.dropdown[shelf].length - 1;
    down.addEventListener("click", () => step(1));

    row.append(select, up, down);
    switchRows.append(row);
  }
}

// ---- Switches: Keep Awake (FR-18) and Bluetooth (FR-38) ----

keepAwakeMinutes.addEventListener("change", () => {
  const minutes = Number(keepAwakeMinutes.value);
  if (!Number.isInteger(minutes) || minutes < 1 || minutes > 1440) {
    showError("The Keep Awake default must be 1-1440 minutes");
    render();
    return;
  }
  void saveChange((next) => {
    next.keepAwake.defaultMinutes = minutes;
  });
});

allowDisplayOff.addEventListener("change", () => {
  const checked = allowDisplayOff.checked;
  void saveChange((next) => {
    next.keepAwake.allowDisplayOff = checked;
  });
});

function renderBluetooth(): void {
  if (!settings) return;
  const selected = settings.bluetooth.deviceId ?? "";
  btSelect.replaceChildren(new Option("None", ""));
  for (const device of btDevices) {
    const suffix = device.connected ? " (connected)" : "";
    btSelect.append(new Option(`${device.name}${suffix}`, device.id));
  }
  // A paired device that is not detected right now keeps its id rather
  // than pretending the user picked None.
  if (selected !== "" && !btDevices.some((device) => device.id === selected)) {
    btSelect.append(new Option("Current device (not detected)", selected));
  }
  btSelect.value = selected;
}

btSelect.addEventListener("change", () => {
  const value = btSelect.value;
  void saveChange((next) => {
    next.bluetooth.deviceId = value === "" ? null : value;
  });
});

// ---- Switches: tray icons (FR-30) ----

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function renderTrayIcons(): void {
  if (!settings) return;
  const visiblePaths = settings.trayIcons.visible;
  trayBox.replaceChildren();

  if (trayIconsError !== undefined) {
    // Windows 10 has no promoted-icon state (NOT_AVAILABLE, FR-29): say so
    // instead of showing an empty shelf.
    trayBox.append(make("p", "note", trayIconsError));
    return;
  }
  const icons = trayIcons ?? [];
  if (icons.length === 0) {
    trayBox.append(make("p", "note", "No third-party tray icons found."));
    return;
  }

  icons.forEach((icon, index) => {
    const row = make("div", "row");
    const checkbox = make("input", "");
    checkbox.type = "checkbox";
    checkbox.id = `tray-icon-${index}`;
    checkbox.checked = visiblePaths.some(
      (path) => path.toLowerCase() === icon.exe.toLowerCase(),
    );
    const label = make("label", "name", baseName(icon.exe));
    label.htmlFor = checkbox.id;
    label.title = icon.exe;
    checkbox.addEventListener("change", () => {
      const on = checkbox.checked;
      void saveChange((next) => {
        // Full paths, matched case-insensitively (spec 5.1, FR-30).
        const others = next.trayIcons.visible.filter(
          (path) => path.toLowerCase() !== icon.exe.toLowerCase(),
        );
        next.trayIcons.visible = on ? [...others, icon.exe] : others;
      });
    });
    row.append(checkbox, label);
    trayBox.append(row);
  });
}

// ---- Hotkeys (FR-45, spec 7.11) ----

function renderHotkeys(): void {
  if (!settings) return;
  const current = settings;
  hotkeyRows.replaceChildren();
  hotkeyRows.append(
    hotkeyRow("recordingMode", "Recording Mode", current.hotkeys.recordingMode),
  );
  for (const id of ALL_SWITCHES) {
    hotkeyRows.append(
      hotkeyRow(`switch:${id}`, LABELS[id], current.hotkeys.switches[id] ?? ""),
    );
  }
}

function hotkeyRow(key: string, label: string, binding: string): HTMLElement {
  const row = make("div", "row");
  const isCapturing = capturing === key;
  if (isCapturing) row.classList.add("capturing");
  row.append(make("span", "name", label));

  if (isCapturing) {
    row.append(make("span", "hint", "Press the new shortcut, Esc to cancel"));
  } else {
    row.append(make("span", "value", binding === "" ? "Not set" : binding));
  }

  const set = makeButton(
    `hotkey-${key}-set`,
    "Set",
    "icon",
    `Set the ${label} shortcut`,
  );
  set.addEventListener("click", () => {
    capturing = key;
    showError(undefined);
    render();
  });

  const clear = makeButton(
    `hotkey-${key}-clear`,
    "Clear",
    "icon",
    `Clear the ${label} shortcut`,
  );
  clear.disabled = binding === "";
  clear.addEventListener("click", () => {
    void assignHotkey(key, "");
  });

  row.append(set, clear);
  return row;
}

/** Save one binding; "" clears it (the core drops empty keys). */
function assignHotkey(key: string, accelerator: string): Promise<void> {
  if (key === "recordingMode") {
    return saveChange((next) => {
      next.hotkeys.recordingMode = accelerator;
    });
  }
  const id = key.slice("switch:".length) as SwitchId;
  return saveChange((next) => {
    if (accelerator === "") delete next.hotkeys.switches[id];
    else next.hotkeys.switches[id] = accelerator;
  });
}

// The capture runs in the capture phase so the press never reaches a
// control: the next combo becomes the binding (spec 7.11).
window.addEventListener(
  "keydown",
  (e) => {
    if (capturing === undefined) return;
    e.preventDefault();
    e.stopPropagation();

    // Escape is reserved for the Lock Keyboard unlock, so it cancels the
    // capture instead of saving a binding the core would reject.
    if (e.key === "Escape") {
      capturing = undefined;
      render();
      return;
    }
    // A bare key cannot be a global hotkey (it would also swallow Tab):
    // keep listening until a combo arrives.
    if (!(e.ctrlKey || e.altKey || e.metaKey)) return;
    const accelerator = eventToAccelerator(e);
    if (accelerator === null) return;

    const target = capturing;
    capturing = undefined;
    void assignHotkey(target, accelerator);
  },
  true,
);

// ---- Backdrop (FR-46) ----

function renderBackdrop(): void {
  if (!settings) return;
  const backdrop = settings.backdrop;

  for (const radio of styleRadios) radio.checked = radio.value === backdrop.style;
  for (const radio of monitorRadios) {
    radio.checked = radio.value === backdrop.monitors;
  }
  colourBlock.hidden = backdrop.style !== "color";
  gradientBlock.hidden = backdrop.style !== "gradient";
  // Both picture styles read backdrop.picture; the wallpaper one falls
  // back to the OS wallpaper when the path is empty (D-087).
  pictureBlock.hidden =
    backdrop.style !== "picture" && backdrop.style !== "wallpaper";

  renderSwatches();
  renderGradients();
  customColour.value = backdrop.color;
  picturePath.textContent = backdrop.picture ?? "No picture chosen.";
  blurRange.value = String(backdrop.blur);
  dimRange.value = String(backdrop.dim);
  blurValue.textContent = `${backdrop.blur} px`;
  dimValue.textContent = `${backdrop.dim} %`;
}

function renderSwatches(): void {
  if (!settings) return;
  // A swatch shows as pressed only while the colour style is the active
  // one; the stored colour is kept for when it comes back.
  const current =
    settings.backdrop.style === "color"
      ? settings.backdrop.color.toLowerCase()
      : undefined;
  swatchBox.replaceChildren();
  PRESET_COLOURS.forEach((colour, index) => {
    const button = makeButton(
      `swatch-${index}`,
      "",
      `swatch ${colour.className}`,
      colour.name,
    );
    button.setAttribute("aria-pressed", String(current === colour.value));
    button.addEventListener("click", () => {
      void saveChange((next) => {
        next.backdrop.color = colour.value;
        next.backdrop.style = "color";
      });
    });
    swatchBox.append(button);
  });
}

function renderGradients(): void {
  if (!settings) return;
  // Like the swatches: pressed only while the gradient style is active.
  const current =
    settings.backdrop.style === "gradient"
      ? settings.backdrop.gradient
      : undefined;
  gradientBox.replaceChildren();
  for (const preset of GRADIENTS) {
    const button = makeButton(
      `gradient-${preset.id}`,
      "",
      `gradient-tile gradient-${preset.id}`,
      `${preset.name} gradient`,
    );
    button.setAttribute("aria-pressed", String(current === preset.id));
    button.addEventListener("click", () => {
      void saveChange((next) => {
        next.backdrop.gradient = preset.id;
        next.backdrop.style = "gradient";
      });
    });
    gradientBox.append(button);
  }
}

/** Pick a picture, then save it. `style` is set in the same save when the
 * user asked for the picture style (save_settings rejects a picture style
 * without a path). */
async function choosePicture(style: BackdropStyle | undefined): Promise<void> {
  if (!settings) return;
  try {
    const path = await pickBackdropPicture();
    if (path === null) {
      render(); // cancelled: the Style radios keep the stored style
      return;
    }
    await saveChange((next) => {
      next.backdrop.picture = path;
      if (style !== undefined) next.backdrop.style = style;
    });
  } catch (e) {
    showError(message(e));
    render();
  }
}

for (const radio of styleRadios) {
  radio.addEventListener("change", () => {
    if (!radio.checked) return;
    const style = radio.value as BackdropStyle;
    if (style === "picture" && !settings?.backdrop.picture) {
      // No path yet: offer the picker, and switch the style once one is
      // chosen.
      void choosePicture("picture");
      return;
    }
    void saveChange((next) => {
      next.backdrop.style = style;
      // A gradient style needs a preset (save_settings rejects null, D-087):
      // the first one stands in until the user picks a tile.
      next.backdrop.gradient ??= GRADIENTS[0].id;
    });
  });
}

for (const radio of monitorRadios) {
  radio.addEventListener("change", () => {
    if (!radio.checked) return;
    const monitors = radio.value as Settings["backdrop"]["monitors"];
    void saveChange((next) => {
      next.backdrop.monitors = monitors;
    });
  });
}

customColour.addEventListener("change", () => {
  const colour = customColour.value;
  void saveChange((next) => {
    next.backdrop.color = colour;
  });
});

choosePictureButton.addEventListener("click", () => {
  void choosePicture(undefined);
});

// The label follows the thumb while dragging; only the release saves, so
// one drag is one write.
for (const [range, label, unit] of [
  [blurRange, blurValue, "px"],
  [dimRange, dimValue, "%"],
] as const) {
  range.addEventListener("input", () => {
    label.textContent = `${range.value} ${unit}`;
  });
}

blurRange.addEventListener("change", () => {
  const blur = Number(blurRange.value);
  void saveChange((next) => {
    next.backdrop.blur = blur;
  });
});

dimRange.addEventListener("change", () => {
  const dim = Number(dimRange.value);
  void saveChange((next) => {
    next.backdrop.dim = dim;
  });
});

// ---- General and About (FR-47, FR-52) ----

startWithWindows.addEventListener("change", () => {
  const checked = startWithWindows.checked;
  void saveChange((next) => {
    next.general.startWithWindows = checked;
  });
});

checkUpdates.addEventListener("change", () => {
  const checked = checkUpdates.checked;
  void saveChange((next) => {
    next.general.checkForUpdates = checked;
  });
});

for (const button of onboardingButtons) {
  button.addEventListener("click", () => {
    openOnboarding().catch((e: unknown) => showError(message(e)));
  });
}

// ---- Loading ----

/** Lists and the version load once; a failed list only costs its section
 * (the tray list has its own Windows 10 message). */
async function loadLists(): Promise<void> {
  const [devices, apps, icons, version] = await Promise.allSettled([
    listBtDevices(),
    listApps(),
    listTrayIcons(),
    getVersion(),
  ]);
  if (devices.status === "fulfilled") btDevices = devices.value;
  if (apps.status === "fulfilled") runningApps = apps.value;
  if (icons.status === "fulfilled") {
    trayIcons = icons.value;
  } else if (
    icons.reason instanceof IpcError &&
    icons.reason.code === "NOT_AVAILABLE"
  ) {
    // FR-29/FR-30: Windows 10 has no promoted-icon state; the task fixes
    // this wording, so it does not echo the core's message.
    trayIcons = [];
    trayIconsError = "Windows 10 doesn't support this";
  } else {
    trayIconsError = message(icons.reason);
  }
  if (version.status === "fulfilled") versionLabel.textContent = version.value;
  render();
}

async function init(): Promise<void> {
  // Listen before the first read so a change written elsewhere (dropdown,
  // tray, onboarding) is never missed (spec 5.1: the core is the writer).
  void onSettingsChanged((next) => {
    // A payload from another writer is newer than any save in flight.
    payloadRevision += 1;
    settings = next;
    render();
  });
  void onStateChanged((next) => {
    state = next;
    const signature = availabilitySignature();
    if (signature === lastAvailability) return;
    lastAvailability = signature;
    renderActions();
  });

  try {
    const [initialSettings, initialState] = await Promise.all([
      getSettings(),
      getState(),
    ]);
    settings = initialSettings;
    state = initialState;
    render();
  } catch (e) {
    showError(message(e));
    return;
  }
  await loadLists();
}

void init();
