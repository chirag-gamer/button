// Dropdown panel (FR-2): Recording Mode card + the switch shelves (FR-44).
// Read-only in Phase 2: quick switches arrive in Phase 4 (FR-27, FR-28).
import "./dropdown.css";
import "../shared/theme.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AppState,
  Availability,
  getState,
  getSettings,
  onStateChanged,
  Settings,
  SwitchId,
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

let state: AppState | undefined;
let settings: Settings | undefined;
let showAll = false;

const count = document.getElementById("action-count")!;
const startRestore = document.getElementById("start-restore") as HTMLButtonElement;
const shelf = document.getElementById("shelf")!;
const showAllButton = document.getElementById("show-all") as HTMLButtonElement;
const more = document.getElementById("more")!;

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

function row(s: AppState["switches"][number]): HTMLElement {
  const rowEl = document.createElement("div");
  rowEl.className = "row";
  if (s.availability !== "available") rowEl.classList.add("unavailable");

  const name = document.createElement("span");
  name.className = "name";
  name.textContent = LABELS[s.id];
  rowEl.append(name);

  if (s.experimental) {
    rowEl.append(badge("Experimental"));
  }
  if (s.availability !== "available") {
    rowEl.append(badge(UNAVAILABLE_LABELS[s.availability]));
  } else if (s.offAt) {
    const secondsLeft = (Date.parse(s.offAt) - Date.now()) / 1000;
    if (secondsLeft > 0) {
      rowEl.append(badge(`off in ${formatElapsed(secondsLeft)}`));
    }
  }

  const dot = document.createElement("span");
  dot.className = s.on ? "state on" : "state";
  rowEl.append(dot);
  return rowEl;
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

// Escape closes the dropdown like a blur would.
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") void getCurrentWindow().hide();
});

Promise.all([getState(), getSettings()]).then(([initialState, initialSettings]) => {
  state = initialState;
  settings = initialSettings;
  render();
});
void onStateChanged((next) => {
  state = next;
  render();
});
