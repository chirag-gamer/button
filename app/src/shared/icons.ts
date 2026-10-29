// Lucide icons (lucide-static 1.64.0, https://lucide.dev), inlined as
// element lists so they render through createElementNS: no network, no
// innerHTML, CSP-safe (D-082).
//
// ISC License
//
// Copyright (c) 2026 Lucide Icons and Contributors
//
// Permission to use, copy, modify, and/or distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
// ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
// OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
//
// check, chevron-down, chevron-left, chevron-right, headphones and moon
// derive from Feather: The MIT License (MIT), Copyright (c)
// 2013-present Cole Bemis. Permission is hereby granted, free of charge, to
// any person obtaining a copy of this software and associated documentation
// files (the "Software"), to deal in the Software without restriction,
// including without limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of the Software, and
// to permit persons to whom the Software is furnished to do so, subject to
// the following conditions: The above copyright notice and this permission
// notice shall be included in all copies or substantial portions of the
// Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
// EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
// MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
// NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
// DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
// OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE
// USE OR OTHER DEALINGS IN THE SOFTWARE.

type IconNode = [string, Record<string, string>][];

const ICONS = {
  "app-window": [["rect", {"x": "2", "y": "4", "width": "20", "height": "16", "rx": "2"}], ["path", {"d": "M10 4v4"}], ["path", {"d": "M2 8h20"}], ["path", {"d": "M6 4v4"}]],
  "bell-off": [["path", {"d": "M10.268 21a2 2 0 0 0 3.464 0"}], ["path", {"d": "M17 17H4a1 1 0 0 1-.74-1.673C4.59 13.956 6 12.499 6 8a6 6 0 0 1 .258-1.742"}], ["path", {"d": "m2 2 20 20"}], ["path", {"d": "M8.668 3.01A6 6 0 0 1 18 8c0 2.687.77 4.653 1.707 6.05"}]],
  "check": [["path", {"d": "M20 6 9 17l-5-5"}]],
  "chevron-down": [["path", {"d": "m6 9 6 6 6-6"}]],
  "chevron-left": [["path", {"d": "m15 18-6-6 6-6"}]],
  "chevron-right": [["path", {"d": "m9 18 6-6-6-6"}]],
  "coffee": [["path", {"d": "M10 2v2"}], ["path", {"d": "M14 2v2"}], ["path", {"d": "M16 8a1 1 0 0 1 1 1v8a4 4 0 0 1-4 4H7a4 4 0 0 1-4-4V9a1 1 0 0 1 1-1h14a4 4 0 1 1 0 8h-1"}], ["path", {"d": "M6 2v2"}]],
  "eye-off": [["path", {"d": "M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"}], ["path", {"d": "M14.084 14.158a3 3 0 0 1-4.242-4.242"}], ["path", {"d": "M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"}], ["path", {"d": "m2 2 20 20"}]],
  "headphones": [["path", {"d": "M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"}]],
  "infinity": [["path", {"d": "M6 16c5 0 7-8 12-8a4 4 0 0 1 0 8c-5 0-7-8-12-8a4 4 0 1 0 0 8"}]],
  "keyboard": [["path", {"d": "M10 8h.01"}], ["path", {"d": "M12 12h.01"}], ["path", {"d": "M14 8h.01"}], ["path", {"d": "M16 12h.01"}], ["path", {"d": "M18 8h.01"}], ["path", {"d": "M6 8h.01"}], ["path", {"d": "M7 16h10"}], ["path", {"d": "M8 12h.01"}], ["rect", {"width": "20", "height": "16", "x": "2", "y": "4", "rx": "2"}]],
  "layout-grid": [["rect", {"width": "7", "height": "7", "x": "3", "y": "3", "rx": "1"}], ["rect", {"width": "7", "height": "7", "x": "14", "y": "3", "rx": "1"}], ["rect", {"width": "7", "height": "7", "x": "14", "y": "14", "rx": "1"}], ["rect", {"width": "7", "height": "7", "x": "3", "y": "14", "rx": "1"}]],
  "mic-off": [["path", {"d": "M12 19v3"}], ["path", {"d": "M15 9.34V5a3 3 0 0 0-5.68-1.33"}], ["path", {"d": "M16.95 16.95A7 7 0 0 1 5 12v-2"}], ["path", {"d": "M18.89 13.23A7 7 0 0 0 19 12v-2"}], ["path", {"d": "m2 2 20 20"}], ["path", {"d": "M9 9v3a3 3 0 0 0 5.12 2.12"}]],
  "monitor-play": [["path", {"d": "M15.033 9.44a.647.647 0 0 1 0 1.12l-4.065 2.352a.645.645 0 0 1-.968-.56V7.648a.645.645 0 0 1 .967-.56z"}], ["path", {"d": "M12 17v4"}], ["path", {"d": "M8 21h8"}], ["rect", {"x": "2", "y": "3", "width": "20", "height": "14", "rx": "2"}]],
  "moon": [["path", {"d": "M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401"}]],
  "panel-bottom": [["rect", {"width": "18", "height": "18", "x": "3", "y": "3", "rx": "2"}], ["path", {"d": "M3 15h18"}]],
  "settings": [["path", {"d": "M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"}], ["circle", {"cx": "12", "cy": "12", "r": "3"}]],
  "sun-moon": [["path", {"d": "M12 2v2"}], ["path", {"d": "M14.837 16.385a6 6 0 1 1-7.223-7.222c.624-.147.97.66.715 1.248a4 4 0 0 0 5.26 5.259c.589-.255 1.396.09 1.248.715"}], ["path", {"d": "M16 12a4 4 0 0 0-4-4"}], ["path", {"d": "m19 5-1.256 1.256"}], ["path", {"d": "M20 12h2"}]],
  "sunset": [["path", {"d": "M12 10V2"}], ["path", {"d": "m4.93 10.93 1.41 1.41"}], ["path", {"d": "M2 18h2"}], ["path", {"d": "M20 18h2"}], ["path", {"d": "m19.07 10.93-1.41 1.41"}], ["path", {"d": "M22 22H2"}], ["path", {"d": "m16 6-4 4-4-4"}], ["path", {"d": "M16 18a4 4 0 0 0-8 0"}]],
  "timer": [["line", {"x1": "10", "x2": "14", "y1": "2", "y2": "2"}], ["line", {"x1": "12", "x2": "15", "y1": "14", "y2": "11"}], ["circle", {"cx": "12", "cy": "14", "r": "8"}]],
  "wallpaper": [["path", {"d": "M12 17v4"}], ["path", {"d": "M8 21h8"}], ["path", {"d": "m9 17 6.1-6.1a2 2 0 0 1 2.81.01L22 15"}], ["circle", {"cx": "8", "cy": "9", "r": "2"}], ["rect", {"x": "2", "y": "3", "width": "20", "height": "14", "rx": "2"}]],
} satisfies Record<string, IconNode>;

export type IconName = keyof typeof ICONS;

const SVG_NS = "http://www.w3.org/2000/svg";

/** A 24x24 stroke icon that inherits currentColor. */
export function icon(name: IconName, size = 16): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "2");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  const nodes: IconNode = ICONS[name];
  for (const [tag, attrs] of nodes) {
    const child = document.createElementNS(SVG_NS, tag);
    for (const [k, v] of Object.entries(attrs)) child.setAttribute(k, v);
    svg.append(child);
  }
  return svg;
}
