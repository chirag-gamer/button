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

// ---- Settings subset the dropdown needs (spec 5.1) ----

export interface Settings {
  schemaVersion: number;
  recordingMode: { actions: SwitchId[] };
  dropdown: { shown: SwitchId[]; more: SwitchId[]; hidden: SwitchId[] };
  keepAwake: { defaultMinutes: number; allowDisplayOff: boolean };
  bluetooth: { deviceId: string | null };
}

/** One Bluetooth audio device from `list_bt_devices` (FR-38). */
export interface BtDevice {
  id: string;
  name: string;
  connected: boolean;
  battery: number | null;
}

/** The `offAfter` argument of `set_switch` (spec 6.1, D-051): a preset
 * string ("15m" | "1h" | "4h" | "tomorrow"), a minute count, or an RFC
 * 3339 time in the future. */
export type OffAfter = string | { minutes: number } | { at: string };

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

// ---- Events (spec 6.2) ----

export function onStateChanged(
  handler: (state: AppState) => void,
): Promise<() => void> {
  return listen<AppState>("state-changed", (event) => handler(event.payload));
}

export function onSettingsChanged(handler: (settings: Settings) => void): Promise<() => void> {
  return listen<Settings>("settings-changed", (event) => handler(event.payload));
}
