// Fluent UI System Icons (@fluentui/svg-icons 1.1.344,
// https://github.com/microsoft/fluentui-system-icons): the 20 px filled
// glyphs on the switch tiles, the 20 px regular ones everywhere else.
// Inlined as path data so they render through createElementNS: no
// network, no innerHTML, CSP-safe (D-082).
//
// MIT License
//
// Copyright (c) 2020 Microsoft Corporation
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

const ICONS = {
  // Switch tiles (filled).
  "alert": "M12.45 16a2.5 2.5 0 0 1-4.9 0zM10 2a5.9 5.9 0 0 1 5.98 5.36l.02.22v3.82l.95 2.6.03.08.01.09v.08c0 .32-.19.6-.51.71l-.12.03-.11.01H3.75a.75.75 0 0 1-.74-.76v-.12l.04-.12L4 11.4V7.57A5.9 5.9 0 0 1 10 2",
  "dark-theme": "M10 3.5a6.5 6.5 0 1 1 0 13zM10 2a8 8 0 1 0 0 16 8 8 0 0 0 0-16",
  "drink-coffee": "M5 2.5a.5.5 0 0 0-1 0c0 1 .69 1.52 1.16 1.87l.04.03c.53.4.8.63.8 1.1a.5.5 0 0 0 1 0c0-1-.69-1.52-1.16-1.87L5.8 3.6c-.53-.4-.8-.63-.8-1.1M3.93 7a.93.93 0 0 0-.93.93V12a6 6 0 0 0 11.2 3H16a3 3 0 1 0 0-6h-1V7.93a.93.93 0 0 0-.93-.93zM16 14h-1.34q.34-.95.34-2v-2h1a2 2 0 1 1 0 4M7.5 2c.28 0 .5.22.5.5 0 .47.27.7.8 1.1l.04.03C9.31 3.98 10 4.5 10 5.5a.5.5 0 0 1-1 0c0-.47-.27-.7-.8-1.1l-.04-.03C7.69 4.02 7 3.5 7 2.5c0-.28.22-.5.5-.5m3.5.5a.5.5 0 0 0-1 0c0 1 .69 1.52 1.16 1.87l.04.03c.53.4.8.63.8 1.1a.5.5 0 0 0 1 0c0-1-.69-1.52-1.16-1.87l-.04-.03c-.53-.4-.8-.63-.8-1.1",
  "eye-off": "M2.85 2.15a.5.5 0 1 0-.7.7l3.5 3.5a8 8 0 0 0-3.37 5.05.5.5 0 1 0 .98.2 7 7 0 0 1 3.1-4.53l1.59 1.59a3.5 3.5 0 1 0 4.88 4.88l4.32 4.31a.5.5 0 0 0 .7-.7zM10.12 8l3.37 3.37A3.5 3.5 0 0 0 10.12 8M7.53 5.41l.8.8Q9.14 6 10 6c3.2 0 6.06 2.33 6.74 5.6a.5.5 0 1 0 .98-.2A8 8 0 0 0 10 5q-1.3 0-2.47.41",
  "grid": "M7.5 11c.83 0 1.5.67 1.5 1.5v4c0 .83-.67 1.5-1.5 1.5h-4A1.5 1.5 0 0 1 2 16.5v-4c0-.83.67-1.5 1.5-1.5zm9 0c.83 0 1.5.67 1.5 1.5v4c0 .83-.67 1.5-1.5 1.5h-4a1.5 1.5 0 0 1-1.5-1.5v-4c0-.83.67-1.5 1.5-1.5zm-9-9C8.33 2 9 2.67 9 3.5v4C9 8.33 8.33 9 7.5 9h-4A1.5 1.5 0 0 1 2 7.5v-4C2 2.67 2.67 2 3.5 2zm9 0c.83 0 1.5.67 1.5 1.5v4c0 .83-.67 1.5-1.5 1.5h-4A1.5 1.5 0 0 1 11 7.5v-4c0-.83.67-1.5 1.5-1.5z",
  "headphones": "M3 10a7 7 0 0 1 14 0v1h-3.5a.5.5 0 0 0-.5.5v6c0 .28.22.5.5.5H16a2 2 0 0 0 2-2v-6a8 8 0 1 0-16 0v6c0 1.1.9 2 2 2h2.5a.5.5 0 0 0 .5-.5v-6a.5.5 0 0 0-.5-.5H3z",
  "image": "M6 3a3 3 0 0 0-3 3v8q.01.99.56 1.74l5.39-5.3a1.5 1.5 0 0 1 2.1 0l5.4 5.3c.34-.49.55-1.1.55-1.74V6a3 3 0 0 0-3-3zm0 14q-.99-.01-1.73-.55l5.38-5.3c.2-.2.5-.2.7 0l5.38 5.3q-.75.54-1.73.55zm6.5-8.25a1.25 1.25 0 1 1 0-2.5 1.25 1.25 0 0 1 0 2.5",
  "keyboard": "M3.5 4C2.67 4 2 4.67 2 5.5v8c0 .83.67 1.5 1.5 1.5h13c.83 0 1.5-.67 1.5-1.5v-8c0-.83-.67-1.5-1.5-1.5zm2.75 3.25a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0m6 0a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0M5 12.5c0-.28.22-.5.5-.5h9a.5.5 0 0 1 0 1h-9a.5.5 0 0 1-.5-.5M14.5 8a.75.75 0 1 1 0-1.5.75.75 0 0 1 0 1.5M7 10.5A.75.75 0 1 1 7 9a.75.75 0 0 1 0 1.5m3.75-.75a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0m2.26.75A.75.75 0 1 1 13 9a.75.75 0 0 1 0 1.5M9.25 7.25a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0",
  "mic": "M5.5 10a.5.5 0 0 0-1 0 5.5 5.5 0 0 0 5 5.48v2.02a.5.5 0 0 0 1 0v-2.02c2.8-.26 5-2.61 5-5.48a.5.5 0 0 0-1 0 4.5 4.5 0 1 1-9 0m7.5 0a3 3 0 0 1-6 0V5a3 3 0 0 1 6 0z",
  "panel-bottom": "M2 6a3 3 0 0 1 3-3h10a3 3 0 0 1 3 3v7a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3zm3-2a2 2 0 0 0-2 2v5h14V6a2 2 0 0 0-2-2z",
  "rectangle-landscape-sparkle": "m14.88 1.28.35 1.07a2.2 2.2 0 0 0 1.4 1.4l1.07.35h.02a.42.42 0 0 1 0 .8l-1.07.35a2.2 2.2 0 0 0-1.4 1.4l-.35 1.07a.42.42 0 0 1-.8 0l-.35-1.07a2.2 2.2 0 0 0-1.4-1.4l-1.07-.35a.42.42 0 0 1 0-.8l1.07-.35a2.2 2.2 0 0 0 1.38-1.4l.35-1.07a.42.42 0 0 1 .8 0m4.9 7.93-.76-.25a1.6 1.6 0 0 1-1-1l-.25-.76a.3.3 0 0 0-.57 0l-.25.77a1.6 1.6 0 0 1-.98 1l-.77.24a.3.3 0 0 0 0 .57l.77.25a1.6 1.6 0 0 1 1 1l.24.77a.3.3 0 0 0 .58 0l.24-.77a1.6 1.6 0 0 1 1-1l.77-.24a.3.3 0 0 0 0-.57zm-3.02 3.56q.32.23.74.23.26 0 .5-.08V13a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3h5.09a1.4 1.4 0 0 0 .17 1.32q.26.37.71.53l1.06.35q.22.06.47.29c.11.11.2.24.3.47l.35 1.1q.13.36.52.68.22.15.45.21a1.3 1.3 0 0 0 .77 1.79l.77.25q.12.05.22.14t.14.22l.26.8q.15.38.48.62",
  "weather-moon": "M16.36 14a7.98 7.98 0 0 1-13.48.54.6.6 0 0 1 .29-.9c3-1.08 4.61-2.33 5.55-4.11.98-1.88 1.24-3.94.55-6.76a.6.6 0 0 1 .61-.73A7.98 7.98 0 0 1 16.36 14",
  "weather-sunny-low": "M5.66 11a4.5 4.5 0 0 1 4.38-5.47A4.48 4.48 0 0 1 14.4 11h3.09a.5.5 0 0 1 0 1h-15a.5.5 0 0 1 0-1zm4.47 7a1 1 0 0 1-.21 0zM5.09 4.38 5 4.32a.5.5 0 0 0-.64.77l.86.86.07.05a.5.5 0 0 0 .63-.76zm10.66.64a.5.5 0 0 0-.77-.64l-.85.86-.06.07a.5.5 0 0 0 .76.64l.86-.86zm-5.22-2.59a.5.5 0 0 0-1 .1v1.2l.02.1a.5.5 0 0 0 .99-.1v-1.2zM8.5 16a.5.5 0 0 0 0 1h3a.5.5 0 0 0 0-1zM5 14c0-.28.22-.5.5-.5h9a.5.5 0 0 1 0 1h-9A.5.5 0 0 1 5 14",
  "window-multiple": "M4.5 2A2.5 2.5 0 0 0 2 4.5v8A2.5 2.5 0 0 0 4.5 15h8a2.5 2.5 0 0 0 2.5-2.5v-8A2.5 2.5 0 0 0 12.5 2zM3 12.5V6h11v6.5c0 .83-.67 1.5-1.5 1.5h-8A1.5 1.5 0 0 1 3 12.5M7.5 18a2.5 2.5 0 0 1-2.45-2h1.04c.2.58.76 1 1.41 1H14a3 3 0 0 0 3-3V7.5c0-.65-.42-1.2-1-1.41V5.05a2.5 2.5 0 0 1 2 2.45V14a4 4 0 0 1-4 4z",
  // Everything else (regular).
  "checkmark": "M3.37 10.17a.5.5 0 0 0-.74.66l4 4.5c.19.22.52.23.72.02l10.5-10.5a.5.5 0 0 0-.7-.7L7.02 14.27z",
  "chevron-down": "M15.85 7.65c.2.2.2.5 0 .7l-5.46 5.49a.55.55 0 0 1-.78 0L4.15 8.35a.5.5 0 1 1 .7-.7L10 12.8l5.15-5.16c.2-.2.5-.2.7 0",
  "chevron-left": "M12.35 15.85a.5.5 0 0 1-.7 0L6.16 10.4a.55.55 0 0 1 0-.78l5.49-5.46a.5.5 0 1 1 .7.7L7.2 10l5.16 5.15c.2.2.2.5 0 .7",
  "chevron-right": "M7.65 4.15c.2-.2.5-.2.7 0l5.49 5.46c.21.22.21.57 0 .78l-5.49 5.47a.5.5 0 0 1-.7-.71L12.8 10 7.65 4.85a.5.5 0 0 1 0-.7",
  "settings": "M1.91 7.38A8.5 8.5 0 0 1 3.7 4.3a.5.5 0 0 1 .54-.13l1.92.68a1 1 0 0 0 1.32-.76l.36-2a.5.5 0 0 1 .4-.4 9 9 0 0 1 3.55 0q.32.08.38.4l.37 2a1 1 0 0 0 1.32.76l1.92-.68a.5.5 0 0 1 .54.13 8.5 8.5 0 0 1 1.78 3.08q.08.31-.15.54l-1.56 1.32a1 1 0 0 0 0 1.52l1.56 1.32a.5.5 0 0 1 .15.54 8.5 8.5 0 0 1-1.78 3.08.5.5 0 0 1-.54.13l-1.92-.68a1 1 0 0 0-1.32.76l-.37 2a.5.5 0 0 1-.38.4 8.5 8.5 0 0 1-3.56 0 .5.5 0 0 1-.39-.4l-.36-2a1 1 0 0 0-1.32-.76l-1.92.68a.5.5 0 0 1-.54-.13 8.5 8.5 0 0 1-1.78-3.08.5.5 0 0 1 .15-.54l1.56-1.32a1 1 0 0 0 0-1.52L2.06 7.92a.5.5 0 0 1-.15-.54m1.06 0 1.3 1.1a2 2 0 0 1 0 3.04l-1.3 1.1q.45 1.19 1.25 2.16l1.6-.58a2 2 0 0 1 2.63 1.53l.3 1.67a8 8 0 0 0 2.5 0l.3-1.67a2 2 0 0 1 2.64-1.53l1.6.58a8 8 0 0 0 1.24-2.16l-1.3-1.1a2 2 0 0 1 0-3.04l1.3-1.1a8 8 0 0 0-1.25-2.16l-1.6.58a2 2 0 0 1-2.63-1.53l-.3-1.67a8 8 0 0 0-2.5 0l-.3 1.67A2 2 0 0 1 5.81 5.8l-1.6-.58a8 8 0 0 0-1.24 2.16M7.5 10a2.5 2.5 0 1 1 5 0 2.5 2.5 0 0 1-5 0m1 0a1.5 1.5 0 1 0 3 0 1.5 1.5 0 0 0-3 0",
  "timer": "M7.5 2a.5.5 0 0 0 0 1h4a.5.5 0 0 0 0-1zm7.66 1.93a.5.5 0 1 0-.71.7l1.41 1.42a.5.5 0 1 0 .71-.7zM9.5 6a.5.5 0 0 0-.5.5v5a.5.5 0 0 0 1 0v-5a.5.5 0 0 0-.5-.5m0 12a7 7 0 1 0 0-14 7 7 0 0 0 0 14m0-1a6 6 0 1 1 0-12 6 6 0 0 1 0 12",
  "timer-off": "M4.96 5.67 2.15 2.85a.5.5 0 1 1 .7-.7l15 15a.5.5 0 0 1-.7.7l-2.32-2.31A6.98 6.98 0 0 1 2.5 11c0-2.13.96-4.05 2.46-5.33m9.16 9.16L10 10.7v.79a.5.5 0 0 1-1 0V9.7L5.67 6.39A5.99 5.99 0 0 0 9.5 17c1.86 0 3.52-.85 4.62-2.17M9 6.5v.38l1 1V6.5a.5.5 0 0 0-1 0m6.5 4.5a6 6 0 0 1-.35 2.03l.77.77q.57-1.3.58-2.8a7 7 0 0 0-9.8-6.42l.77.77A6 6 0 0 1 15.5 11m-8-9a.5.5 0 0 0 0 1h4a.5.5 0 0 0 0-1zm7.66 1.93a.5.5 0 1 0-.71.7l1.41 1.42a.5.5 0 1 0 .71-.7z",
} satisfies Record<string, string>;

export type IconName = keyof typeof ICONS;

const SVG_NS = "http://www.w3.org/2000/svg";

/** A 20x20 filled glyph that inherits currentColor. */
export function icon(name: IconName, size = 16): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", "0 0 20 20");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("fill", "currentColor");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  const path = document.createElementNS(SVG_NS, "path");
  path.setAttribute("d", ICONS[name]);
  svg.append(path);
  return svg;
}
