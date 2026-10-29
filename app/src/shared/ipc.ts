// Typed wrappers for the IPC commands and events (spec 6.1, 6.2).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ---- AppState (spec 5.4) ----

export type SwitchId =
  | "dnd"
  | "desktop-icons"
  | "other-apps"
  | "taskbar"
  | "tray-icons"
  | "backdrop"
  | "keep-awake"
  | "alert-sounds"
  | "dark-mode"
  | "night-light"
  | "screen-saver"
  | "microphone"
  | "lock-keyboard"
  | "bluetooth";

export type Availability =
  | "available"
  | "needs-windows-11"
  | "probe-failed"
  | "not-built";

export type SessionStatus =
  | "idle"
  | "countdown"
  | "applying"
  | "active"
  | "restoring";

export interface AppState {
  session: {
    status: SessionStatus;
    startedAt?: string;
    endsAt?: string;
    applied: SwitchId[];
    skipped: { id: SwitchId; reason: string }[];
    failed: { id: SwitchId; reason: string }[];
  };
  switches: {
    id: SwitchId;
    on: boolean;
    availability: Availability;
    experimental: boolean;
    offAt?: string;
    detail?: string;
  }[];
  hotkey: { binding: string; registered: boolean };
  os: { build: number; isWindows11: boolean };
  update?: { version: string };
}

// ---- Settings (spec 5.1) ----

/** A backdrop gradient preset id (D-087); the CSS classes live in
 * overlays/backdrop.css. */
export type GradientPreset =
  | "dusk"
  | "ocean"
  | "ember"
  | "forest"
  | "violet"
  | "slate";

export type BackdropStyle =
  | "color"
  | "gradient"
  | "picture"
  | "wallpaper"
  | "builtin";

export interface Settings {
  schemaVersion: number;
  recordingMode: {
    actions: SwitchId[];
    countdown: boolean;
    timeLimitMinutes: number | null;
  };
  hotkeys: { recordingMode: string; switches: Partial<Record<SwitchId, string>> };
  dropdown: { shown: SwitchId[]; more: SwitchId[]; hidden: SwitchId[] };
  apps: { staysOnScreen: string[]; hidden: string[] };
  trayIcons: { visible: string[] };
  backdrop: {
    style: BackdropStyle;
    color: string;
    gradient: GradientPreset | null;
    picture: string | null;
    /** A built-in picture id (shared/backdrops.ts) when style = builtin. */
    builtin: string | null;
    blur: number;
    dim: number;
    monitors: "all" | "front";
  };
  keepAwake: { defaultMinutes: number; allowDisplayOff: boolean };
  bluetooth: { deviceId: string | null };
  general: {
    startWithWindows: boolean;
    checkForUpdates: boolean;
    onboardingDone: boolean;
  };
}

/** One Bluetooth audio device from `list_bt_devices` (FR-38). */
export interface BtDevice {
  id: string;
  name: string;
  connected: boolean;
  battery: number | null;
}

/** The `offAfter` argument of `set_switch` (spec 6.1, D-051): a preset
 * string, a minute count, or an RFC 3339 time in the future. */
export type OffAfter =
  | "15m"
  | "1h"
  | "4h"
  | "tomorrow"
  | { minutes: number }
  | { at: string };

// ---- Commands (spec 6.1) ----

export type IpcErrorCode =
  | "SESSION_ACTIVE"
  | "NO_SESSION"
  | "NOT_AVAILABLE"
  | "INVALID_ARGUMENT"
  | "OS_ERROR";

export class IpcError extends Error {
  readonly code: IpcErrorCode;

  constructor(code: IpcErrorCode, message: string) {
    super(message);
    this.code = code;
  }
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (e) {
    // The core rejects with { code, message } (spec 6.1).
    const shape = e as { code?: IpcErrorCode; message?: string };
    if (shape && shape.code && shape.message !== undefined) {
      throw new IpcError(shape.code, shape.message);
    }
    throw e;
  }
}

export const getState = () => call<AppState>("get_state");
export const getSettings = () => call<Settings>("get_settings");
export const startRecording = () => call<AppState>("start_recording");
export const stopRecording = () => call<AppState>("stop_recording");
export const toggleRecording = () => call<AppState>("toggle_recording");
export const setSwitch = (id: SwitchId, on: boolean, offAfter?: OffAfter) =>
  call<AppState>("set_switch", { id, on, offAfter });
export const unlockKeyboard = () => call<AppState>("unlock_keyboard");
export const listBtDevices = () => call<BtDevice[]>("list_bt_devices");
export const setBluetoothDevice = (id: string | null) =>
  call<Settings>("set_bluetooth_device", { id });
export const setKeepAwakeOptions = (allowDisplayOff: boolean) =>
  call<Settings>("set_keep_awake_options", { allowDisplayOff });
export const saveSettings = (settings: Settings) =>
  call<Settings>("save_settings", { settings });
export const extendRecording = (minutes: number) =>
  call<AppState>("extend_recording", { minutes });

/** One running app for the app shelves (FR-43): exe basename + name. */
export interface AppInfo {
  exe: string;
  name: string;
}

/** One third-party tray icon for the Visible shelf (FR-30). */
export interface TrayIconInfo {
  key: string;
  exe: string;
  promoted: boolean;
}

export const listApps = () => call<AppInfo[]>("list_apps");
export const listTrayIcons = () => call<TrayIconInfo[]>("list_tray_icons");
/** Settings sections (settings.html's `section-<id>`). */
export type SettingsSection =
  | "general"
  | "recording"
  | "switches"
  | "hotkeys"
  | "backdrop"
  | "about";

export const openSettings = (section?: SettingsSection) =>
  call<void>("open_settings", { section });
/** Fit the dropdown window to its content height (CSS px). */
export const fitDropdown = (height: number) =>
  call<void>("fit_dropdown", { height });
export const openOnboarding = () => call<void>("open_onboarding");

/** `check_update`'s result (spec 6.1). */
export interface UpdateCheck {
  available: boolean;
  version?: string;
}

/** Ask GitHub for a newer release (FR-22); a found one also lands in
 * AppState.update. */
export const checkUpdate = () => call<UpdateCheck>("check_update");
/** Install the found update; SESSION_ACTIVE while anything is applied.
 * On success the app exits and the installer starts the new version. */
export const installUpdate = () => call<void>("install_update");
/** Copy the diagnostic report to the clipboard (FR-24). */
export const copyDiagnostics = () => call<void>("copy_diagnostics");
export const pickBackdropPicture = () =>
  call<string | null>("pick_backdrop_picture");

/** Raw image bytes for the backdrop picture/wallpaper; an empty body
 * means "no usable image, fall back to the colour". */
export async function getBackdropImage(): Promise<ArrayBuffer> {
  const response = await invoke<ArrayBuffer>("get_backdrop_image");
  return response;
}

export const getBackdropStyle = () =>
  call<Settings["backdrop"]>("get_backdrop_style");

// ---- Events (spec 6.2) ----

export function onStateChanged(
  handler: (state: AppState) => void,
): Promise<() => void> {
  return listen<AppState>("state-changed", (event) => handler(event.payload));
}

/** An open settings window is asked to show a section. */
export function onSettingsSection(
  handler: (section: SettingsSection) => void,
): Promise<() => void> {
  return listen<SettingsSection>("settings-section", (event) =>
    handler(event.payload),
  );
}

export function onSettingsChanged(handler: (settings: Settings) => void): Promise<() => void> {
  return listen<Settings>("settings-changed", (event) => handler(event.payload));
}

/** The 3·2·1 countdown ticks (FR-41). */
export function onCountdown(
  handler: (secondsLeft: 3 | 2 | 1) => void,
): Promise<() => void> {
  return listen<{ secondsLeft: 3 | 2 | 1 }>("countdown", (event) =>
    handler(event.payload.secondsLeft),
  );
}
