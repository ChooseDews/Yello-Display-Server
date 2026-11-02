// The in-Studio agent: a chat panel that talks to OpenRouter directly from
// the browser (the user's key never touches this server — it lives in
// localStorage and is sent only to openrouter.ai). The model drives the
// Studio through client-side tools that reuse the editor's own validated
// paths: write_design_json goes through the same /api/layout/validate the
// JSON editor uses, save_design is the Studio's own saveLayout().

import { $ } from "./util.js";
import { S, activePageIdFor, homePageIdOf, pageOf } from "./state.js";
import { fetchJson, postLayout } from "./api.js";
import { pushHistory } from "./history.js";
import { renderAll } from "./editor.js";
import { schedulePreview, syncCodeEditor } from "./canvas.js";
import { loadDesign, saveLayout } from "./views.js";
import { toast, appConfirm } from "./dialog.js";
import { saveChat, listChats, getChat, deleteChat } from "./chatstore.js";

const KEY_STORAGE = "yello-openrouter-key";
const MODEL_STORAGE = "yello-openrouter-model";
/** Which conversation is "current" across reloads: a chat id, or "new"
 *  after Clear (a fresh conversation must stay fresh after a reload
 *  instead of resurrecting the previous chat). */
const ACTIVE_CHAT_KEY = "yello-agent-active-chat";
let chatEpoch = 0;
// Any OpenAI-compatible endpoint works; override via localStorage
// "yello-openrouter-base" (used by the smoke tests with a mock server).
const API_BASE = localStorage.getItem("yello-openrouter-base") || "https://openrouter.ai/api/v1";
const MAX_TOOL_ROUNDS = 10;

let initialized = false;
let sending = false;
let aborter = null;
let dockOpen = false;
let panelMode = localStorage.getItem("yello-agent-panel-mode") === "float" ? "float" : "dock";

const clampNumber = (value, min, max) => Math.min(max, Math.max(min, value));

function floatGeometry() {
  try {
    const saved = JSON.parse(localStorage.getItem("yello-agent-panel-float") || "null");
    if (saved && saved.w && saved.h) {
      return {
        w: clampNumber(saved.w, 320, window.innerWidth - 16),
        h: clampNumber(saved.h, 360, window.innerHeight - 16),
        x: clampNumber(saved.x, 0, window.innerWidth - 120),
        y: clampNumber(saved.y, 0, window.innerHeight - 80),
      };
    }
  } catch { /* fall through to the default */ }
  const w = Math.min(480, window.innerWidth - 32);
  const h = Math.min(720, window.innerHeight - 100);
  return { x: window.innerWidth - w - 24, y: 84, w, h };
}

function applyPanelGeometry(inPanel) {
  const panel = $("agent-panel");
  if (!panel) return;
  if (!inPanel) {
    panel.removeAttribute("style");
    return;
  }
  if (panelMode === "dock") {
    const width = clampNumber(Number(localStorage.getItem("yello-agent-dock-width")) || 372, 300, 760);
    panel.style.cssText = `width: ${width}px;`;
  } else {
    const g = floatGeometry();
    panel.style.cssText = `left: ${g.x}px; top: ${g.y}px; width: ${g.w}px; height: ${g.h}px; right: auto; bottom: auto;`;
  }
  const floatButton = $("agent-panel-float");
  if (floatButton) {
    floatButton.querySelector("use")?.setAttribute("href", panelMode === "float" ? "#i-screen-embed" : "#i-external");
    floatButton.setAttribute("aria-label", panelMode === "float" ? "Dock the agent panel" : "Undock the agent panel");
    floatButton.title = panelMode === "float" ? "Dock" : "Undock";
  }
}
const usageTotals = { in: 0, out: 0, cost: 0 };
const conversation = []; // OpenAI-format messages (system prompt added per request)

const isWide = () => window.matchMedia("(min-width: 1200px)").matches;

// On desktop the agent lives in a docked right-hand column: the setup and
// chat cards are moved between the page mount and the panel mount. On
// narrow screens the cards live on the Agent page instead.
export function placeAgent() {
  const docked = isWide() && dockOpen;
  document.body.classList.toggle("agent-dock", docked && panelMode === "dock");
  document.body.classList.toggle("agent-float", docked && panelMode === "float");
  applyPanelGeometry(docked);
  const mount = $(docked ? "agent-mount-panel" : "agent-mount-page");
  for (const id of ["agent-setup-card", "agent-chat-card"]) {
    const card = $(id);
    if (card && card.parentElement !== mount) mount.append(card);
  }
  // The provider status light follows the agent window: panel head when
  // docked/floating, chat card head on the Agent page.
  const pill = $("agent-status-pill");
  if (pill) {
    if (docked) {
      const anchor = $("agent-panel-float");
      if (pill.parentElement !== anchor.parentElement) anchor.parentElement.insertBefore(pill, anchor);
    } else {
      const head = document.querySelector(".agent-chat-head");
      const tools = head?.querySelector(".agent-head-tools");
      if (tools && pill.parentElement !== head) head.insertBefore(pill, tools);
    }
  }
  document.querySelectorAll('[data-route="agent"]').forEach((link) => {
    link.classList.toggle("active", docked || S.currentPage === "agent");
  });
}

function setDock(open) {
  dockOpen = open;
  placeAgent();
}

// Called from showPage: on a wide screen, toggle the dock and report that
// the navigation was handled (no page switch). Returns false on narrow
// screens so the normal Agent page opens.
export function tryDockAgent() {
  if (!isWide()) return false;
  initAgentPage();
  setDock(!dockOpen);
  return true;
}

const SYSTEM_PROMPT = `You are the agent built into Yello Studio, a designer for ESP32 touch displays (240x320 portrait, 320x240 landscape). You edit the design the user has open by calling tools. The screen is small and rendered server-side to pixels, so layouts must be simple, large, and legible.

## Design document (schema v2)
{ schemaVersion: 2, name, profile: "ili9341-240x320", orientation: "portrait"|"landscape", background: "#0a0a1e", idleHomeSeconds: 0, homePageId: "<page id>", pages: [...], sharedElements: [...], dataSources: [...], actions: [...] }
- pages: 1..50 pages, each { id, name, background, elements: [0..100 elements] }. Page ids are unique slugs; homePageId must name one of them. Portrait pages are 240 wide x 320 tall; landscape 320 x 240.
- Element: { id (unique slug), type, name, frame: {x, y, w, h} in page pixels, style: {...}, props: {...}, visible: true, locked: false }. Elements render in array order (later = on top).
- sharedElements: elements with an extra pages: ["pageId", ...] field; a shared element renders ONLY on the pages listed there.
- Colors are "#rrggbb". Common style keys: color, fontSize (number), align: "left"|"center"|"right", verticalAlign: "top"|"middle"|"bottom", radius (corner radius px).
- Text renders in Liberation Sans Bold with a monochrome emoji fallback: emoji and symbols like 🔥 ⚠ ✅ 🌡 render in the element's text color at about one character width each, so they can be used in text, clock formats, button labels, and script text() output. (Script label() is different: it uses an 8x8 ASCII bitmap font, so no emoji there — they become "?".)

## Element types (props; style beyond the common keys)
- text: props {text, wrap}. Plain label.
- clock: props {format (strftime, e.g. "%H:%M" or "%a %b %d"), timezone ("" = display local time)}.
- external-text: props {sourceId (a dataSources id), wrap}. Shows fetched text.
- color-block: style {color, radius}; no props. A filled rectangle.
- button: props {label, actionId (an actions id, or "" for none)}. style {color = background, fontSize, radius}. Fires its HTTP action when tapped.
- design-link: props {label, targetDesignId, targetPageId optional}. style like button. Navigates the screen to another design/page.
- image: props {src (URL), fit: "contain"|"cover"|"stretch"}.
- ha-state: props {entityId (e.g. "sensor.office_temperature"), attribute ("" = state), prefix, suffix, decimals, showUnit, refreshInterval (seconds), wrap}. Shows a Home Assistant entity's state, formatted.
- ha-toggle: props {entityId (a switch/light/etc.), label, refreshInterval}. style {color = on color, offColor, fontSize, radius}. Tapping toggles the entity in Home Assistant.
- screen: props {designId, pageId}. Embeds another design's page inside the frame (one level deep, view-only).
- gauge: a bar. props {sourceKind: "static"|"external"|"ha", value (for static), sourceId (for external), entityId (for ha), min, max, decimals, unit, showValue, refreshInterval}. style {color = bar, trackColor, fontSize}.
- qr: props {text (URL/text to encode)}. style {color = modules, bgColor}.
- shape: props {shape: "rect"|"circle"|"ellipse"}. style {color, radius, outlineColor, outlineWidth}.
- script: props {code, refreshInterval, wrap}. Runs the script language below; the most flexible element.

## dataSources and actions
- dataSources: [{ id, name, type: "http-text", url, interval (seconds, 5..86400) }] — the server fetches the URL as plain text on the interval; external-text elements and gauges (sourceKind "external") read it by sourceId.
- actions: [{ id, name, type: "http", method: "GET"|"POST", url, body }] — fired by button elements (props.actionId) when tapped on the device.

## Script language (script elements)
A small, bounded, Python-like language. The server re-runs the script on every refresh of the element (props.refreshInterval seconds). Code is capped at 8000 characters and about 3000 operations per run, so keep loops short. A script error renders in the element itself, in red, as "Script: <message>" — write carefully, there is no console.

Values and variables:
- Numbers (floats), strings ("..." or '...'), and the booleans True / False. Assign with name = expression; there are no declarations and no lists/dicts/functions of your own.
- width and height are preset to the element's frame size in px — lay out relative to them, never hardcode a page size.
- Operators: + - * / % ** and comparisons == != < <= > >=, plus and / or / not. + between two strings concatenates them; any other arithmetic on strings is an error. Division is float division.
- f-strings: f"Temp {value:.1f}" — {expr} inserts a value, {expr:.Nf} formats with N decimals, {expr:.N%} formats as a percentage (value * 100 with N decimals and a % sign). In design JSON the code is a JSON string: use double quotes for f-strings and single quotes inside them.

Statements:
- One statement per line; blocks are indented (spaces) after a line ending with ':'.
- if cond: / else: — there is NO elif; nest an if inside the else block instead.
- for x in range(n), range(a, b), or range(a, b, step): — the only loop form; the body is the indented block.
- # starts a comment to end of line.

Output — exactly one of two modes per script:
- Text mode: call text(s) and draw nothing. The string renders with the element's style (fontSize, color, align) and the wrap prop applies.
- Drawing mode: if the script calls any drawing built-in, the drawing IS the output (text() is ignored). The canvas is exactly the element's frame: origin at its top-left corner, x grows right, y grows down, units are px. Anything drawn outside the frame is clipped.

Built-in functions:
- ha("sensor.x") -> the entity's state (string, or number when the state is numeric). ha("sensor.x", "attribute") -> an attribute value. Missing or errored entities return the string "unavailable"; a missing attribute returns "unknown" — always guard numeric use: number(ha("sensor.x"), fallback).
- now("%H:%M") -> the local time formatted with strftime codes (default "%H:%M").
- number(x, default = 0) -> x as a number, or default when x is not numeric.
- clamp(x, lo, hi); min(a, b, ...); max(a, b, ...); abs(x); round(x) or round(x, digits); sqrt(x); sin(x), cos(x), tan(x) (radians).
- text(s) -> set the text-mode output (see above).
- clear("#rrggbb") -> fill the whole frame with a color. Colors are always "#rrggbb" strings.
- rect(x, y, w, h, color, radius = 0) -> filled rectangle (radius rounds the corners).
- circle(cx, cy, r, color) -> filled circle.
- line(x0, y0, x1, y1, color, width = 1) -> straight line.
- label(x, y, text, color = "#ffffff", size = 16, anchor = "la") -> draw text on the canvas. label uses a fixed bitmap font: every character cell is a square of G px, where G = 8 * round(size / 8) — sizes 4-11 give G = 8, 12-19 give 16, 20-27 give 24. EVERY character (digits, spaces, symbols) is exactly G px wide, so a string of N characters is N * G px wide — use that to position and center text exactly. anchor is two letters: horizontal l (x is the left edge) / c or m (x is the center) / r (x is the right edge); vertical a (y is the top) / m (y is the middle) / b (y is the bottom edge). Example anchors: "la" top-left, "mm" centered, "rb" bottom-right.

Examples (complete scripts):
Battery bar (drawing mode):
  value = number(ha("sensor.battery_level"), 72)
  clear("#18202b")
  rect(4, 4, width - 8, height - 8, "#273447", 6)
  rect(8, 8, (width - 16) * clamp(value, 0, 100) / 100, height - 16, "#f2c94c", 4)
  label(width / 2, height / 2, f"{value:.0f}%", "#ffffff", 14, "mm")
Door state (if/else, color changes with state):
  state = ha("binary_sensor.front_door")
  if state == "on":
      clear("#9b2c3b")
      label(width / 2, height / 2, "DOOR OPEN", "#ffffff", 15, "mm")
  else:
      clear("#1f7a4d")
      label(width / 2, height / 2, "Door closed", "#ffffff", 15, "mm")
Temperature readout (text mode — the element's own style does the layout):
  temp = number(ha("sensor.office_temperature"), 0)
  if temp == 0:
      text("Office temp unavailable")
  else:
      text(f"Office {temp:.1f}°C")
Sparkline (loop + line + a small label; size 11 = 8px glyphs, so the 5-char clock is 40px wide):
  clear("#121826")
  last_x = 0
  last_y = height / 2
  for point in range(1, 13):
      x = point * width / 12
      y = height / 2 + sin(point * 0.9) * height * 0.3
      line(last_x, last_y, x, y, "#6b9cff", 2)
      last_x = x
      last_y = y
  label(5, 5, now("%H:%M"), "#f2c94c", 11, "la")

## Home Assistant
HA elements and ha() read live entity states from the user's connected Home Assistant; the current entity list (id = state) is included in the context message when HA is connected. Only use entity ids from that list or already present in the design; never invent entity ids. ha-toggle works for entities in domains like light, switch, fan, cover, climate (anything with on/off). When Home Assistant is connected you also have list_home_assistant_sensors — call it when you need the full live entity list with names, states and units (the context message only carries a truncated summary).

## How to work
- This conversation starts with a copy of the open design's full Design JSON and a render of its active page. Work from that copy; call get_current_design to refresh it whenever the design may have changed since (after the user edits in the Studio, or when you need the ground truth again).
- To change the design, call write_design_json with the COMPLETE updated document as a JSON string. It is validated against the device schema and applied to the editor (the user can undo). If validation returns errors, fix them and retry.
- For a change to ONE existing element, prefer write_element_json: pass the element id and the complete updated element as a JSON object (get_element_json fetches the current one). Reserve write_design_json for structural changes (adding or removing elements or pages).
- To write or change a script element's code, use write_script_code with the code as one plain string. Never embed script code inside write_design_json or write_element_json payloads — nested inside that JSON, the script's quotes and newlines need double escaping and usually arrive broken.
- Then call view_screen and actually look at the render before saving: fix overlaps, clipped or unreadable text, and elements running off the page. view_screen renders the open design including your unsaved changes, with live values, and returns an image; pass pageId to inspect a different page, or designId + pageId for another design.
- After a successful edit (write_design_json, write_element_json, or write_script_code), call save_design in the same turn (saving pushes the design to the screens using it), unless the user asked only to draft or preview.
- Keep frames inside the page, avoid overlaps unless asked, give new elements unique ids, and prefer fontSize >= 12 for readable text on the small display.
- Be concise in chat. Summarize what you changed in one or two sentences; do not paste the whole JSON back.`;

const TOOLS = [
  {
    type: "function",
    function: {
      name: "get_current_design",
      description: "Return the design currently open in the Studio editor, as a JSON document, plus its design id and name.",
      parameters: { type: "object", properties: {}, additionalProperties: false },
    },
  },
  {
    type: "function",
    function: {
      name: "write_design_json",
      description: "Replace the current design with a complete schema-v2 document (JSON string). Validated against the device schema and applied to the editor; returns a summary or the validation errors.",
      parameters: {
        type: "object",
        properties: { json: { type: "string", description: "The complete design document as a JSON string." } },
        required: ["json"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "get_element_json",
      description: "Return one element of the design currently open in the editor, by element id (searches every page, then shared elements), as a JSON object — the starting point for editing just that element.",
      parameters: {
        type: "object",
        properties: { elementId: { type: "string", description: "The element's id." } },
        required: ["elementId"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "write_element_json",
      description: "Replace ONE element of the open design, by id, with the complete updated element passed as a JSON object (not a string). Everything else in the design is untouched. The whole design is re-validated against the device schema and applied to the editor; returns validation errors if any. Use get_element_json first to see the element as it is. For script code, prefer write_script_code.",
      parameters: {
        type: "object",
        properties: {
          elementId: { type: "string", description: "The id of the element to replace." },
          element: { type: "object", description: "The complete replacement element object (same shape as in the Design JSON; its id is forced to elementId)." },
        },
        required: ["elementId", "element"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "write_script_code",
      description: "Set the script code (props.code) of a script element in the open design. Pass the code as ONE plain string, exactly as it should run — newlines, quotes and apostrophes are fine; do not pre-escape anything. This is the only reliable way to write scripts: embedding code inside write_design_json / write_element_json JSON double-escapes it. The design is re-validated and applied to the editor; check the result with view_screen.",
      parameters: {
        type: "object",
        properties: {
          elementId: { type: "string", description: "The id of the script element." },
          code: { type: "string", description: "The complete script source, exactly as written." },
        },
        required: ["elementId", "code"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "save_design",
      description: "Save the current design to the server and push it to the screens using it (the Studio's normal save).",
      parameters: { type: "object", properties: {}, additionalProperties: false },
    },
  },
  {
    type: "function",
    function: {
      name: "view_screen",
      description: "Render a design page exactly as the server renders it for the device — with live Home Assistant and external values — and return the render as an image you can look at. Use it after editing to check layout, overlaps, and readability before saving. Defaults to the page currently active in the editor of the open design (unsaved changes included); pass designId to view another design's saved home page (or pageId within it) instead. Requires a vision-capable model.",
      parameters: {
        type: "object",
        properties: {
          designId: { type: "string", description: "Design id to view; defaults to the open design." },
          pageId: { type: "string", description: "Page id within the design; defaults to the active page (open design) or home page." },
        },
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "list_designs",
      description: "List all designs on this server (id, name, revision, page count).",
      parameters: { type: "object", properties: {}, additionalProperties: false },
    },
  },
  {
    type: "function",
    function: {
      name: "list_screens",
      description: "List the known ESP32 screens (id, name, connected, which design each uses).",
      parameters: { type: "object", properties: {}, additionalProperties: false },
    },
  },
  {
    type: "function",
    function: {
      name: "list_home_assistant_sensors",
      description: "List the user's Home Assistant entities (sensors and other devices) with their current live states, names and units. Only available while Home Assistant is connected. Pass pattern to grep the list instead of returning everything.",
      parameters: {
        type: "object",
        properties: {
          pattern: { type: "string", description: "Optional regex (grep-style, case-insensitive) matched against each entity's id and name — e.g. '^sensor\\.' or 'temperature|humidity'. Omit to list everything." },
        },
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "open_design",
      description: "Open a different design in the editor by id (switches what get_current_design / write_design_json operate on).",
      parameters: {
        type: "object",
        properties: { designId: { type: "string" } },
        required: ["designId"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "assign_design",
      description: "Assign the current design to a screen by screen id.",
      parameters: {
        type: "object",
        properties: { screenId: { type: "string" } },
        required: ["screenId"],
        additionalProperties: false,
      },
    },
  },
];

// ---------- tool implementations ----------

function designSummary(doc) {
  const pages = doc.pages || [];
  return {
    name: doc.name,
    orientation: doc.orientation,
    pages: pages.map((page) => ({ id: page.id, name: page.name, elements: (page.elements || []).length })),
    sharedElements: (doc.sharedElements || []).length,
    totalElements: pages.reduce((sum, page) => sum + (page.elements || []).length, 0) + (doc.sharedElements || []).length,
  };
}

async function validateDoc(doc) {
  const response = await fetch(`/api/layout/validate?design=${encodeURIComponent(S.currentDesignId)}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(doc),
  });
  const result = await response.json().catch(() => ({}));
  return { response, result };
}

async function toolGetCurrentDesign() {
  if (!S.layout) throw new Error("No design is open in the editor yet.");
  return { designId: S.currentDesignId, json: JSON.stringify(S.layout) };
}

/** Validate a complete document and, when valid, make it the editor's
 *  layout (undo history, dirty flag, preview — the write_design_json path).
 *  Pass { selectId } to set the editor selection before rendering. */
async function applyDesignDoc(doc, opts = {}) {
  const { response, result } = await validateDoc(doc);
  if (!response.ok) {
    return { applied: false, errors: result.details || result.errors || [{ path: "$", message: result.error || "Validation failed" }] };
  }
  pushHistory();
  S.layout = result.layout;
  const designId = S.editingDesignId ?? S.currentDesignId;
  S.designLayouts.set(designId, S.layout);
  if (!pageOf(S.layout, activePageIdFor(designId))) {
    S.activePages.set(designId, homePageIdOf(S.layout));
  }
  if ("selectId" in opts) S.selectedId = opts.selectId;
  S.dirty = true;
  syncCodeEditor(true);
  renderAll();
  schedulePreview(0);
  return { applied: true };
}

async function toolSetDesignJson(args) {
  const doc = JSON.parse(args.json);
  const applied = await applyDesignDoc(doc, { selectId: null });
  if (!applied.applied) return applied;
  return { applied: true, summary: designSummary(S.layout), note: "Applied to the editor (not saved yet — call save_design to persist)." };
}

/** Find an element by id: every page in order, then shared elements. */
function findElement(layout, elementId) {
  for (const page of layout.pages || []) {
    const list = page.elements || [];
    const index = list.findIndex((el) => el.id === elementId);
    if (index >= 0) return { list, index, pageId: page.id, element: list[index] };
  }
  const list = layout.sharedElements || [];
  const index = list.findIndex((el) => el.id === elementId);
  if (index >= 0) return { list, index, pageId: null, element: list[index] };
  return null;
}

function activePageOpts(elementId, pageId) {
  const designId = S.editingDesignId ?? S.currentDesignId;
  return pageId && pageId === activePageIdFor(designId) ? { selectId: elementId } : {};
}

async function toolGetElementJson(args) {
  if (!S.layout) throw new Error("No design is open in the editor yet.");
  const found = findElement(S.layout, args.elementId);
  if (!found) return { error: `No element with id "${args.elementId}" in the open design.` };
  return { elementId: args.elementId, page: found.pageId || "shared", element: found.element };
}

async function toolSetElementJson(args) {
  if (!S.layout) throw new Error("No design is open in the editor yet.");
  if (!args.element || typeof args.element !== "object" || Array.isArray(args.element)) {
    return { applied: false, error: "Pass the complete replacement element as a JSON object in 'element'." };
  }
  const doc = structuredClone(S.layout);
  const found = findElement(doc, args.elementId);
  if (!found) {
    return { applied: false, error: `No element with id "${args.elementId}" in the open design. Use write_design_json to add new elements.` };
  }
  found.list[found.index] = { ...args.element, id: args.elementId };
  const applied = await applyDesignDoc(doc, activePageOpts(args.elementId, found.pageId));
  if (!applied.applied) return applied;
  return {
    applied: true,
    elementId: args.elementId,
    type: args.element.type,
    page: found.pageId || "shared",
    note: "Element updated in the editor (not saved yet — call save_design to persist).",
  };
}

async function toolSetScriptCode(args) {
  if (!S.layout) throw new Error("No design is open in the editor yet.");
  if (typeof args.code !== "string") {
    return { applied: false, error: "Pass the script as one plain string in 'code'." };
  }
  const doc = structuredClone(S.layout);
  const found = findElement(doc, args.elementId);
  if (!found) {
    return { applied: false, error: `No element with id "${args.elementId}" in the open design.` };
  }
  if (found.element.type !== "script") {
    return { applied: false, error: `Element "${args.elementId}" is a ${found.element.type} element, not a script. Use write_element_json for it.` };
  }
  found.element.props = { ...(found.element.props || {}), code: args.code };
  const applied = await applyDesignDoc(doc, activePageOpts(args.elementId, found.pageId));
  if (!applied.applied) return applied;
  return {
    applied: true,
    elementId: args.elementId,
    chars: args.code.length,
    lines: args.code.split("\n").length,
    note: "Script updated in the editor (not saved yet). Call view_screen to check what it renders, then save_design to persist.",
  };
}

async function toolSaveDesign() {
  await saveLayout();
  return { saved: true, dirty: S.dirty };
}

function blobToDataUrl(blob) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(reader.error || new Error("Could not read the rendered image."));
    reader.readAsDataURL(blob);
  });
}

/**
 * Render a design page server-side (live HA/external values included) and
 * hand the PNG to the model as an image. The loop picks the `image` field
 * out of the result and attaches it as a vision input; it is stripped
 * before the tool result is stored as text.
 */
async function toolViewScreen(args) {
  const designId = args.designId || S.currentDesignId;
  let layout;
  let designName;
  if (designId === S.currentDesignId && S.layout) {
    layout = S.layout;
    designName = S.layout.name;
  } else {
    const data = await fetchJson(`/api/layout?design=${encodeURIComponent(designId)}`);
    layout = data.layout;
    designName = layout?.name || designId;
  }
  if (!layout) throw new Error(`Design ${designId} could not be loaded.`);
  const pageId = args.pageId || (designId === S.currentDesignId ? activePageIdFor(designId) : homePageIdOf(layout));
  const page = pageOf(layout, pageId);
  if (!page) throw new Error(`Page ${pageId} does not exist in design ${designId}.`);
  const response = await fetch(`/api/preview?design=${encodeURIComponent(designId)}&page=${encodeURIComponent(pageId)}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(layout),
  });
  if (!response.ok) {
    const detail = await response.json().catch(() => ({}));
    throw new Error(detail.error || `Preview render failed (HTTP ${response.status}).`);
  }
  const dataUrl = await blobToDataUrl(await response.blob());
  const landscape = layout.orientation === "landscape";
  const width = landscape ? 320 : 240;
  const height = landscape ? 240 : 320;
  return {
    viewed: { designId, designName, pageId, pageName: page.name, width, height },
    note: "The rendered image is attached next; inspect it for layout problems (overlaps, clipping, unreadable text).",
    image: dataUrl,
    imageCaption: `Live render of design "${designName}", page "${page.name}" (${width}x${height}), rendered just now with current values:`,
  };
}

async function toolListDesigns() {
  return S.studio.designs.map((d) => ({ id: d.id, name: d.name, revision: d.revision, pages: d.pageCount, orientation: d.orientation }));
}

async function toolListScreens() {
  return S.studio.screens.map((s) => ({ id: s.id, name: s.name, connected: s.connected, designId: s.designId }));
}

/** Home Assistant counts as enabled only while it is connected here. */
function haEnabled() {
  return Boolean(S.homeAssistant?.configured);
}

/** The tools offered to the model: the HA sensor tool exists only while
 *  Home Assistant is connected. */
function availableTools() {
  return TOOLS.filter((tool) => tool.function.name !== "list_home_assistant_sensors" || haEnabled());
}

async function toolListHomeAssistantSensors(args) {
  if (!haEnabled()) {
    return { enabled: false, sensors: [], note: "Home Assistant is not connected in this Studio, so no sensors are available." };
  }
  let regex = null;
  if (args.pattern) {
    try {
      regex = new RegExp(args.pattern, "i");
    } catch (error) {
      return { error: `Invalid regex "${args.pattern}": ${error.message}` };
    }
  }
  const all = (S.homeAssistant.entities || []).map((e) => ({
    entityId: e.entityId,
    name: e.name,
    state: e.state,
    ...(e.unit ? { unit: e.unit } : {}),
  }));
  const sensors = regex ? all.filter((e) => regex.test(e.entityId) || regex.test(e.name || "")) : all;
  return {
    enabled: true,
    count: sensors.length,
    ...(regex ? { pattern: args.pattern, totalCount: all.length } : {}),
    sensors,
  };
}

async function toolOpenDesign(args) {
  await loadDesign(args.designId);
  return { opened: args.designId, current: S.currentDesignId };
}

async function toolAssignDesign(args) {
  const result = await fetchJson(`/api/screens/${encodeURIComponent(args.screenId)}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ designId: S.currentDesignId }),
  });
  const screen = S.studio.screens.find((item) => item.id === args.screenId);
  if (screen) screen.designId = S.currentDesignId;
  return { assigned: true, screen: result.screen?.name || args.screenId, designId: S.currentDesignId };
}

const TOOL_IMPL = {
  get_current_design: toolGetCurrentDesign,
  write_design_json: toolSetDesignJson,
  get_element_json: toolGetElementJson,
  write_element_json: toolSetElementJson,
  write_script_code: toolSetScriptCode,
  save_design: toolSaveDesign,
  view_screen: toolViewScreen,
  list_designs: toolListDesigns,
  list_screens: toolListScreens,
  list_home_assistant_sensors: toolListHomeAssistantSensors,
  open_design: toolOpenDesign,
  assign_design: toolAssignDesign,
};

/** While any tool call runs, the designer window breathes a green border. */
function setToolPulse(on) {
  document.querySelectorAll(".canvas-area").forEach((el) => el.classList.toggle("tool-pulse", on));
}

async function runTool(name, argsJson) {
  setToolPulse(true);
  try {
    return await runToolCall(name, argsJson);
  } finally {
    setToolPulse(false);
  }
}

async function runToolCall(name, argsJson) {
  const impl = TOOL_IMPL[name];
  if (!impl) return { error: `Unknown tool ${name}` };
  let args = {};
  try {
    args = argsJson ? JSON.parse(argsJson) : {};
  } catch {
    return { error: "Tool arguments were not valid JSON." };
  }
  try {
    return await impl(args);
  } catch (error) {
    return { error: error.message || String(error) };
  }
}

// ---------- rendering ----------

function escapeHtml(text) {
  return text.replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[ch]));
}

function markdownLite(text) {
  const parts = text.split(/```(?:\w+)?\n?/);
  return parts
    .map((part, index) => {
      if (index % 2 === 1) return `<pre>${escapeHtml(part.replace(/\n$/, ""))}</pre>`;
      return escapeHtml(part)
        .replace(/`([^`\n]+)`/g, "<code>$1</code>")
        .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
        .replace(/\n/g, "<br>");
    })
    .join("");
}

function addMessage(role, text = "") {
  // Never render an empty assistant bubble.
  if (role === "assistant" && !text.trim()) return null;
  closeToolGroup();
  const box = $("agent-messages");
  const el = document.createElement("div");
  el.className = `msg ${role}`;
  el.innerHTML = role === "assistant" ? markdownLite(text) : escapeHtml(text).replace(/\n/g, "<br>");
  box.append(el);
  box.scrollTop = box.scrollHeight;
  return el;
}

/** Expand string values that are themselves JSON (e.g. a whole design
 *  document carried in a `json` field) so the dev view shows one clean
 *  document instead of an escaped string blob. */
function expandJsonStrings(value, expansions = 0) {
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (expansions < 4 && ((trimmed.startsWith("{") && trimmed.endsWith("}")) || (trimmed.startsWith("[") && trimmed.endsWith("]")))) {
      try {
        return expandJsonStrings(JSON.parse(trimmed), expansions + 1);
      } catch {
        return value;
      }
    }
    return value;
  }
  if (Array.isArray(value)) return value.map((item) => expandJsonStrings(item, expansions));
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, expandJsonStrings(item, expansions)]));
  }
  return value;
}

function jsonBlockText(value) {
  let text;
  try {
    text = JSON.stringify(expandJsonStrings(value), null, 2) || "";
  } catch {
    text = String(value ?? "");
  }
  return text.length > 40000 ? `${text.slice(0, 40000)}\n… (truncated)` : text;
}

/** A dev-mode JSON block with a token estimate (chars / 4) when long. */
function makeJsonBlock() {
  const wrap = document.createElement("div");
  wrap.className = "tool-json-wrap";
  const meta = document.createElement("div");
  meta.className = "tool-json-meta";
  const pre = document.createElement("pre");
  pre.className = "tool-json";
  wrap.append(meta, pre);
  return {
    wrap,
    set(value) {
      const text = jsonBlockText(value);
      pre.textContent = text;
      meta.textContent = text.length > 1000
        ? `≈ ${Math.round(text.length / 4).toLocaleString("en-US")} tokens · ${text.length.toLocaleString("en-US")} chars`
        : "";
    },
  };
}

// Consecutive tool calls share one group: their chips sit side by side in
// a wrapping row, with the dev-mode JSON bodies stacked underneath. Any
// other transcript content closes the group.
let openToolGroup = null;

function closeToolGroup() {
  openToolGroup = null;
}

function currentToolGroup(box) {
  if (openToolGroup && openToolGroup.group.isConnected && box.lastElementChild === openToolGroup.group) {
    return openToolGroup;
  }
  const group = document.createElement("div");
  group.className = "tool-group";
  const chips = document.createElement("div");
  chips.className = "tool-chips";
  const bodies = document.createElement("div");
  bodies.className = "tool-bodies";
  group.append(chips, bodies);
  box.append(group);
  openToolGroup = { group, chips, bodies };
  return openToolGroup;
}

// One tool call in the transcript: a chip in the current group's chip row;
// in dev mode the arguments and result JSON show in the group's bodies.
function addToolEntry(name, argsJson) {
  const box = $("agent-messages");
  const group = currentToolGroup(box);
  const chip = document.createElement("div");
  chip.className = "tool-chip";
  chip.textContent = `… ${name}`;
  group.chips.append(chip);
  const body = document.createElement("div");
  body.className = "tool-body";
  const args = makeJsonBlock();
  let parsedArgs = argsJson || "{}";
  try {
    parsedArgs = JSON.parse(argsJson || "{}");
  } catch { /* keep raw */ }
  args.set(parsedArgs);
  const result = makeJsonBlock();
  body.append(args.wrap, result.wrap);
  group.bodies.append(body);
  box.scrollTop = box.scrollHeight;
  return {
    finish(resultObj, ok, imageUrl) {
      chip.textContent = `${ok ? "✓" : "✕"} ${name}`;
      chip.classList.add(ok ? "tool-ok" : "tool-fail");
      if (!ok) body.classList.add("tool-fail");
      result.set(resultObj);
      if (imageUrl) {
        const img = document.createElement("img");
        img.className = "tool-thumb";
        img.src = imageUrl;
        img.alt = "Rendered screen view";
        body.append(img);
        box.scrollTop = box.scrollHeight;
      }
    },
  };
}

// An assistant turn: an optional thinking block (dev mode only) above the
// streamed bubble. The bubble stays out of the DOM until the turn actually
// has text — tool-only and thinking-only rounds must not leave an empty
// assistant pill behind. setContent("") removes it again.
function addAssistantTurn() {
  closeToolGroup();
  const box = $("agent-messages");
  const row = document.createElement("div");
  row.className = "msg-row";
  const thinking = document.createElement("details");
  thinking.className = "thinking";
  const summary = document.createElement("summary");
  summary.textContent = "Thinking";
  const thinkingBody = document.createElement("div");
  thinkingBody.className = "thinking-body";
  thinking.append(summary, thinkingBody);
  const bubble = document.createElement("div");
  bubble.className = "msg assistant";
  box.append(row);
  box.scrollTop = box.scrollHeight;
  return {
    bubble,
    thinkingBody,
    row,
    setThinking(text) {
      if (!text || !text.trim()) return;
      if (!thinking.isConnected) row.insertBefore(thinking, bubble.isConnected ? bubble : null);
      thinkingBody.textContent = text;
    },
    setContent(text) {
      // Whitespace-only text (or text that renders to nothing but <br>)
      // must never surface as an empty bubble.
      if (text && text.trim()) {
        if (!bubble.isConnected) row.append(bubble);
        bubble.innerHTML = markdownLite(text);
      } else {
        bubble.remove();
      }
    },
    /** Drop the whole row if the turn produced neither text nor thinking. */
    finish() {
      if (!bubble.isConnected && !thinking.isConnected) row.remove();
    },
  };
}

// ---------- Dev-mode context graph ----------
// A stacked bar estimating what the next request's context is made of:
// system prompt, tool definitions, model thinking, tool calls/results,
// images, and plain text. Text is estimated at chars/4; images at a flat
// ~258 tokens each (their real vision cost, not their data-URL length).
const CONTEXT_SEGMENTS = [
  { key: "system", label: "System", color: "#6b9cff" },
  { key: "tools", label: "Tools", color: "#b388ff" },
  { key: "thinking", label: "Thinking", color: "#4dd0e1" },
  { key: "toolcalls", label: "Tool calls", color: "#f2c94c" },
  { key: "images", label: "Images", color: "#f48fb1" },
  { key: "text", label: "Text", color: "#8a93a5" },
];

function renderContextGraph() {
  const bar = $("agent-context-bar");
  const legend = $("agent-context-legend");
  if (!bar || !legend) return;
  const est = (chars) => Math.ceil(chars / 4);
  const seg = {
    system: est(SYSTEM_PROMPT.length),
    tools: est(JSON.stringify(availableTools()).length),
    thinking: 0,
    toolcalls: 0,
    images: 0,
    text: 0,
  };
  for (const message of conversation) {
    if (message._reasoning) seg.thinking += est(String(message._reasoning).length);
    if (message.role === "tool") {
      seg.toolcalls += est(String(message.content ?? "").length);
      continue;
    }
    if (Array.isArray(message.tool_calls)) {
      for (const call of message.tool_calls) {
        seg.toolcalls += est((call.function?.name || "").length + (call.function?.arguments || "").length);
      }
    }
    if (typeof message.content === "string") {
      seg.text += est(message.content.length);
    } else if (Array.isArray(message.content)) {
      for (const part of message.content) {
        if (part.type === "text") seg.text += est(part.text.length);
        else if (part.type === "image_url") seg.images += 258;
      }
    }
  }
  const total = Object.values(seg).reduce((sum, n) => sum + n, 0);
  bar.replaceChildren();
  legend.replaceChildren();
  for (const { key, label, color } of CONTEXT_SEGMENTS) {
    const tokens = seg[key];
    if (total > 0 && tokens > 0) {
      const slice = document.createElement("span");
      slice.style.width = `${(tokens / total) * 100}%`;
      slice.style.background = color;
      slice.title = `${label}: ≈${tokens.toLocaleString("en-US")} tokens`;
      bar.append(slice);
    }
    const item = document.createElement("span");
    item.className = "agent-context-item";
    const dot = document.createElement("span");
    dot.className = "agent-context-dot";
    dot.style.background = color;
    item.append(dot, `${label} ≈${tokens >= 1000 ? `${(tokens / 1000).toFixed(1)}k` : tokens}`);
    legend.append(item);
  }
}

function renderUsage() {
  renderContextGraph();
  const el = $("agent-usage");
  if (!el) return;
  if (!usageTotals.in && !usageTotals.out) {
    el.textContent = "";
    return;
  }
  const fmt = (n) => n.toLocaleString("en-US");
  el.textContent = `${fmt(usageTotals.in)} in · ${fmt(usageTotals.out)} out` +
    (usageTotals.cost ? ` · $${usageTotals.cost.toFixed(4)}` : "");
  el.title = "Session token usage";
}

// ---------- OpenRouter ----------

function apiKey() {
  return localStorage.getItem(KEY_STORAGE) || "";
}

// ---------- model picker ----------
// Only vision-capable models are offered: view_screen hands the model a
// rendered image, so a text-only model cannot do the job. Favorites are
// pinned to the top of the picker and stored in this browser only.

const FAV_STORAGE = "yello-agent-fav-models";
let visionModels = [];
let modelsLoadOk = false;
let setupOpen = false;

function loadFavorites() {
  try {
    return new Set(JSON.parse(localStorage.getItem(FAV_STORAGE) || "[]"));
  } catch {
    return new Set();
  }
}

function saveFavorites(favs) {
  localStorage.setItem(FAV_STORAGE, JSON.stringify([...favs]));
}

/** Vision = the model accepts image input. OpenRouter describes this in
 *  architecture.input_modalities / modality; endpoints that describe
 *  nothing are trusted (the mock and small compatible APIs). */
function isVisionModel(model) {
  const inputs = model.architecture?.input_modalities;
  if (Array.isArray(inputs)) return inputs.includes("image");
  const modality = model.architecture?.modality;
  if (typeof modality === "string") return modality.split("->")[0].includes("image");
  return true;
}

function modelPriceText(model) {
  const inPrice = Number(model.pricing?.prompt || 0) * 1e6;
  const outPrice = Number(model.pricing?.completion || 0) * 1e6;
  if (inPrice === 0 && outPrice === 0) return "free";
  return `$${inPrice.toFixed(2)}/$${outPrice.toFixed(2)} per 1M`;
}

function modelMetaText(model) {
  const bits = [model.id, modelPriceText(model)];
  const ctx = Number(model.context_length || model.top_provider?.context_length || 0);
  if (ctx >= 1000) bits.push(`${Math.round(ctx / 1000)}K ctx`);
  return bits.join(" · ");
}

function selectedModel() {
  return visionModels.find((m) => m.id === $("agent-model").value) || null;
}

function syncModelButton() {
  const model = selectedModel();
  $("agent-model-label").textContent = model ? (model.name || model.id) : "Select a model";
  $("agent-model-button").title = model ? model.id : "Choose a model";
}

function selectModel(id) {
  const select = $("agent-model");
  if (![...select.options].some((o) => o.value === id)) return;
  select.value = id;
  localStorage.setItem(MODEL_STORAGE, id);
  syncModelButton();
  renderModelList();
}

function renderModelList() {
  const list = $("agent-model-list");
  if (!list) return;
  const query = ($("agent-model-search").value || "").trim().toLowerCase();
  const favs = loadFavorites();
  const matches = visionModels.filter((m) => !query || `${m.name || ""} ${m.id}`.toLowerCase().includes(query));
  const current = $("agent-model").value;
  list.replaceChildren();
  const addGroup = (title, models) => {
    if (!models.length) return;
    const head = document.createElement("div");
    head.className = "agent-model-group";
    head.textContent = title;
    list.append(head);
    for (const model of models) {
      const row = document.createElement("div");
      row.className = `agent-model-row${model.id === current ? " selected" : ""}`;
      const pick = document.createElement("button");
      pick.type = "button";
      pick.className = "agent-model-pick";
      pick.setAttribute("role", "option");
      pick.setAttribute("aria-selected", String(model.id === current));
      const name = document.createElement("span");
      name.className = "agent-model-name";
      name.textContent = model.name || model.id;
      const meta = document.createElement("span");
      meta.className = "agent-model-meta";
      meta.textContent = modelMetaText(model);
      pick.append(name, meta);
      pick.onclick = () => {
        selectModel(model.id);
        setModelDropdown(false);
      };
      const fav = document.createElement("button");
      fav.type = "button";
      fav.className = `agent-model-fav${favs.has(model.id) ? " is-fav" : ""}`;
      fav.title = favs.has(model.id) ? "Remove from favorites" : "Add to favorites";
      fav.setAttribute("aria-label", fav.title);
      fav.setAttribute("aria-pressed", String(favs.has(model.id)));
      fav.innerHTML = '<svg class="ic" aria-hidden="true"><use href="#i-star"></use></svg>';
      fav.onclick = () => {
        const next = loadFavorites();
        if (next.has(model.id)) next.delete(model.id);
        else next.add(model.id);
        saveFavorites(next);
        renderModelList();
      };
      row.append(pick, fav);
      list.append(row);
    }
  };
  addGroup("Favorites", matches.filter((m) => favs.has(m.id)));
  addGroup("All models", matches.filter((m) => !favs.has(m.id)));
  if (!matches.length) {
    const empty = document.createElement("div");
    empty.className = "agent-model-empty";
    empty.textContent = visionModels.length ? `No vision models match “${query}”.` : "No models loaded yet.";
    list.append(empty);
  }
  $("agent-model-count").textContent = `${visionModels.length} vision models`;
}

function setModelDropdown(open) {
  const dropdown = $("agent-model-dropdown");
  dropdown.classList.toggle("hidden", !open);
  $("agent-model-button").setAttribute("aria-expanded", String(open));
  if (open) {
    renderModelList();
    $("agent-model-search").focus();
  }
}

// ---------- provider bar ----------
// Once a key is saved and the model list loads with it, the provider
// section collapses to a one-line "Connected" bar; the gear reopens it.

function updateProviderUi() {
  const hasKey = Boolean(apiKey());
  const connected = hasKey && modelsLoadOk;
  // The connection status light lives in the agent window's own top bar.
  const statusText = connected ? "Connected" : hasKey ? "Key saved" : "Not connected";
  $("agent-status-dot").classList.toggle("connected", connected);
  const topText = $("agent-status-text");
  const baseHost = API_BASE.replace(/^https?:\/\//, "").split("/")[0];
  const providerName = baseHost.includes("openrouter") ? "OpenRouter" : baseHost;
  topText.textContent = `${providerName} · ${statusText}`;
  topText.classList.toggle("connected", connected);
  $("agent-key-state").textContent = hasKey ? "Key saved in this browser" : "No key saved";
  const open = setupOpen || !connected;
  $("agent-setup-body").style.display = open ? "" : "none";
  $("agent-settings-toggle").setAttribute("aria-expanded", String(open));
  $("agent-provider-name").textContent = providerName;
}

async function loadModels() {
  const select = $("agent-model");
  const status = $("agent-model-state");
  status.textContent = "Loading models…";
  try {
    const headers = {};
    if (apiKey()) headers.Authorization = `Bearer ${apiKey()}`;
    const response = await fetch(`${API_BASE}/models`, { headers });
    if (!response.ok) throw new Error(`OpenRouter returned ${response.status}`);
    const { data } = await response.json();
    visionModels = (data || [])
      .filter(isVisionModel)
      .sort((a, b) => (a.name || a.id).localeCompare(b.name || b.id));
    select.replaceChildren();
    for (const model of visionModels) {
      const option = document.createElement("option");
      option.value = model.id;
      option.textContent = `${model.name || model.id} · ${modelPriceText(model)}`;
      select.append(option);
    }
    const stored = localStorage.getItem(MODEL_STORAGE);
    const favs = loadFavorites();
    const firstFav = visionModels.find((m) => favs.has(m.id));
    if (stored && visionModels.some((m) => m.id === stored)) select.value = stored;
    else if (firstFav) select.value = firstFav.id;
    else {
      const sonnet = visionModels.find((m) => m.id.includes("claude-sonnet"));
      if (sonnet) select.value = sonnet.id;
    }
    if (select.value) localStorage.setItem(MODEL_STORAGE, select.value);
    modelsLoadOk = true;
    status.textContent = `${visionModels.length} vision models`;
  } catch (error) {
    modelsLoadOk = false;
    status.textContent = `Could not load models: ${error.message}`;
  }
  syncModelButton();
  renderModelList();
  updateProviderUi();
}

async function streamChat(messages) {
  const response = await fetch(`${API_BASE}/chat/completions`, {
    method: "POST",
    signal: aborter.signal,
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${apiKey()}`,
      "X-Title": "Yello Studio",
    },
    body: JSON.stringify({
      model: $("agent-model").value,
      messages,
      tools: availableTools(),
      stream: true,
      usage: { include: true },
    }),
  });
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.error?.message || `OpenRouter returned ${response.status}`);
  }
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let content = "";
  let reasoning = "";
  let usage = null;
  const toolCalls = new Map();
  let turn = null;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop();
    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed.startsWith("data:")) continue;
      const data = trimmed.slice(5).trim();
      if (data === "[DONE]") continue;
      let chunk;
      try {
        chunk = JSON.parse(data);
      } catch {
        continue;
      }
      if (chunk.usage) usage = chunk.usage;
      const delta = chunk.choices?.[0]?.delta || {};
      const reasoningDelta = delta.reasoning || delta.reasoning_content || "";
      if (reasoningDelta) {
        reasoning += reasoningDelta;
        if (!turn) turn = addAssistantTurn();
        turn.setThinking(reasoning);
        $("agent-messages").scrollTop = $("agent-messages").scrollHeight;
      }
      if (delta.content) {
        content += delta.content;
        if (!turn) turn = addAssistantTurn();
        turn.setContent(content);
        $("agent-messages").scrollTop = $("agent-messages").scrollHeight;
      }
      for (const call of delta.tool_calls || []) {
        const entry = toolCalls.get(call.index) || { id: "", name: "", arguments: "" };
        if (call.id) entry.id = call.id;
        if (call.function?.name) entry.name = call.function.name;
        if (call.function?.arguments) entry.arguments += call.function.arguments;
        toolCalls.set(call.index, entry);
      }
    }
  }
  if (turn) turn.finish();
  return { content, reasoning, toolCalls: [...toolCalls.values()], usage };
}

/** Only the newest view_screen image is worth its tokens on later rounds:
 *  once a newer render exists, older ones collapse to their caption. */
function collapseOldImages(messages) {
  let lastImageIndex = -1;
  messages.forEach((message, index) => {
    if (Array.isArray(message.content) && message.content.some((part) => part.type === "image_url")) {
      lastImageIndex = index;
    }
  });
  if (lastImageIndex < 0) return messages;
  return messages.map((message, index) => {
    if (index >= lastImageIndex || !Array.isArray(message.content)) return message;
    const text = message.content.find((part) => part.type === "text")?.text || "Rendered view:";
    return { ...message, content: `${text} [image omitted — a newer render follows]` };
  });
}

/** Strip local-only fields (e.g. _reasoning) before messages go to the API. */
function sanitizeMessage(message) {
  const clean = { role: message.role, content: message.content };
  if (message.tool_calls) clean.tool_calls = message.tool_calls;
  if (message.tool_call_id) clean.tool_call_id = message.tool_call_id;
  return clean;
}

function compactedHistory() {
  return collapseOldImages(conversation).map(sanitizeMessage);
}

// ---------- conversation persistence (IndexedDB) ----------

let currentChatId = null;
let currentChatCreatedAt = 0;

function chatTitle() {
  const first = conversation.find((m) => m.role === "user" && typeof m.content === "string" && !m._context);
  const text = first?.content.trim().replace(/\s+/g, " ") || "";
  return text ? text.slice(0, 70) : "Chat";
}

/** Save the current conversation (fire-and-forget; storage failures are
 *  non-fatal — the chat keeps working in memory). */
function persistChat() {
  if (!conversation.some((m) => m.role === "user")) return;
  const now = Date.now();
  if (!currentChatId) {
    currentChatId = `chat-${now}-${Math.random().toString(36).slice(2, 8)}`;
    currentChatCreatedAt = now;
  }
  localStorage.setItem(ACTIVE_CHAT_KEY, currentChatId);
  saveChat({
    id: currentChatId,
    title: chatTitle(),
    createdAt: currentChatCreatedAt,
    updatedAt: now,
    model: localStorage.getItem(MODEL_STORAGE) || "",
    designId: S.currentDesignId || "",
    designName: S.layout?.name || "",
    usage: { ...usageTotals },
    messages: collapseOldImages(conversation),
  }).catch(() => {});
}

function startNewChat() {
  // Clear really starts a new conversation: stop any in-flight reply,
  // invalidate its pending persist, and remember that the fresh chat is
  // the current one so a reload does not resurrect the previous chat.
  chatEpoch += 1;
  if (aborter) aborter.abort();
  localStorage.setItem(ACTIVE_CHAT_KEY, "new");
  conversation.length = 0;
  currentChatId = null;
  currentChatCreatedAt = 0;
  usageTotals.in = 0;
  usageTotals.out = 0;
  usageTotals.cost = 0;
  renderUsage();
  $("agent-messages").replaceChildren();
  greet();
}

/** Rebuild the chat DOM from a stored message list. */
function renderConversationDom() {
  closeToolGroup();
  const box = $("agent-messages");
  box.replaceChildren();
  for (let i = 0; i < conversation.length; i += 1) {
    const message = conversation[i];
    if (message._context) continue;
    if (message.role === "user" && typeof message.content === "string") {
      addMessage("user", message.content);
    } else if (message.role === "user" && Array.isArray(message.content)) {
      closeToolGroup();
      const caption = message.content.find((p) => p.type === "text")?.text || "Rendered view:";
      const image = message.content.find((p) => p.type === "image_url")?.image_url?.url;
      const row = document.createElement("div");
      row.className = "msg-row";
      const note = document.createElement("div");
      note.className = "msg view-note";
      note.textContent = caption;
      row.append(note);
      if (image) {
        const img = document.createElement("img");
        img.className = "tool-thumb";
        img.src = image;
        img.alt = "Rendered screen view";
        row.append(img);
      }
      box.append(row);
    } else if (message.role === "assistant") {
      if (message.content || message._reasoning) {
        const turn = addAssistantTurn();
        if (message._reasoning) turn.setThinking(message._reasoning);
        if (message.content) turn.setContent(message.content);
        turn.finish();
      }
      for (const call of message.tool_calls || []) {
        const toolMessage = conversation
          .slice(i + 1)
          .find((m) => m.role === "tool" && m.tool_call_id === call.id);
        const entry = addToolEntry(call.function.name, call.function.arguments);
        let parsed = {};
        try {
          parsed = JSON.parse(toolMessage?.content || "{}");
        } catch {
          parsed = { result: toolMessage?.content || "" };
        }
        const failed = Boolean(parsed && (parsed.error || parsed.applied === false || parsed.valid === false));
        entry.finish(parsed, !failed);
      }
    }
  }
  box.scrollTop = box.scrollHeight;
}

function loadChat(record) {
  localStorage.setItem(ACTIVE_CHAT_KEY, record.id);
  conversation.length = 0;
  for (const message of record.messages || []) conversation.push(message);
  currentChatId = record.id;
  currentChatCreatedAt = record.createdAt || Date.now();
  usageTotals.in = record.usage?.in || 0;
  usageTotals.out = record.usage?.out || 0;
  usageTotals.cost = record.usage?.cost || 0;
  renderUsage();
  renderConversationDom();
  setHistoryOpen(false);
}

async function restoreLastChat() {
  const active = localStorage.getItem(ACTIVE_CHAT_KEY);
  if (active === "new") {
    greet();
    return;
  }
  try {
    if (active) {
      const record = await getChat(active);
      if (record && (record.messages || []).length) {
        loadChat(record);
        return;
      }
    }
    const chats = await listChats();
    if (chats.length) {
      const record = await getChat(chats[0].id);
      if (record && (record.messages || []).length) {
        loadChat(record);
        return;
      }
    }
  } catch { /* storage unavailable — fall through to a fresh chat */ }
  greet();
}

// ---------- history view ----------

function setHistoryOpen(open) {
  $("agent-history-view").classList.toggle("hidden", !open);
  $("agent-system-prompt")?.classList.toggle("hidden", open);
  $("agent-tools-prompt")?.classList.toggle("hidden", open);
  $("agent-messages").classList.toggle("hidden", open);
  document.querySelector("#agent-chat-card .agent-composer")?.classList.toggle("hidden", open);
  document.querySelector("#agent-chat-card .agent-statusbar")?.classList.toggle("hidden", open);
  if (open) renderHistoryList();
}

async function renderHistoryList() {
  const list = $("agent-history-list");
  list.replaceChildren();
  const chats = await listChats().catch(() => []);
  if (!chats.length) {
    const empty = document.createElement("p");
    empty.className = "subtle";
    empty.textContent = "No saved conversations yet.";
    list.append(empty);
    return;
  }
  for (const chat of chats) {
    const row = document.createElement("div");
    row.className = "history-row";
    const main = document.createElement("div");
    main.className = "history-main";
    const title = document.createElement("div");
    title.className = "history-title";
    title.textContent = chat.title + (chat.id === currentChatId ? " (current)" : "");
    const meta = document.createElement("div");
    meta.className = "history-meta";
    const when = new Date(chat.updatedAt).toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
    const bits = [when, chat.messageCount + " messages"];
    if (chat.designName) bits.push(`design “${chat.designName}”`);
    if (chat.model) bits.push(chat.model.split("/").pop());
    if (chat.usage?.in || chat.usage?.out) bits.push(`${fmtTokens(chat.usage.in)} in · ${fmtTokens(chat.usage.out)} out`);
    meta.textContent = bits.join(" · ");
    main.append(title, meta);
    const openButton = document.createElement("button");
    openButton.type = "button";
    openButton.className = "assign-button";
    openButton.textContent = "Open";
    openButton.onclick = async () => {
      const record = await getChat(chat.id).catch(() => null);
      if (record) loadChat(record);
    };
    const deleteButton = document.createElement("button");
    deleteButton.type = "button";
    deleteButton.className = "danger-button";
    deleteButton.textContent = "Delete";
    deleteButton.onclick = async () => {
      const confirmed = await appConfirm({
        title: "Delete conversation?",
        message: `“${chat.title}” will be removed from this browser's history.`,
        confirmLabel: "Delete",
      });
      if (!confirmed) return;
      await deleteChat(chat.id).catch(() => {});
      if (chat.id === currentChatId) startNewChat();
      renderHistoryList();
    };
    row.append(main, openButton, deleteButton);
    list.append(row);
  }
}

function fmtTokens(n) {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n || 0);
}

/** Seed a fresh conversation with the current design: its full Design JSON
 *  plus a render of the active page (a 320x240 screenshot costs roughly
 *  100-260 vision tokens depending on the provider — cheap next to the
 *  fixed prompt overhead). Both are marked _context so the transcript never
 *  renders them as user bubbles; sanitizeMessage strips the flag before
 *  anything goes to the API, and the usual image collapsing applies. */
async function seedStartContext() {
  if (!S.layout) return;
  conversation.push({
    role: "user",
    _context: true,
    content: `Starting context: my design "${S.layout.name || S.currentDesignId}" (id ${S.currentDesignId}) is open in the editor${S.dirty ? " with unsaved changes" : ""}. Here is its full Design JSON right now — work from this copy instead of calling get_current_design first:\n\n${JSON.stringify(S.layout)}`,
  });
  try {
    const view = await toolViewScreen({});
    if (view?.image) {
      conversation.push({
        role: "user",
        _context: true,
        content: [
          { type: "text", text: `Starting context: ${view.imageCaption || "a render of the design's active page, rendered just now:"}` },
          { type: "image_url", image_url: { url: view.image } },
        ],
      });
    }
  } catch {
    // The render is a bonus; the JSON seed still stands.
  }
}

async function sendAgentMessage() {
  const input = $("agent-input");
  const text = input.value.trim();
  if (!text || sending) return;
  if (!apiKey()) {
    addMessage("assistant", "Add your OpenRouter API key first — tap the gear on the OpenRouter bar above. It stays in this browser and is only sent to OpenRouter.");
    return;
  }
  if (!$("agent-model").value) {
    addMessage("assistant", "Pick a model first (the list loads from OpenRouter).");
    return;
  }
  localStorage.setItem(MODEL_STORAGE, $("agent-model").value);
  renderToolsList();
  input.value = "";
  input.style.height = "auto";
  const epoch = chatEpoch;
  sending = true;
  $("agent-send").disabled = true;
  $("agent-stop").classList.remove("hidden");
  aborter = new AbortController();
  if (conversation.length === 0) await seedStartContext();
  addMessage("user", text);
  conversation.push({ role: "user", content: text });
  try {
    for (let round = 0; round < MAX_TOOL_ROUNDS; round += 1) {
      let contextText = `Current context: design "${S.layout?.name || S.currentDesignId}" (id ${S.currentDesignId}) is open in the editor${S.dirty ? " with unsaved changes" : ""}. ${S.studio.screens.length} screen(s) known, ${S.studio.screens.filter((s) => s.connected).length} connected.`;
      const entities = S.homeAssistant?.entities || [];
      if (entities.length) {
        const list = entities.slice(0, 80).map((e) => `${e.entityId} = ${e.state}${e.unit ? ` ${e.unit}` : ""}`).join("; ");
        contextText += ` Home Assistant entities: ${list}${entities.length > 80 ? " (list truncated)" : ""}.`;
      }
      const context = { role: "system", content: contextText };
      const { content, reasoning, toolCalls, usage } = await streamChat([
        { role: "system", content: SYSTEM_PROMPT },
        context,
        ...compactedHistory(),
      ]);
      if (usage) {
        usageTotals.in += usage.prompt_tokens || 0;
        usageTotals.out += usage.completion_tokens || 0;
        usageTotals.cost += usage.cost || 0;
        renderUsage();
      }
      const assistantMessage = { role: "assistant", content: content && content.trim() ? content : null };
      if (reasoning) assistantMessage._reasoning = reasoning;
      if (toolCalls.length) {
        assistantMessage.tool_calls = toolCalls.map((call) => ({
          id: call.id,
          type: "function",
          function: { name: call.name, arguments: call.arguments },
        }));
      }
      conversation.push(assistantMessage);
      if (!toolCalls.length) break;
      for (const call of toolCalls) {
        const entry = addToolEntry(call.name, call.arguments);
        const result = await runTool(call.name, call.arguments);
        const failed = Boolean(result && (result.error || result.applied === false || result.valid === false));
        // A view_screen result carries a rendered image: keep it out of the
        // text tool result and attach it as a vision input right after.
        const image = result?.image;
        const caption = result?.imageCaption;
        const textResult = image ? { ...result } : result;
        if (textResult) {
          delete textResult.image;
          delete textResult.imageCaption;
        }
        entry.finish(textResult, !failed, image);
        conversation.push({
          role: "tool",
          tool_call_id: call.id,
          content: JSON.stringify(textResult).slice(0, 60000),
        });
        if (image && !failed) {
          conversation.push({
            role: "user",
            content: [
              { type: "text", text: caption || "Rendered view:" },
              { type: "image_url", image_url: { url: image } },
            ],
          });
        }
      }
    }
    // A Clear mid-flight starts a new conversation: this send's history
    // belongs to the old one and must not be persisted over it.
    if (epoch === chatEpoch) persistChat();
  } catch (error) {
    if (error.name !== "AbortError" && epoch === chatEpoch) addMessage("assistant", `⚠ ${error.message}`);
  } finally {
    sending = false;
    $("agent-send").disabled = false;
    $("agent-stop").classList.add("hidden");
    aborter = null;
  }
}

// ---------- page setup ----------

/** Dev-mode tools dropdown: reflects the tools actually offered right now
 *  (the HA sensor tool appears only while Home Assistant is connected). */
function renderToolsList() {
  const tools = availableTools();
  $("agent-tools-summary").textContent = `Tools (${tools.length})`;
  $("agent-tools-body").textContent = tools.map((tool) => {
    const fn = tool.function;
    const props = fn.parameters?.properties || {};
    const required = new Set(fn.parameters?.required || []);
    const params = Object.entries(props)
      .map(([name, spec]) => `${name} (${spec.type || "value"}${required.has(name) ? ", required" : ""})`)
      .join(", ");
    return `${fn.name} — ${fn.description}${params ? `\n  params: ${params}` : ""}`;
  }).join("\n\n");
}

export function initAgentPage() {
  if (initialized) return;
  initialized = true;
  const keyInput = $("agent-key");
  keyInput.value = apiKey();
  $("agent-key-state").textContent = apiKey() ? "Key saved in this browser" : "No key saved";
  $("agent-key-save").onclick = () => {
    const value = keyInput.value.trim();
    if (value) localStorage.setItem(KEY_STORAGE, value);
    else localStorage.removeItem(KEY_STORAGE);
    toast(value ? "OpenRouter key saved in this browser" : "OpenRouter key cleared");
    setupOpen = false;
    updateProviderUi();
    loadModels();
  };
  $("agent-settings-toggle").onclick = () => {
    setupOpen = $("agent-setup-body").style.display === "none";
    updateProviderUi();
  };
  $("agent-models-refresh").onclick = loadModels;
  $("agent-model").onchange = () => {
    localStorage.setItem(MODEL_STORAGE, $("agent-model").value);
    syncModelButton();
  };
  $("agent-model-button").onclick = (event) => {
    event.stopPropagation();
    setModelDropdown($("agent-model-dropdown").classList.contains("hidden"));
  };
  $("agent-model-dropdown").onclick = (event) => event.stopPropagation();
  $("agent-model-search").addEventListener("input", renderModelList);
  $("agent-model-search").addEventListener("keydown", (event) => {
    if (event.key === "Escape") setModelDropdown(false);
  });
  document.addEventListener("click", () => setModelDropdown(false));
  $("agent-send").onclick = sendAgentMessage;
  $("agent-stop").onclick = () => aborter?.abort();
  $("agent-clear").onclick = () => startNewChat();
  $("agent-history").onclick = () => setHistoryOpen(true);
  $("agent-history-close").onclick = () => setHistoryOpen(false);
  $("agent-system-prompt-body").textContent = SYSTEM_PROMPT;
  renderToolsList();
  const devButton = $("agent-dev");
  const applyDev = () => {
    const on = localStorage.getItem("yello-agent-dev") === "1";
    $("agent-chat-card").classList.toggle("agent-dev", on);
    devButton.setAttribute("aria-pressed", String(on));
    if (on) renderContextGraph();
  };
  devButton.onclick = () => {
    const on = localStorage.getItem("yello-agent-dev") === "1";
    localStorage.setItem("yello-agent-dev", on ? "0" : "1");
    applyDev();
  };
  applyDev();
  $("agent-panel-close").onclick = () => setDock(false);
  $("agent-panel-float").onclick = () => {
    panelMode = panelMode === "float" ? "dock" : "float";
    localStorage.setItem("yello-agent-panel-mode", panelMode);
    placeAgent();
  };
  // Pointer-drag helper: drag(event target) with move/up on the window.
  const startDrag = (event, onMove, onDone) => {
    event.preventDefault();
    const move = (ev) => onMove(ev);
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      if (onDone) onDone();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  // Docked: drag the panel's left edge to widen/narrow the column.
  $("agent-panel-resize").addEventListener("pointerdown", (event) => {
    if (panelMode !== "dock" || !dockOpen) return;
    const startX = event.clientX;
    const startWidth = $("agent-panel").getBoundingClientRect().width;
    startDrag(event, (ev) => {
      const width = clampNumber(startWidth + (startX - ev.clientX), 300, 760);
      localStorage.setItem("yello-agent-dock-width", String(Math.round(width)));
      applyPanelGeometry(true);
    });
  });
  // Floating: drag the header to move, the corner grip to resize.
  $("agent-panel").querySelector(".agent-panel-head").addEventListener("pointerdown", (event) => {
    if (panelMode !== "float" || !dockOpen || event.target.closest("button")) return;
    const start = floatGeometry();
    const startX = event.clientX;
    const startY = event.clientY;
    startDrag(event, (ev) => {
      const next = { ...start, x: start.x + (ev.clientX - startX), y: start.y + (ev.clientY - startY) };
      localStorage.setItem("yello-agent-panel-float", JSON.stringify(next));
      applyPanelGeometry(true);
    });
  });
  $("agent-panel-grip").addEventListener("pointerdown", (event) => {
    if (panelMode !== "float" || !dockOpen) return;
    const start = floatGeometry();
    const startX = event.clientX;
    const startY = event.clientY;
    startDrag(event, (ev) => {
      const next = { ...start, w: start.w + (ev.clientX - startX), h: start.h + (ev.clientY - startY) };
      localStorage.setItem("yello-agent-panel-float", JSON.stringify(next));
      applyPanelGeometry(true);
    });
  });
  let resizeTimer = null;
  window.addEventListener("resize", () => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(placeAgent, 150);
  });
  placeAgent();
  const input = $("agent-input");
  const growInput = () => {
    input.style.height = "auto";
    input.style.height = `${Math.min(input.scrollHeight, 140)}px`;
  };
  input.addEventListener("input", growInput);
  growInput();
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      sendAgentMessage();
    }
  });
  loadModels();
  restoreLastChat();
}

function greet() {
  addMessage(
    "assistant",
    "I can edit the design you have open: add or rearrange elements, restyle, create pages — then save and push it to your screens. Try: “Add a big clock and today's date to the main page.”",
  );
}
