// First-run onboarding wizard (FR-52, spec 7.14, D-047): four steps:
// what button does, the Recording Mode hotkey with Change, where the tray
// icon is, and Start with Windows. Every save sends the whole Settings
// object and adopts the returned one, like the Settings window, so a
// rejected hotkey leaves the old binding on screen. Closing the window
// (Done or the title bar X) is what sets general.onboardingDone, in
// onboarding.rs; this page never writes that flag.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { eventToAccelerator } from "../shared/accel";
import {
  Settings,
  getSettings,
  getState,
  onSettingsChanged,
  saveSettings,
} from "../shared/ipc";
import "./onboarding.css";
import "../shared/theme.css";

/** The four steps in order; `hidden` on a section is the only step state. */
const STEPS = ["step-1", "step-2", "step-3", "step-4"] as const;
const LAST_STEP = STEPS.length - 1;

const stepSections = STEPS.map(
  (id) => document.getElementById(id) as HTMLElement,
);
const indicator = document.getElementById("step-indicator")!;
const errorLine = document.getElementById("step-error")!;
const backButton = document.getElementById("back") as HTMLButtonElement;
const nextButton = document.getElementById("next") as HTMLButtonElement;
const introHotkey = document.getElementById("intro-hotkey")!;
const hotkeyValue = document.getElementById("hotkey-value")!;
const hotkeyChange = document.getElementById(
  "hotkey-change",
) as HTMLButtonElement;
const hotkeyHint = document.getElementById("hotkey-hint")!;
const trayWin11 = document.getElementById("tray-win11")!;
const trayWin10 = document.getElementById("tray-win10")!;
const startWithWindows = document.getElementById(
  "start-with-windows",
) as HTMLInputElement;

/** The last settings object the core returned; every save builds on it. */
let settings: Settings | undefined;
let step = 0;
/** True while the next keydown becomes the new Recording Mode binding. */
let capturing = false;
/** A save is in flight: the control that started it stays disabled, so a
 * second save cannot be built from the copy from before the first. */
let savingHotkeys = false;
let savingGeneral = false;

/** Disabling a focused control drops focus to the body; put it back so a
 * keyboard user is not thrown out of the step by a slow save. */
function restoreFocus(element: HTMLElement, hadFocus: boolean): void {
  if (hadFocus && document.activeElement === document.body) {
    element.focus();
  }
}

function showError(message: string): void {
  errorLine.textContent = message;
  errorLine.hidden = false;
}

function hideError(): void {
  errorLine.textContent = "";
  errorLine.hidden = true;
}

/** Steps 1 and 2 show the binding from the same saved value, so step 1
 * stays truthful after a change made on step 2. */
function renderHotkey(): void {
  const binding = settings?.hotkeys.recordingMode ?? "";
  introHotkey.textContent = binding;
  hotkeyValue.textContent = binding;
}

/** Everything that depends on `settings` or on a save in flight. Before
 * the first successful read both saves are disabled: a save built from an
 * unknown object would overwrite settings this page never saw. While a
 * save runs the checkbox keeps the state the user just chose. */
function renderControls(): void {
  const loaded = settings !== undefined;
  if (!savingGeneral) {
    startWithWindows.checked = settings?.general.startWithWindows ?? false;
  }
  startWithWindows.disabled = !loaded || savingGeneral;
  hotkeyChange.disabled = !loaded || savingHotkeys;
  // Next doubles as Done, which closes the window and drops a save still
  // in flight; hold it until the save has landed either way.
  nextButton.disabled = savingGeneral || savingHotkeys;
}

/** Show one step, relabel the primary button, and move focus to the new
 * heading so a keyboard or screen-reader user lands inside the step. */
function render(focusHeading: boolean): void {
  stepSections.forEach((section, index) => {
    section.hidden = index !== step;
  });
  indicator.textContent = `Step ${step + 1} of ${STEPS.length}`;
  backButton.hidden = step === 0;
  nextButton.textContent = step === LAST_STEP ? "Done" : "Next";
  if (focusHeading) {
    const heading = stepSections[step].querySelector<HTMLElement>("h1");
    heading?.focus();
  }
}

/** Leave capture without saving: Escape, a second Change click, or the
 * moment a full binding was captured (the save runs after this). */
function stopCapture(): void {
  if (!capturing) return;
  capturing = false;
  hotkeyHint.hidden = true;
  window.removeEventListener("keydown", onCaptureKey, true);
  renderControls();
}

/** The capture listener (spec 7.11): it uses the same helper as the
 * Settings window, on the capture phase, so no control acts on the press
 * and a shortcut never triggers anything. A modifier-only press or a key
 * the global-hotkey parser cannot take keeps the capture running. */
function onCaptureKey(event: KeyboardEvent): void {
  event.preventDefault();
  event.stopPropagation();
  if (event.repeat) return;
  // Escape cancels instead of saving: it is reserved for unlocking the
  // keyboard, so the core would reject it anyway.
  if (event.key === "Escape" || event.code === "Escape") {
    stopCapture();
    hotkeyChange.focus();
    return;
  }
  // A bare key cannot be a global hotkey (it would swallow normal
  // typing), so keep listening until a combo arrives, like the Settings
  // window's editor (spec 7.11).
  if (!(event.ctrlKey || event.altKey || event.metaKey)) return;
  const binding = eventToAccelerator(event);
  if (binding === null) return;
  stopCapture();
  void saveRecordingMode(binding);
}

function startCapture(): void {
  if (settings === undefined || savingHotkeys) return;
  if (capturing) {
    stopCapture();
    return;
  }
  capturing = true;
  hideError();
  hotkeyHint.hidden = false;
  window.addEventListener("keydown", onCaptureKey, true);
}

/** Build the next Settings from the copy we hold, send it, and adopt what
 * the core returns (spec 6.1: save_settings answers with the saved copy).
 * The core registers the new binding in the same call, so it works at
 * once (FR-52, spec 7.11). */
async function saveRecordingMode(binding: string): Promise<void> {
  const current = settings;
  if (current === undefined) return;
  const next: Settings = {
    ...current,
    hotkeys: { ...current.hotkeys, recordingMode: binding },
  };
  const hadFocus = document.activeElement === hotkeyChange;
  savingHotkeys = true;
  renderControls();
  try {
    settings = await saveSettings(next);
    hideError();
  } catch (e) {
    // A clash, a key that needs Ctrl/Alt/Win, or a registration failure:
    // the message is the core's own text and the old binding stays.
    showError(e instanceof Error ? e.message : String(e));
  } finally {
    savingHotkeys = false;
    renderHotkey();
    renderControls();
    restoreFocus(hotkeyChange, hadFocus);
  }
}

/** Start with Windows (FR-47, FR-52): the same full-object save. A failed
 * save leaves the checkbox reading the stored value again. */
async function saveStartWithWindows(): Promise<void> {
  const current = settings;
  if (current === undefined) return;
  const next: Settings = {
    ...current,
    general: { ...current.general, startWithWindows: startWithWindows.checked },
  };
  const hadFocus = document.activeElement === startWithWindows;
  savingGeneral = true;
  renderControls();
  try {
    settings = await saveSettings(next);
    hideError();
  } catch (e) {
    showError(e instanceof Error ? e.message : String(e));
  } finally {
    savingGeneral = false;
    renderControls();
    restoreFocus(startWithWindows, hadFocus);
  }
}

/** Done closes the window; onboarding.rs marks the profile on destroy, so
 * this and the title bar X end the wizard the same way (FR-52). */
function finish(): void {
  getCurrentWindow()
    .close()
    .catch((e: unknown) => {
      showError(e instanceof Error ? e.message : String(e));
    });
}

backButton.addEventListener("click", () => {
  if (step === 0) return;
  stopCapture();
  hideError();
  step -= 1;
  render(true);
});

nextButton.addEventListener("click", () => {
  if (step === LAST_STEP) {
    finish();
    return;
  }
  stopCapture();
  hideError();
  step += 1;
  render(true);
});

hotkeyChange.addEventListener("click", startCapture);

startWithWindows.addEventListener("change", () => {
  void saveStartWithWindows();
});

// Keep the snapshot current (spec 6.1: the core is the writer): a save
// from the Settings window while onboarding is open must not be
// overwritten by onboarding's next full-object save. While one of this
// page's own saves is in flight, its returned copy is the authoritative
// update instead.
void onSettingsChanged((next) => {
  if (!savingHotkeys && !savingGeneral) {
    settings = next;
    renderHotkey();
    renderControls();
  }
});

// Step 3's text depends on the OS, not on a setting. A failed read leaves
// both blocks hidden and the shared overflow line in the markup showing.
void getState().then(
  (state) => {
    trayWin11.hidden = !state.os.isWindows11;
    trayWin10.hidden = state.os.isWindows11;
  },
  (e: unknown) => {
    showError(e instanceof Error ? e.message : String(e));
  },
);

void getSettings().then(
  (initial) => {
    settings = initial;
    renderHotkey();
    renderControls();
  },
  (e: unknown) => {
    showError(e instanceof Error ? e.message : String(e));
  },
);

// The step chrome renders before the reads settle (the window must not
// appear blank); renderControls then applies whatever state they left.
render(false);
renderControls();
