// The switch catalogue and backdrop presets the dropdown and the settings
// window share (spec 7.5 names, D-087 presets). Rust validates the ids;
// catalog.test.ts keeps the built-in list in step with settings.rs.
import type { Availability, GradientPreset, SwitchId } from "./ipc";

// Display names (FR-3's names, spec 7.5 catalogue).
export const LABELS: Record<SwitchId, string> = {
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

export const UNAVAILABLE_LABELS: Record<Availability, string> = {
  available: "",
  "needs-windows-11": "Needs Windows 11",
  "probe-failed": "Unavailable",
  "not-built": "Not built yet",
};

/** The Recording Mode actions in spec 7.1 apply order (FR-40). */
export const ACTION_IDS: SwitchId[] = [
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

/** Gradient presets (D-087) with the names the tiles announce. */
export const GRADIENTS: { id: GradientPreset; name: string }[] = [
  { id: "dusk", name: "Dusk" },
  { id: "ocean", name: "Ocean" },
  { id: "ember", name: "Ember" },
  { id: "forest", name: "Forest" },
  { id: "violet", name: "Violet" },
  { id: "slate", name: "Slate" },
];

/** Preset colours (FR-46). The fills live in shared/swatches.css: inline
 * styles are banned by the CSP (D-082). */
export const PRESET_COLOURS: {
  value: string;
  name: string;
  className: string;
}[] = [
  { value: "#202020", name: "Charcoal", className: "swatch-charcoal" },
  { value: "#000000", name: "Black", className: "swatch-black" },
  { value: "#ffffff", name: "White", className: "swatch-white" },
  { value: "#3a3d45", name: "Slate", className: "swatch-slate" },
  { value: "#1e4d3b", name: "Forest", className: "swatch-forest" },
  { value: "#4b2a80", name: "Plum", className: "swatch-plum" },
];

/** The built-in backdrop pictures (settings.rs BUILTIN_BACKDROPS), served
 * from /backdrops/<id>.jpg with /backdrops/thumbs/<id>.jpg tiles. */
export const BUILTIN_BACKDROPS: { id: string; name: string }[] = [
  { id: "graphite", name: "Graphite" },
  { id: "midnight", name: "Midnight" },
  { id: "nebula", name: "Nebula" },
  { id: "orchid", name: "Orchid" },
  { id: "sakura", name: "Sakura" },
  { id: "sunset", name: "Sunset" },
  { id: "ember", name: "Ember" },
  { id: "dune", name: "Dune" },
  { id: "meadow", name: "Meadow" },
  { id: "aurora", name: "Aurora" },
  { id: "lagoon", name: "Lagoon" },
  { id: "glacier", name: "Glacier" },
];

export const builtinThumb = (id: string) => `/backdrops/thumbs/${id}.jpg`;
export const builtinPicture = (id: string) => `/backdrops/${id}.jpg`;

/** Blur presets (px) for the backdrop picture; 20 is the default. */
export const BLUR_PRESETS: { label: string; px: number }[] = [
  { label: "None", px: 0 },
  { label: "Low", px: 8 },
  { label: "Medium", px: 20 },
  { label: "High", px: 36 },
];
