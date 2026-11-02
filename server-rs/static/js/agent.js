// The in-Studio agent: a chat panel that talks to OpenRouter directly from
// the browser (the user's key never touches this server — it lives in
// localStorage and is sent only to openrouter.ai). The model drives the
// Studio through client-side tools that reuse the editor's own validated
// paths: set_design_json goes through the same /api/layout/validate the
// JSON editor uses, save_design is the Studio's own saveLayout().

import { $ } from "./util.js";
import { S, activePageIdFor, homePageIdOf, pageOf } from "./state.js";
import { fetchJson, postLayout } from "./api.js";
import { pushHistory } from "./history.js";
import { renderAll } from "./editor.js";
import { schedulePreview, syncCodeEditor } from "./canvas.js";
import { loadDesign, saveLayout } from "./views.js";
import { toast } from "./dialog.js";

const KEY_STORAGE = "yello-openrouter-key";
const MODEL_STORAGE = "yello-openrouter-model";
// Any OpenAI-compatible endpoint works; override via localStorage
// "yello-openrouter-base" (used by the smoke tests with a mock server).
const API_BASE = localStorage.getItem("yello-openrouter-base") || "https://openrouter.ai/api/v1";
const MAX_TOOL_ROUNDS = 10;

let initialized = false;
let sending = false;
let aborter = null;
const conversation = []; // OpenAI-format messages (system prompt added per request)

const SYSTEM_PROMPT = `You are the agent built into Yello Studio, a designer for 240x320 (portrait) or 320x240 (landscape) ESP32 touch displays. You edit the design the user currently has open by calling tools.

Designs are schema v2 documents: {schemaVersion: 2, name, orientation, background, homePageId, pages: [{id, name, background, elements}], sharedElements, dataSources, actions}. Each element is {id, type, name, frame: {x, y, w, h}, style, props, visible, locked}. Coordinates are pixels on the page (portrait pages are 240 wide, 320 tall). Supported types: text, clock, external-text, button, design-link, image, color-block, ha-state, ha-toggle, script, screen (embeds another design's page), gauge, qr, shape. Shared elements render only on the pages listed in their own pages array.

How to work:
- Always call get_current_design first and edit the JSON it returns — its elements are the ground truth for style/props shapes (e.g. style: {color, fontSize, align, verticalAlign}; clock props: {format (strftime like "%H:%M"), timezone}; text props: {text}).
- To change the design, call set_design_json with the COMPLETE updated document as a JSON string. It is validated against the device schema and applied to the editor (the user can undo). If validation returns errors, fix them and retry.
- After a successful set_design_json, call save_design in the same turn (saving pushes the design to the screens using it), unless the user asked only to draft or preview.
- Use validate_design to check a document without applying it.
- Keep frames inside the page, avoid overlaps unless asked, and give new elements unique ids.
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
      name: "set_design_json",
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
      name: "validate_design",
      description: "Validate a complete design document (JSON string) against the device schema without applying it. Returns the normalized document summary or the validation errors.",
      parameters: {
        type: "object",
        properties: { json: { type: "string", description: "The design document as a JSON string." } },
        required: ["json"],
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
      name: "open_design",
      description: "Open a different design in the editor by id (switches what get_current_design / set_design_json operate on).",
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

async function toolValidateDesign(args) {
  const doc = JSON.parse(args.json);
  const { response, result } = await validateDoc(doc);
  if (!response.ok) {
    return { valid: false, errors: result.details || result.errors || [{ path: "$", message: result.error || "Validation failed" }] };
  }
  return { valid: true, summary: designSummary(result.layout) };
}

async function toolSetDesignJson(args) {
  const doc = JSON.parse(args.json);
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
  S.selectedId = null;
  S.dirty = true;
  syncCodeEditor(true);
  renderAll();
  schedulePreview(0);
  return { applied: true, summary: designSummary(S.layout), note: "Applied to the editor (not saved yet — call save_design to persist)." };
}

async function toolSaveDesign() {
  await saveLayout();
  return { saved: true, dirty: S.dirty };
}

async function toolListDesigns() {
  return S.studio.designs.map((d) => ({ id: d.id, name: d.name, revision: d.revision, pages: d.pageCount, orientation: d.orientation }));
}

async function toolListScreens() {
  return S.studio.screens.map((s) => ({ id: s.id, name: s.name, connected: s.connected, designId: s.designId }));
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
  set_design_json: toolSetDesignJson,
  validate_design: toolValidateDesign,
  save_design: toolSaveDesign,
  list_designs: toolListDesigns,
  list_screens: toolListScreens,
  open_design: toolOpenDesign,
  assign_design: toolAssignDesign,
};

async function runTool(name, argsJson) {
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
  const box = $("agent-messages");
  const el = document.createElement("div");
  el.className = `msg ${role}`;
  el.innerHTML = role === "assistant" ? markdownLite(text) : escapeHtml(text).replace(/\n/g, "<br>");
  box.append(el);
  box.scrollTop = box.scrollHeight;
  return el;
}

function addToolChip(name, ok) {
  const box = $("agent-messages");
  const el = document.createElement("div");
  el.className = "tool-chip";
  el.textContent = `${ok ? "✓" : "✕"} ${name}`;
  box.append(el);
  box.scrollTop = box.scrollHeight;
}

// ---------- OpenRouter ----------

function apiKey() {
  return localStorage.getItem(KEY_STORAGE) || "";
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
    const stored = localStorage.getItem(MODEL_STORAGE);
    select.replaceChildren();
    for (const model of data.sort((a, b) => (a.name || a.id).localeCompare(b.name || b.id))) {
      const option = document.createElement("option");
      option.value = model.id;
      const inPrice = Number(model.pricing?.prompt || 0) * 1e6;
      const outPrice = Number(model.pricing?.completion || 0) * 1e6;
      option.textContent = inPrice === 0 && outPrice === 0
        ? `${model.name || model.id} · free`
        : `${model.name || model.id} · $${inPrice.toFixed(2)}/$${outPrice.toFixed(2)} per 1M`;
      select.append(option);
    }
    if (stored && [...select.options].some((o) => o.value === stored)) select.value = stored;
    else if ([...select.options].some((o) => o.value.includes("claude-sonnet"))) {
      select.value = [...select.options].find((o) => o.value.includes("claude-sonnet")).value;
    }
    status.textContent = `${data.length} models`;
  } catch (error) {
    status.textContent = `Could not load models: ${error.message}`;
  }
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
      tools: TOOLS,
      stream: true,
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
  const toolCalls = new Map();
  let bubble = null;
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
      const delta = chunk.choices?.[0]?.delta || {};
      if (delta.content) {
        content += delta.content;
        if (!bubble) bubble = addMessage("assistant", "");
        bubble.innerHTML = markdownLite(content);
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
  return { content, toolCalls: [...toolCalls.values()] };
}

async function sendAgentMessage() {
  const input = $("agent-input");
  const text = input.value.trim();
  if (!text || sending) return;
  if (!apiKey()) {
    addMessage("assistant", "Add your OpenRouter API key above first — it stays in this browser and is only sent to OpenRouter.");
    return;
  }
  if (!$("agent-model").value) {
    addMessage("assistant", "Pick a model first (the list loads from OpenRouter).");
    return;
  }
  localStorage.setItem(MODEL_STORAGE, $("agent-model").value);
  input.value = "";
  sending = true;
  $("agent-send").disabled = true;
  $("agent-stop").classList.remove("hidden");
  aborter = new AbortController();
  addMessage("user", text);
  conversation.push({ role: "user", content: text });
  try {
    for (let round = 0; round < MAX_TOOL_ROUNDS; round += 1) {
      const context = {
        role: "system",
        content: `Current context: design "${S.layout?.name || S.currentDesignId}" (id ${S.currentDesignId}) is open in the editor${S.dirty ? " with unsaved changes" : ""}. ${S.studio.screens.length} screen(s) known, ${S.studio.screens.filter((s) => s.connected).length} connected.`,
      };
      const { content, toolCalls } = await streamChat([
        { role: "system", content: SYSTEM_PROMPT },
        context,
        ...conversation,
      ]);
      const assistantMessage = { role: "assistant", content: content || null };
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
        const result = await runTool(call.name, call.arguments);
        const failed = Boolean(result && (result.error || result.applied === false || result.valid === false));
        addToolChip(call.name, !failed);
        conversation.push({
          role: "tool",
          tool_call_id: call.id,
          content: JSON.stringify(result).slice(0, 60000),
        });
      }
    }
  } catch (error) {
    if (error.name !== "AbortError") addMessage("assistant", `⚠ ${error.message}`);
  } finally {
    sending = false;
    $("agent-send").disabled = false;
    $("agent-stop").classList.add("hidden");
    aborter = null;
  }
}

// ---------- page setup ----------

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
    $("agent-key-state").textContent = value ? "Key saved in this browser" : "No key saved";
    toast(value ? "OpenRouter key saved in this browser" : "OpenRouter key cleared");
    loadModels();
  };
  $("agent-models-refresh").onclick = loadModels;
  $("agent-model").onchange = () => localStorage.setItem(MODEL_STORAGE, $("agent-model").value);
  $("agent-send").onclick = sendAgentMessage;
  $("agent-stop").onclick = () => aborter?.abort();
  $("agent-clear").onclick = () => {
    conversation.length = 0;
    $("agent-messages").replaceChildren();
    greet();
  };
  $("agent-input").addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      sendAgentMessage();
    }
  });
  loadModels();
  greet();
}

function greet() {
  addMessage(
    "assistant",
    "I can edit the design you have open: add or rearrange elements, restyle, create pages — then save and push it to your screens. Try: “Add a big clock and today's date to the main page.”",
  );
}
