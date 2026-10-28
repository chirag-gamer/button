// Dropdown panel (FR-2): Recording Mode card + the switch shelves (FR-44),
// with clickable switch rows, off-timer menus (FR-27, FR-28), the Keep
// Awake durations (FR-18, D-081) and the Bluetooth picker (FR-38).
import "./dropdown.css";
import "../shared/theme.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AppState,
  Availability,
  BtDevice,
  OffAfter,
  Settings,
  SwitchId,
  extendRecording,
  getState,
  getSettings,
  listBtDevices,
  onCountdown,
  onSettingsChanged,
  onStateChanged,
  openSettings,
  setBluetoothDevice,
  setKeepAwakeOptions,
  setSwitch,
  stopRecording,
  toggleRecording,
} from "../shared/ipc";
import { formatElapsed } from "../shared/format";

// Display names (FR-3's names, spec 7.5 catalogue).
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

// The generic off-timer presets (D-051); Keep Awake adds its own set.
const TIMER_PRESETS: { label: string; value: OffAfter }[] = [
  { label: "15 min", value: "15m" },
  { label: "1 h", value: "1h" },
  { label: "4 h", value: "4h" },
  { label: "Tomorrow 8:00", value: "tomorrow" },
];

const KEEP_AWAKE_PRESETS: { label: string; value: OffAfter | undefined }[] = [
  { label: "15 min", value: "15m" },
  { label: "1 h", value: "1h" },
  { label: "2 h", value: { minutes: 120 } },
  { label: "4 h", value: "4h" },
  { label: "Tomorrow 8:00", value: "tomorrow" },
  { label: "No end", value: undefined },
];

const EXTEND_CHIPS: { label: string; minutes: number }[] = [
  { label: "+15 min", minutes: 15 },
  { label: "+1 h", minutes: 60 },
  { label: "+4 h", minutes: 240 },
];

let state: AppState | undefined;
let settings: Settings | undefined;
let btDevices: BtDevice[] = [];
let showAll = false;
// The one open popover (timer menu or Bluetooth picker), closed on any
// click outside it.
let openMenu: HTMLElement | undefined;

const count = document.getElementById("action-count")!;
const startRestore = document.getElementById("start-restore") as HTMLButtonElement;
const shelf = document.getElementById("shelf")!;
const showAllButton = document.getElementById("show-all") as HTMLButtonElement;
const more = document.getElementById("more")!;
const summary = document.getElementById("switch-summary")!;
const switchError = document.getElementById("switch-error")!;
const countdownLine = document.getElementById("countdown-line")!;
const countdownText = document.getElementById("countdown-text")!;
const countdownCancel = document.getElementById(
  "countdown-cancel",
) as HTMLButtonElement;
const limitLine = document.getElementById("limit-line")!;
const limitText = document.getElementById("limit-text")!;
const extendButton = document.getElementById("extend") as HTMLButtonElement;
const openSettingsButton = document.getElementById(
  "open-settings",
) as HTMLButtonElement;

// The latest countdown tick (FR-41); the dropdown shows it while the
// session status is "countdown".
let countdownSeconds = 3;

let errorTimer: number | undefined;

/** FR-27's inline error surface: the message shows for 4 s. */
function showError(message: string): void {
  switchError.textContent = message;
  switchError.hidden = false;
  window.clearTimeout(errorTimer);
  errorTimer = window.setTimeout(() => {
    switchError.hidden = true;
  }, 4000);
}

function closeMenu(): void {
  openMenu?.remove();
  openMenu = undefined;
}

document.addEventListener("click", (e) => {
  if (openMenu && e.target instanceof Node && !openMenu.contains(e.target)) {
    closeMenu();
  }
});

function render(): void {
  if (!state || !settings) return;

  // FR-9: warn when the hotkey could not be registered.
  const warning = document.getElementById("hotkey-warning")!;
  warning.hidden = state.hotkey.registered;
  if (!state.hotkey.registered) {
    warning.textContent = `Hotkey ${state.hotkey.binding} could not be registered (in use by another app?)`;
  }

  // Card (FR-2): the number of selected actions and Start/Restore.
  const selected = settings.recordingMode.actions.length;
  count.textContent = `${selected} action${selected === 1 ? "" : "s"}`;
  const active = state.session.status === "active";
  startRestore.textContent = active ? "Restore" : "Start";
  startRestore.classList.toggle("recording", active);
  startRestore.disabled =
    state.session.status !== "idle" && state.session.status !== "active";

  // Summary (FR-27): available, non-action switches only.
  const counted = state.switches.filter(
    (s) => s.availability === "available" && !isAction(s.id),
  );
  const on = counted.filter((s) => s.on).length;
  summary.textContent = `${on} on · ${counted.length - on} off`;
  summary.hidden = counted.length === 0;

  // FR-41: while the countdown runs, show the tick and a Cancel.
  if (state.session.status === "countdown") {
    countdownText.textContent = `Starting in ${countdownSeconds}…`;
    countdownLine.hidden = false;
  } else {
    countdownLine.hidden = true;
  }

  // FR-42: with a time limit, show the remaining time and the extend
  // action (desktop notifications have no buttons, D-090).
  const endsAt = state.session.endsAt
    ? new Date(state.session.endsAt).getTime()
    : undefined;
  if (active && endsAt !== undefined) {
    const remaining = Math.max(0, Math.round((endsAt - Date.now()) / 1000));
    limitText.textContent = `Ends in ${formatElapsed(remaining)}`;
    limitLine.hidden = false;
  } else {
    limitLine.hidden = true;
  }

  // An open menu must survive the once-a-second state-changed re-render
  // (replaceChildren would destroy the row it lives in); the shelves
  // rebuild on the next event after it closes.
  if (openMenu) return;

  // Shelves (FR-44): shown, "Show all", hidden.
  const byId = new Map(state.switches.map((s) => [s.id, s]));
  renderShelf(shelf, settings.dropdown.shown, byId);
  renderShelf(more, settings.dropdown.more, byId);
  showAllButton.hidden = settings.dropdown.more.length === 0;
  showAllButton.textContent = showAll ? "Show less" : "Show all";
  more.hidden = !showAll;
}

function renderShelf(
  element: HTMLElement,
  ids: SwitchId[],
  byId: Map<SwitchId, AppState["switches"][number]>,
): void {
  element.replaceChildren();
  for (const id of ids) {
    const switchState = byId.get(id);
    if (!switchState) continue;
    element.append(row(switchState));
  }
}

function isAction(id: SwitchId): boolean {
  // Screen saver is the only engine action: it fires once, no on/off state.
  // Lock keyboard is a real toggle (click while locked unlocks it).
  return id === "screen-saver";
}

function row(s: AppState["switches"][number]): HTMLElement {
  const rowEl = document.createElement("div");
  rowEl.className = "row";
  if (s.availability !== "available") rowEl.classList.add("unavailable");
  if (isAction(s.id) || s.id === "keep-awake") rowEl.classList.add("wide");

  // ARIA: role=switch/button on the row would fold every child (badges,
  // chips, the "⋯" menu) into the widget and hide it from screen readers,
  // so the row is only a labelled group; the toggle semantics live on the
  // dot/Start control at the row's end (spec 10).
  rowEl.setAttribute("role", "group");
  rowEl.setAttribute("aria-label", LABELS[s.id]);

  const name = document.createElement("span");
  name.className = "name";
  name.textContent = LABELS[s.id];
  rowEl.append(name);

  if (s.id === "bluetooth") {
    rowEl.append(bluetoothDetail());
  }

  if (s.experimental) {
    rowEl.append(badge("Experimental"));
  }
  if (s.availability !== "available") {
    rowEl.append(badge(s.detail ?? UNAVAILABLE_LABELS[s.availability]));
  } else if (s.offAt) {
    const secondsLeft = (Date.parse(s.offAt) - Date.now()) / 1000;
    if (secondsLeft > 0) {
      rowEl.append(badge(`off in ${formatElapsed(secondsLeft)}`));
    }
  }

  // Clicking anywhere on the row toggles it (FR-27). Keep Awake's plain
  // click uses the default duration (D-081); actions fire their trigger.
  rowEl.addEventListener("click", () => {
    void toggleRow(s);
  });

  if (s.availability === "available" && !isAction(s.id)) {
    const more = document.createElement("button");
    more.className = "more-btn";
    more.textContent = "⋯";
    more.title = "Timer options";
    more.addEventListener("click", (e) => {
      e.stopPropagation();
      openRowMenu(rowEl, s);
    });
    rowEl.append(more);
  }

  if (s.id === "keep-awake" && s.on) {
    rowEl.append(extendChips(s));
  }

  // The dedicated toggle control: keyboard-focusable, Enter/Space native.
  const actionable = s.availability === "available";
  if (s.id === "screen-saver") {
    const start = document.createElement("button");
    start.className = "badge start";
    start.textContent = "Start";
    start.setAttribute("aria-label", `Start ${LABELS[s.id]}`);
    if (actionable) {
      start.addEventListener("click", (e) => {
        e.stopPropagation(); // the row's click handler would double-fire
        closeMenu(); // our stopPropagation hides this click from the
        // document-level menu closer, and the render guard would freeze
        void toggleRow(s); // the shelves while it believed a menu is open
      });
    } else {
      start.disabled = true;
    }
    rowEl.append(start);
  } else {
    const dot = document.createElement("button");
    dot.className = s.on ? "state on" : "state";
    if (actionable) {
      dot.setAttribute("role", "switch");
      dot.setAttribute("aria-checked", String(s.on));
      // Name the entity, not the action: role+aria-checked carry the
      // state ("Dark Mode, switch, checked"), and an action label loses
      // the context entirely.
      dot.setAttribute("aria-label", LABELS[s.id]);
      dot.addEventListener("click", (e) => {
        e.stopPropagation(); // the row's click handler would double-fire
        closeMenu(); // see the Start badge above
        void toggleRow(s);
      });
    } else {
      dot.disabled = true;
      dot.setAttribute("aria-label", LABELS[s.id]);
    }
    rowEl.append(dot);
  }
  return rowEl;
}

async function toggleRow(s: AppState["switches"][number]): Promise<void> {
  if (s.availability !== "available") return;
  try {
    if (s.id === "keep-awake" && !s.on && settings) {
      // Plain click: on for the default duration (D-081).
      state = await setSwitch(s.id, true, {
        minutes: settings.keepAwake.defaultMinutes,
      });
    } else {
      state = await setSwitch(s.id, !s.on);
    }
    if (s.id === "bluetooth") refreshBtSoon();
    render();
  } catch (e) {
    showError(e instanceof Error ? e.message : String(e));
  }
}

/** The +N chips shift an existing deadline, or start one from now. */
function extendChips(s: AppState["switches"][number]): HTMLElement {
  const chips = document.createElement("span");
  chips.className = "chips";
  for (const chip of EXTEND_CHIPS) {
    const button = document.createElement("button");
    button.className = "chip";
    button.textContent = chip.label;
    button.addEventListener("click", async (e) => {
      e.stopPropagation();
      closeMenu(); // same render-guard hazard as the dot/Start clicks
      const from = s.offAt ? Date.parse(s.offAt) : Date.now();
      const at = new Date(from + chip.minutes * 60_000).toISOString();
      try {
        state = await setSwitch(s.id, true, { at });
        render();
      } catch (err) {
        showError(err instanceof Error ? err.message : String(err));
      }
    });
    chips.append(button);
  }
  return chips;
}

function bluetoothDetail(): HTMLElement {
  const chosen = settings?.bluetooth.deviceId;
  const device = btDevices.find((d) => d.id === chosen);
  const detail = document.createElement("span");
  detail.className = "detail";
  if (!chosen) {
    detail.textContent = "Choose a device";
  } else if (device) {
    const battery = device.battery === null ? "n/a" : `${device.battery}%`;
    detail.textContent = `${device.name} · ${battery}`;
  } else {
    detail.textContent = "Device not found";
  }
  return detail;
}

/** The "⋯" popover: timer presets, Keep Awake extras, or the BT picker. */
function openRowMenu(rowEl: HTMLElement, s: AppState["switches"][number]): void {
  closeMenu();
  const menu = document.createElement("div");
  menu.className = "menu";
  // The menu lives inside the clickable row: without this, every item
  // click bubbles on and also toggles the switch.
  menu.addEventListener("click", (e) => e.stopPropagation());
  // Open upward when the row sits low, so the menu is not clipped.
  const shelf = rowEl.parentElement;
  if (shelf && rowEl.offsetTop > shelf.offsetHeight / 2) {
    menu.classList.add("up");
  }
  if (s.id === "bluetooth") {
    fillBtPicker(menu);
    // The cached list can be stale (paired since the dropdown opened):
    // re-query and refill while it is open.
    void listBtDevices()
      .then((devices) => {
        btDevices = devices;
        if (openMenu === menu) {
          menu.replaceChildren();
          fillBtPicker(menu);
        }
      })
      .catch(() => {
        /* keep the cached list; errors surface on the toggle click */
      });
  } else {
    const presets = s.id === "keep-awake" ? KEEP_AWAKE_PRESETS : TIMER_PRESETS;
    for (const preset of presets) {
      const item = document.createElement("button");
      item.textContent = preset.label;
      item.addEventListener("click", async () => {
        closeMenu();
        try {
          state = await setSwitch(s.id, true, preset.value);
          render();
        } catch (e) {
          showError(e instanceof Error ? e.message : String(e));
        }
      });
      menu.append(item);
    }
    if (s.id === "keep-awake" && settings) {
      const label = document.createElement("label");
      label.className = "menu-check";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.checked = settings.keepAwake.allowDisplayOff;
      box.addEventListener("change", async () => {
        try {
          settings = await setKeepAwakeOptions(box.checked);
          render();
        } catch (e) {
          showError(e instanceof Error ? e.message : String(e));
          box.checked = !box.checked;
        }
      });
      label.append(box, " Let the screen turn off");
      menu.append(label);
    }
  }
  rowEl.append(menu);
  openMenu = menu;
}

function fillBtPicker(menu: HTMLElement): void {
  if (btDevices.length === 0) {
    const empty = document.createElement("span");
    empty.className = "menu-empty";
    empty.textContent = "No paired Bluetooth audio devices";
    menu.append(empty);
  }
  for (const device of btDevices) {
    const item = document.createElement("button");
    const battery = device.battery === null ? "" : ` · ${device.battery}%`;
    item.textContent = `${device.connected ? "● " : ""}${device.name}${battery}`;
    if (device.id === settings?.bluetooth.deviceId) {
      item.classList.add("current");
    }
    item.addEventListener("click", async () => {
      closeMenu();
      try {
        settings = await setBluetoothDevice(device.id);
        render();
      } catch (e) {
        showError(e instanceof Error ? e.message : String(e));
      }
    });
    menu.append(item);
  }
}

/** Connecting settles over seconds: refresh the picker list a few times
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
          /* picker keeps the last list; errors already surfaced on click */
        });
    }, delay);
  }
}

function badge(text: string): HTMLElement {
  const el = document.createElement("span");
  el.className = "badge";
  el.textContent = text;
  return el;
}

startRestore.addEventListener("click", async () => {
  try {
    state = await toggleRecording();
    render();
  } catch (e) {
    // The state changed under us (e.g. session started elsewhere);
    // refresh instead of erroring (FR-2 has no error UI).
    console.error(e);
    state = await getState();
    render();
  }
});

showAllButton.addEventListener("click", () => {
  showAll = !showAll;
  render();
});

// FR-41: cancelling the countdown is a Stop (the dispatcher cancels the
// countdown instead of stopping a session that never started).
countdownCancel.addEventListener("click", async () => {
  try {
    state = await stopRecording();
    render();
  } catch (e) {
    console.error(e);
    state = await getState();
    render();
  }
});

// FR-42: push the time limit back 15 minutes.
extendButton.addEventListener("click", async () => {
  try {
    state = await extendRecording(15);
    render();
  } catch (e) {
    console.error(e);
    state = await getState();
    render();
  }
});

// FR-39: the settings window opens next to the dropdown; the dropdown
// hides on the blur that follows.
openSettingsButton.addEventListener("click", () => {
  void openSettings();
});

// FR-41: the countdown ticks arrive as events (the ticker does not run
// during a countdown).
void onCountdown((secondsLeft) => {
  countdownSeconds = secondsLeft;
  if (state?.session.status === "countdown") {
    render();
  }
});

// Blur hides the whole dropdown (tray.rs): close any open menu too, or
// the render guard keeps the shelves frozen on stale state after reopen.
window.addEventListener("blur", () => closeMenu());

// Escape closes an open menu first; only a second Escape hides the
// dropdown (standard popover behaviour, spec 10).
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    if (openMenu) {
      closeMenu();
      return;
    }
    void getCurrentWindow().hide();
  }
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
  settings = next;
  render();
});
