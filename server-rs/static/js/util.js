// Small DOM and formatting helpers shared by every Studio module.

export const $ = (id) => document.getElementById(id);

// Small-screen check (phones): the editor switches to one panel at a
// time there. Guarded for environments without matchMedia.
export const isNarrowScreen = () =>
  typeof window.matchMedia === "function" &&
  window.matchMedia("(max-width: 860px)").matches;
export const clone = (value) => structuredClone(value);

export const PORTRAIT_WIDTH = 240;
export const PORTRAIT_HEIGHT = 320;
export const LANDSCAPE_WIDTH = 320;
export const LANDSCAPE_HEIGHT = 240;
export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 3;
export const clampZoom = (z) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));

export const TYPE_META = {
  text: { label: "Text", icon: "type" },
  clock: { label: "Clock", icon: "clock" },
  "external-text": { label: "External text", icon: "external" },
  button: { label: "Button", icon: "button-rect" },
  "design-link": { label: "Screen button", icon: "enter" },
  image: { label: "Image", icon: "image" },
  "color-block": { label: "Color block", icon: "square-fill" },
  "ha-state": { label: "HA sensor", icon: "activity" },
  "ha-toggle": { label: "HA toggle", icon: "power" },
  "touch-zone": { label: "Touch zone", icon: "pointer" },
  script: { label: "Script", icon: "code" },
  screen: { label: "Screen embed", icon: "screen-embed" },
  gauge: { label: "Gauge", icon: "gauge" },
  qr: { label: "QR code", icon: "qr" },
  shape: { label: "Shape", icon: "shapes" },
};

// Inline-SVG icon from the sprite defined in index.html (no remote assets).
// `name` is a symbol id suffix: icon("trash") renders <use href="#i-trash">.
export const icon = (name) =>
  `<svg class="ic" aria-hidden="true"><use href="#i-${name}"></use></svg>`;

export function compactNumber(value) {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1000) return `${(value / 1000).toFixed(1)}k`;
  return String(Math.round(value || 0));
}

export function wifiSignalLabel(rssi) {
  if (rssi === null || rssi === undefined || rssi <= -126) return "—";
  if (rssi >= -55) return `${rssi} dBm · Excellent`;
  if (rssi >= -67) return `${rssi} dBm · Good`;
  if (rssi >= -75) return `${rssi} dBm · Fair`;
  return `${rssi} dBm · Weak`;
}

export function relativeLastSeen(value) {
  if (!value) return "Never seen";
  const seconds = Math.max(0, Math.round((Date.now() - new Date(value).getTime()) / 1000));
  if (seconds < 60) return "Seen just now";
  if (seconds < 3600) return `Seen ${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `Seen ${Math.floor(seconds / 3600)}h ago`;
  return `Seen ${Math.floor(seconds / 86400)}d ago`;
}
