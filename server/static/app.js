"use strict";

const PORTRAIT_WIDTH = 240;
const PORTRAIT_HEIGHT = 320;
const ZOOM_LEVELS = [1, 1.5, 2];

let layout = null;
let catalog = null;
let homeAssistant = { configured: false, entities: [] };
let selectedId = null;
let appliedRevision = 0;
let dirty = false;
let zoomIndex = 2;
let undoStack = [];
let redoStack = [];
let previewTimer = null;
let previewUrl = null;
let previewController = null;
let studio = { designs: [], screens: [] };
let currentDesignId = "default";
let selectedScreenId = "";
let currentPage = "designer";
let homeAssistantSettings = { url: "", configured: false, tokenConfigured: false };
let liveMode = false;
let canvasMode = "edit";
let codeEditorDirty = false;
let activeModal = null;

const $ = (id) => document.getElementById(id);
const clone = (value) => structuredClone(value);
const displayWidth = () => layout?.orientation === "landscape" ? 320 : PORTRAIT_WIDTH;
const displayHeight = () => layout?.orientation === "landscape" ? 240 : PORTRAIT_HEIGHT;

const TYPE_META = {
  text: { label: "Text", icon: "T" },
  clock: { label: "Clock", icon: "◷" },
  "external-text": { label: "External text", icon: "↗" },
  button: { label: "Button", icon: "▣" },
  "design-link": { label: "Screen button", icon: "↪" },
  image: { label: "Image", icon: "▧" },
  "color-block": { label: "Color block", icon: "■" },
  "ha-state": { label: "HA sensor", icon: "⌁" },
  "ha-toggle": { label: "HA toggle", icon: "⏻" },
  script: { label: "Script", icon: "{ }" },
};

function pageFromPath() {
  const name = window.location.pathname.replace(/^\//, "");
  return ["designer", "devices", "designs", "settings"].includes(name) ? name : "designer";
}

function showPage(page, push = false) {
  currentPage = page;
  for (const name of ["designer", "devices", "designs", "settings"]) {
    $(`${name}-page`).classList.toggle("hidden", name !== page);
  }
  $("designer-actions").classList.toggle("hidden", page !== "designer");
  document.querySelectorAll("[data-route]").forEach((link) => {
    link.classList.toggle("active", link.dataset.route === page);
  });
  if (push && window.location.pathname !== `/${page}`) history.pushState({ page }, "", `/${page}`);
  if (page === "devices") renderDevicePage();
  if (page === "designs") renderDesignPage();
  if (page === "settings") loadHomeAssistantSettings().catch((error) => toast(error.message, true));
}

function selected() {
  return layout?.elements.find((element) => element.id === selectedId) || null;
}

function uniqueId(prefix) {
  let index = Date.now().toString(36);
  let candidate = `${prefix}${index}`;
  const allIds = new Set([
    ...layout.elements.map((item) => item.id),
    ...layout.dataSources.map((item) => item.id),
    ...layout.actions.map((item) => item.id),
  ]);
  while (allIds.has(candidate)) candidate = `${prefix}${index}_${Math.floor(Math.random() * 1000)}`;
  return candidate;
}

function pushHistory() {
  undoStack.push(clone(layout));
  if (undoStack.length > 80) undoStack.shift();
  redoStack = [];
}

function commit(mutator) {
  pushHistory();
  mutator();
  dirty = true;
  renderAll();
  schedulePreview();
}

function undo() {
  if (!undoStack.length) return;
  redoStack.push(clone(layout));
  layout = undoStack.pop();
  if (!selected()) selectedId = null;
  dirty = true;
  renderAll();
  schedulePreview();
}

function redo() {
  if (!redoStack.length) return;
  undoStack.push(clone(layout));
  layout = redoStack.pop();
  if (!selected()) selectedId = null;
  dirty = true;
  renderAll();
  schedulePreview();
}

function setDocumentState(message = "") {
  const designName = studio.designs.find((item) => item.id === currentDesignId)?.name || layout?.name || "Design";
  const changed = dirty || codeEditorDirty;
  $("document-state").textContent = message || `${designName} · ${changed ? codeEditorDirty ? "JSON changes not applied" : "Unsaved changes" : `Revision ${appliedRevision}`}`;
  $("save").disabled = !changed;
  $("undo").disabled = undoStack.length === 0;
  $("redo").disabled = redoStack.length === 0;
}

function toast(message, error = false) {
  const item = document.createElement("div");
  item.className = `toast${error ? " error" : ""}`;
  item.textContent = message;
  $("toast-region").append(item);
  setTimeout(() => item.remove(), 3500);
}

function closeAppModal(value) {
  if (!activeModal) return;
  const { resolve, returnFocus } = activeModal;
  activeModal = null;
  $("app-modal-backdrop").classList.add("hidden");
  $("app-modal-confirm").classList.remove("app-modal-confirm-danger");
  resolve(value);
  if (returnFocus?.isConnected) returnFocus.focus();
}

function showAppModal({
  title, message, confirmLabel = "Confirm", cancelLabel = "Cancel", danger = false,
  inputLabel = "", inputValue = "", inputPlaceholder = "", inputMaxLength = 100,
}) {
  if (activeModal) closeAppModal(null);
  const hasInput = Boolean(inputLabel);
  $("app-modal-title").textContent = title;
  $("app-modal-message").textContent = message;
  $("app-modal-cancel").textContent = cancelLabel;
  $("app-modal-confirm").textContent = confirmLabel;
  $("app-modal-confirm").classList.toggle("app-modal-confirm-danger", danger);
  $("app-modal-input-label").textContent = inputLabel;
  $("app-modal-input-label").classList.toggle("hidden", !hasInput);
  $("app-modal-input").classList.toggle("hidden", !hasInput);
  $("app-modal-input").value = inputValue;
  $("app-modal-input").placeholder = inputPlaceholder;
  $("app-modal-input").maxLength = inputMaxLength;
  $("app-modal-backdrop").classList.remove("hidden");
  return new Promise((resolve) => {
    activeModal = { resolve, returnFocus: document.activeElement, hasInput };
    requestAnimationFrame(() => (hasInput ? $("app-modal-input") : $("app-modal-confirm")).focus());
  });
}

async function appConfirm(options) {
  return Boolean(await showAppModal(options));
}

async function appPrompt(options) {
  const value = await showAppModal(options);
  return typeof value === "string" ? value : null;
}

function addElement(type) {
  const template = catalog?.elementDefaults?.[type];
  if (!template) {
    toast("This block is not available from the running server yet. Refresh after the server restarts.", true);
    return;
  }
  const element = clone(template);
  element.id = uniqueId("element");
  element.name = TYPE_META[type].label;
  commit(() => {
    if (type === "external-text") {
      const sourceId = uniqueId("source");
      layout.dataSources.push({
        id: sourceId, name: "External text source", type: "http-text",
        url: "https://wttr.in/?format=3", interval: 300,
      });
      element.props.sourceId = sourceId;
    }
    if (type === "design-link") {
      element.props.targetDesignId = studio.designs.find((item) => item.id !== currentDesignId)?.id || currentDesignId;
    }
    layout.elements.push(element);
    selectedId = element.id;
  });
}

function duplicateSelected() {
  const element = selected();
  if (!element) return;
  commit(() => {
    const duplicate = clone(element);
    duplicate.id = uniqueId("element");
    duplicate.name = `${element.name} copy`;
    duplicate.frame.x = Math.min(displayWidth() - duplicate.frame.w, Math.max(0, duplicate.frame.x + 8));
    duplicate.frame.y = Math.min(displayHeight() - duplicate.frame.h, Math.max(0, duplicate.frame.y + 8));
    layout.elements.push(duplicate);
    selectedId = duplicate.id;
  });
}

function deleteSelected() {
  if (!selected()) return;
  commit(() => {
    layout.elements = layout.elements.filter((element) => element.id !== selectedId);
    selectedId = null;
  });
}

function setOrientation(orientation) {
  if (orientation === (layout.orientation || "portrait")) return;
  const oldWidth = displayWidth();
  const oldHeight = displayHeight();
  const newWidth = orientation === "landscape" ? 320 : 240;
  const newHeight = orientation === "landscape" ? 240 : 320;
  commit(() => {
    layout.orientation = orientation;
    for (const element of layout.elements) {
      element.frame.x = Math.max(0, Math.round(element.frame.x * newWidth / oldWidth));
      element.frame.y = Math.max(0, Math.round(element.frame.y * newHeight / oldHeight));
      element.frame.w = Math.max(1, Math.min(newWidth, Math.round(element.frame.w * newWidth / oldWidth)));
      element.frame.h = Math.max(1, Math.min(newHeight, Math.round(element.frame.h * newHeight / oldHeight)));
      element.frame.x = Math.min(newWidth - element.frame.w, element.frame.x);
      element.frame.y = Math.min(newHeight - element.frame.h, element.frame.y);
    }
  });
}

function alignSelected(action) {
  const element = selected();
  if (!element) return;
  commit(() => {
    if (action === "align-left") element.frame.x = 0;
    if (action === "align-center") element.frame.x = Math.round((displayWidth() - element.frame.w) / 2);
    if (action === "align-right") element.frame.x = displayWidth() - element.frame.w;
    if (action === "align-top") element.frame.y = 0;
    if (action === "align-middle") element.frame.y = Math.round((displayHeight() - element.frame.h) / 2);
    if (action === "align-bottom") element.frame.y = displayHeight() - element.frame.h;
  });
}

function hideContextMenu() {
  $("canvas-menu").classList.add("hidden");
}

function showContextMenu(event, element) {
  event.preventDefault();
  event.stopPropagation();
  selectedId = element.id;
  renderAll();
  const menu = $("canvas-menu");
  menu.classList.remove("hidden");
  const left = Math.min(event.clientX, window.innerWidth - 188);
  const top = Math.min(event.clientY, window.innerHeight - 285);
  menu.style.left = `${Math.max(6, left)}px`;
  menu.style.top = `${Math.max(6, top)}px`;
}

function moveLayer(direction) {
  const index = layout.elements.findIndex((element) => element.id === selectedId);
  const target = index + direction;
  if (index < 0 || target < 0 || target >= layout.elements.length) return;
  commit(() => {
    [layout.elements[index], layout.elements[target]] = [layout.elements[target], layout.elements[index]];
  });
}

function renderDocumentFields() {
  $("screen-name").value = layout.name;
  $("background").value = layout.background;
  $("background-value").textContent = layout.background;
  $("orientation").value = layout.orientation || "portrait";
  $("canvas-dimensions").textContent = `${displayWidth()} × ${displayHeight()}`;
  applyZoom();
}

function renderLayers() {
  const list = $("layer-list");
  list.replaceChildren();
  $("layer-count").textContent = `${layout.elements.length} block${layout.elements.length === 1 ? "" : "s"}`;
  [...layout.elements].reverse().forEach((element) => {
    const row = document.createElement("div");
    row.className = `layer-row${element.id === selectedId ? " selected" : ""}${element.visible ? "" : " hidden-layer"}`;
    row.onclick = () => {
      selectedId = element.id;
      renderAll();
    };

    const type = document.createElement("span");
    type.className = "layer-type";
    type.textContent = TYPE_META[element.type].icon;
    const name = document.createElement("span");
    name.className = "layer-name";
    name.textContent = element.name;
    const visibility = document.createElement("button");
    visibility.title = element.visible ? "Hide layer" : "Show layer";
    visibility.textContent = element.visible ? "◉" : "○";
    visibility.onclick = (event) => {
      event.stopPropagation();
      commit(() => { element.visible = !element.visible; });
    };
    const lock = document.createElement("button");
    lock.title = element.locked ? "Unlock layer" : "Lock layer";
    lock.textContent = element.locked ? "▣" : "▢";
    lock.onclick = (event) => {
      event.stopPropagation();
      commit(() => { element.locked = !element.locked; });
    };
    row.append(type, name, visibility, lock);
    list.append(row);
  });
}

function renderOverlay() {
  const overlay = $("canvas-overlay");
  overlay.replaceChildren();
  layout.elements.forEach((element) => {
    if (!element.visible) return;
    const frame = element.frame;
    const box = document.createElement("div");
    box.className = `element-box${element.id === selectedId ? " selected" : ""}${element.locked ? " locked" : ""}`;
    box.dataset.elementId = element.id;
    Object.assign(box.style, {
      left: `${frame.x}px`, top: `${frame.y}px`, width: `${frame.w}px`, height: `${frame.h}px`,
    });
    box.onpointerdown = (event) => beginPointerInteraction(event, element, null);
    box.oncontextmenu = (event) => showContextMenu(event, element);
    if (element.id === selectedId && !element.locked) {
      for (const handleName of ["nw", "ne", "sw", "se"]) {
        const handle = document.createElement("span");
        handle.className = "resize-handle";
        handle.dataset.handle = handleName;
        handle.onpointerdown = (event) => beginPointerInteraction(event, element, handleName);
        box.append(handle);
      }
      const tag = document.createElement("span");
      tag.className = "size-tag";
      tag.textContent = `${frame.w} × ${frame.h}`;
      box.append(tag);
    }
    overlay.append(box);
  });
}

function beginPointerInteraction(event, element, handle) {
  event.preventDefault();
  event.stopPropagation();
  if (selectedId !== element.id) {
    selectedId = element.id;
    renderLayers();
    renderInspector();
    event.currentTarget.closest(".element-box")?.classList.add("selected");
  }
  if (element.locked) return;
  const zoom = ZOOM_LEVELS[zoomIndex];
  const origin = clone(element.frame);
  const startX = event.clientX;
  const startY = event.clientY;
  let started = false;
  const target = event.currentTarget;
  const box = target.closest(".element-box");
  target.setPointerCapture(event.pointerId);

  const move = (moveEvent) => {
    const dx = Math.round((moveEvent.clientX - startX) / zoom);
    const dy = Math.round((moveEvent.clientY - startY) / zoom);
    if (!started && (dx !== 0 || dy !== 0)) {
      pushHistory();
      started = true;
      dirty = true;
    }
    if (!started) return;
    if (!handle) {
      element.frame.x = Math.max(0, Math.min(displayWidth() - element.frame.w, origin.x + dx));
      element.frame.y = Math.max(0, Math.min(displayHeight() - element.frame.h, origin.y + dy));
    } else {
      resizeFrame(element.frame, origin, handle, dx, dy);
    }
    Object.assign(box.style, {
      left: `${element.frame.x}px`, top: `${element.frame.y}px`,
      width: `${element.frame.w}px`, height: `${element.frame.h}px`,
    });
    const sizeTag = box.querySelector(".size-tag");
    if (sizeTag) sizeTag.textContent = `${element.frame.w} × ${element.frame.h}`;
    setDocumentState();
    schedulePreview();
  };
  const end = () => {
    target.removeEventListener("pointermove", move);
    target.removeEventListener("pointerup", end);
    target.removeEventListener("pointercancel", end);
    renderAll();
  };
  target.addEventListener("pointermove", move);
  target.addEventListener("pointerup", end);
  target.addEventListener("pointercancel", end);
}

function resizeFrame(frame, origin, handle, dx, dy) {
  const minSize = 8;
  let left = origin.x;
  let top = origin.y;
  let right = origin.x + origin.w;
  let bottom = origin.y + origin.h;
  if (handle.includes("w")) left = Math.max(0, Math.min(right - minSize, origin.x + dx));
  if (handle.includes("e")) right = Math.min(displayWidth(), Math.max(left + minSize, origin.x + origin.w + dx));
  if (handle.includes("n")) top = Math.max(0, Math.min(bottom - minSize, origin.y + dy));
  if (handle.includes("s")) bottom = Math.min(displayHeight(), Math.max(top + minSize, origin.y + origin.h + dy));
  Object.assign(frame, { x: left, y: top, w: right - left, h: bottom - top });
}

function createInput(label, value, options) {
  const labelElement = document.createElement("label");
  labelElement.textContent = label;
  let input;
  if (options.type === "select") {
    input = document.createElement("select");
    for (const [optionValue, optionLabel] of options.options) {
      const option = document.createElement("option");
      option.value = optionValue;
      option.textContent = optionLabel;
      input.append(option);
    }
    input.value = value;
  } else if (options.type === "textarea") {
    input = document.createElement("textarea");
    input.value = value;
  } else {
    input = document.createElement("input");
    input.type = options.type || "text";
    if (input.type === "checkbox") input.checked = Boolean(value);
    else input.value = value ?? "";
    if (options.min !== undefined) input.min = options.min;
    if (options.max !== undefined) input.max = options.max;
  }
  return [labelElement, input];
}

function appendField(container, label, value, options, setter) {
  const [labelElement, input] = createInput(label, value, options);
  let historyCaptured = false;
  input.oninput = () => {
    if (!historyCaptured) {
      pushHistory();
      historyCaptured = true;
    }
    const next = input.type === "number" ? Number(input.value) : input.type === "checkbox" ? input.checked : input.value;
    setter(next);
    dirty = true;
    renderOverlay();
    setDocumentState();
    schedulePreview();
  };
  input.onchange = () => renderAll();
  container.append(labelElement, input);
  return input;
}

function appendHomeAssistantEntityField(container, value, setter) {
  const input = appendField(container, "Entity", value, { type: "text" }, setter);
  input.setAttribute("list", "ha-entity-options");
  input.placeholder = homeAssistant.configured ? "sensor.temperature" : "Configure Home Assistant in Settings";
  return input;
}

function renderInspector() {
  const container = $("inspector");
  const element = selected();
  $("duplicate").disabled = !element;
  if (!element) {
    container.className = "inspector-empty";
    container.innerHTML = '<div class="empty-icon">◇</div><p>Select a block on the canvas or in Layers.</p>';
    renderResourceEditor(null);
    return;
  }
  container.className = "inspector-fields";
  container.replaceChildren();
  appendField(container, "Name", element.name, { type: "text" }, (value) => { element.name = value; });

  const coordinateGrid = document.createElement("div");
  coordinateGrid.className = "coordinate-grid";
  for (const key of ["x", "y", "w", "h"]) {
    const field = document.createElement("div");
    field.className = "coordinate-field";
    const prefix = document.createElement("span");
    prefix.textContent = key.toUpperCase();
    const input = document.createElement("input");
    input.type = "number";
    input.value = element.frame[key];
    input.min = key === "w" || key === "h" ? "1" : "0";
    input.max = key === "x" || key === "w" ? String(displayWidth()) : String(displayHeight());
    input.onchange = () => commit(() => {
      const maximum = key === "x" ? displayWidth() - element.frame.w : key === "y" ? displayHeight() - element.frame.h : key === "w" ? displayWidth() - element.frame.x : displayHeight() - element.frame.y;
      element.frame[key] = Math.max(key === "w" || key === "h" ? 1 : 0, Math.min(maximum, Number(input.value)));
    });
    field.append(prefix, input);
    coordinateGrid.append(field);
  }
  container.append(coordinateGrid);

  const colorField = () => appendField(container, "Color", element.style.color, { type: "color" }, (value) => { element.style.color = value; });
  const fontFields = () => {
    appendField(container, "Font size", element.style.fontSize, { type: "number", min: 6, max: 120 }, (value) => { element.style.fontSize = value; });
    colorField();
  };
  const alignmentFields = () => {
    appendField(container, "Align", element.style.align, { type: "select", options: [["left", "Left"], ["center", "Center"], ["right", "Right"]] }, (value) => { element.style.align = value; });
    appendField(container, "Vertical", element.style.verticalAlign, { type: "select", options: [["top", "Top"], ["middle", "Middle"], ["bottom", "Bottom"]] }, (value) => { element.style.verticalAlign = value; });
  };

  if (element.type === "text") {
    appendField(container, "Text", element.props.text, { type: "textarea" }, (value) => { element.props.text = value; });
    fontFields(); alignmentFields();
    appendField(container, "Wrap", element.props.wrap, { type: "checkbox" }, (value) => { element.props.wrap = value; });
  } else if (element.type === "clock") {
    appendField(container, "Format", element.props.format, { type: "text" }, (value) => { element.props.format = value; });
    appendField(container, "Timezone", element.props.timezone, { type: "text" }, (value) => { element.props.timezone = value; });
    fontFields(); alignmentFields();
  } else if (element.type === "external-text") {
    fontFields(); alignmentFields();
    appendField(container, "Wrap", element.props.wrap, { type: "checkbox" }, (value) => { element.props.wrap = value; });
  } else if (element.type === "button") {
    appendField(container, "Label", element.props.label, { type: "text" }, (value) => { element.props.label = value; });
    fontFields();
    appendField(container, "Radius", element.style.radius, { type: "number", min: 0, max: 50 }, (value) => { element.style.radius = value; });
  } else if (element.type === "design-link") {
    appendField(container, "Label", element.props.label, { type: "text" }, (value) => { element.props.label = value; });
    appendField(
      container, "Target screen", element.props.targetDesignId,
      { type: "select", options: studio.designs.map((item) => [item.id, item.name]) },
      (value) => { element.props.targetDesignId = value; },
    );
    fontFields();
    appendField(container, "Radius", element.style.radius, { type: "number", min: 0, max: 50 }, (value) => { element.style.radius = value; });
    const help = document.createElement("p");
    help.className = "integration-help";
    help.textContent = "Opens the target design on only the device that pressed it. The device's assigned home design is unchanged.";
    container.append(help);
  } else if (element.type === "color-block") {
    colorField();
    appendField(container, "Radius", element.style.radius, { type: "number", min: 0, max: 50 }, (value) => { element.style.radius = value; });
  } else if (element.type === "image") {
    appendField(container, "Image URL", element.props.src, { type: "text" }, (value) => { element.props.src = value; });
    appendField(container, "Fit", element.props.fit, { type: "select", options: [["contain", "Contain"], ["cover", "Cover"], ["stretch", "Stretch"]] }, (value) => { element.props.fit = value; });
  } else if (element.type === "ha-state") {
    appendHomeAssistantEntityField(container, element.props.entityId, (value) => { element.props.entityId = value; });
    appendField(container, "Attribute", element.props.attribute, { type: "text" }, (value) => { element.props.attribute = value; });
    appendField(container, "Prefix", element.props.prefix, { type: "text" }, (value) => { element.props.prefix = value; });
    appendField(container, "Suffix", element.props.suffix, { type: "text" }, (value) => { element.props.suffix = value; });
    appendField(container, "Decimals", element.props.decimals, { type: "number", min: 0, max: 6 }, (value) => { element.props.decimals = value; });
    appendField(container, "Auto unit", element.props.showUnit, { type: "checkbox" }, (value) => { element.props.showUnit = value; });
    appendField(container, "Refresh (s)", element.props.refreshInterval, { type: "number", min: 1, max: 3600 }, (value) => { element.props.refreshInterval = value; });
    fontFields(); alignmentFields();
    appendField(container, "Wrap", element.props.wrap, { type: "checkbox" }, (value) => { element.props.wrap = value; });
  } else if (element.type === "ha-toggle") {
    appendHomeAssistantEntityField(container, element.props.entityId, (value) => { element.props.entityId = value; });
    appendField(container, "Label", element.props.label, { type: "text" }, (value) => { element.props.label = value; });
    appendField(container, "Font size", element.style.fontSize, { type: "number", min: 6, max: 120 }, (value) => { element.style.fontSize = value; });
    appendField(container, "On color", element.style.color, { type: "color" }, (value) => { element.style.color = value; });
    appendField(container, "Off color", element.style.offColor, { type: "color" }, (value) => { element.style.offColor = value; });
    appendField(container, "Radius", element.style.radius, { type: "number", min: 0, max: 50 }, (value) => { element.style.radius = value; });
    appendField(container, "Refresh (s)", element.props.refreshInterval, { type: "number", min: 1, max: 3600 }, (value) => { element.props.refreshInterval = value; });
  } else if (element.type === "script") {
    const examples = [["", "Choose an example…"], ...(catalog.scriptExamples || []).map((item) => [item.id, item.name])];
    appendField(container, "Example", "", { type: "select", options: examples }, (value) => {
      const example = catalog.scriptExamples?.find((item) => item.id === value);
      if (example) element.props.code = example.code;
    });
    const editor = appendField(container, "Code", element.props.code, { type: "textarea" }, (value) => { element.props.code = value; });
    editor.classList.add("script-editor");
    appendField(container, "Refresh (s)", element.props.refreshInterval, { type: "number", min: 1, max: 3600 }, (value) => { element.props.refreshInterval = value; });
    fontFields(); alignmentFields();
    appendField(container, "Wrap", element.props.wrap, { type: "checkbox" }, (value) => { element.props.wrap = value; });
    const help = document.createElement("p");
    help.className = "integration-help";
    help.textContent = "Allowed: text, ha, now, number, clamp, rect, line, circle, label, clear, bounded range loops, and basic math. Scripts cannot access files or the network.";
    container.append(help);
  }

  if (["ha-state", "ha-toggle", "script"].includes(element.type) && !homeAssistant.configured) {
    const help = document.createElement("p");
    help.className = "integration-help";
    help.textContent = "Home Assistant is not configured. Add its URL and token on the Settings page to use ha().";
    container.append(help);
  }

  const actions = document.createElement("div");
  actions.className = "inspector-actions";
  const backward = document.createElement("button");
  backward.textContent = "Send backward";
  backward.onclick = () => moveLayer(-1);
  const forward = document.createElement("button");
  forward.textContent = "Bring forward";
  forward.onclick = () => moveLayer(1);
  const remove = document.createElement("button");
  remove.className = "danger";
  remove.textContent = "Delete";
  remove.onclick = deleteSelected;
  actions.append(backward, forward, remove);
  container.append(actions);
  renderResourceEditor(element);
}

function renderResourceEditor(element) {
  const section = $("resource-section");
  const editor = $("resource-editor");
  editor.replaceChildren();
  if (!element || !["external-text", "button"].includes(element.type)) {
    section.classList.add("hidden");
    return;
  }
  section.classList.remove("hidden");
  const fields = document.createElement("div");
  fields.className = "inspector-fields";
  if (element.type === "external-text") {
    $("resource-title").textContent = "Data source";
    const source = layout.dataSources.find((item) => item.id === element.props.sourceId);
    if (!source) return;
    appendField(fields, "Name", source.name, { type: "text" }, (value) => { source.name = value; });
    appendField(fields, "URL", source.url, { type: "text" }, (value) => { source.url = value; });
    appendField(fields, "Refresh", source.interval, { type: "number", min: 5, max: 86400 }, (value) => { source.interval = value; });
    const help = document.createElement("p");
    help.className = "resource-help wide";
    help.textContent = "Private network URLs are blocked unless the server explicitly allowlists their host.";
    fields.append(help);
  } else {
    $("resource-title").textContent = "Button action";
    let action = layout.actions.find((item) => item.id === element.props.actionId);
    if (!action) {
      const create = document.createElement("button");
      create.className = "primary-button wide";
      create.textContent = "Create HTTP action";
      create.onclick = () => commit(() => {
        const actionId = uniqueId("action");
        layout.actions.push({ id: actionId, name: "Button action", type: "http", method: "GET", url: "", body: "" });
        element.props.actionId = actionId;
      });
      fields.append(create);
    } else {
      appendField(fields, "Name", action.name, { type: "text" }, (value) => { action.name = value; });
      appendField(fields, "Method", action.method, { type: "select", options: [["GET", "GET"], ["POST", "POST"]] }, (value) => { action.method = value; });
      appendField(fields, "URL", action.url, { type: "text" }, (value) => { action.url = value; });
      if (action.method === "POST") appendField(fields, "Body", action.body, { type: "textarea" }, (value) => { action.body = value; });
    }
  }
  editor.append(fields);
}

function renderAll() {
  renderDocumentFields();
  renderLayers();
  renderOverlay();
  renderInspector();
  if (canvasMode === "code" && !codeEditorDirty) syncCodeEditor(true);
  setDocumentState();
}

function schedulePreview(delay = 100) {
  clearTimeout(previewTimer);
  previewTimer = setTimeout(refreshPreview, delay);
}

function setCodeEditorState(message, state = "") {
  const label = $("code-editor-state");
  label.textContent = message;
  label.className = state;
}

function showCodeErrors(errors = []) {
  const container = $("code-errors");
  if (!errors.length) {
    container.classList.add("hidden");
    container.textContent = "";
    return;
  }
  container.textContent = errors.map((error) => `${error.path || "$"}: ${error.message || error}`).join("\n");
  container.classList.remove("hidden");
}

function syncCodeEditor(force = false) {
  if (!layout || (codeEditorDirty && !force)) return;
  $("layout-json").value = JSON.stringify(layout, null, 2);
  codeEditorDirty = false;
  showCodeErrors();
  setCodeEditorState("Edit the complete normalized design configuration.");
}

function parseCodeEditor() {
  try {
    return { value: JSON.parse($("layout-json").value), errors: [] };
  } catch (error) {
    const match = String(error.message).match(/position (\d+)/);
    const path = match ? `character ${match[1]}` : "$";
    return { value: null, errors: [{ path, message: error.message }] };
  }
}

async function applyCodeLayout() {
  const parsed = parseCodeEditor();
  if (parsed.errors.length) {
    showCodeErrors(parsed.errors);
    setCodeEditorState("JSON syntax error", "invalid");
    return false;
  }
  $("apply-json").disabled = true;
  setCodeEditorState("Validating against the device layout schema…", "pending");
  try {
    const response = await fetch(`/api/layout/validate?design=${encodeURIComponent(currentDesignId)}`, {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(parsed.value),
    });
    const result = await response.json();
    if (!response.ok) {
      showCodeErrors(result.errors || [{ path: "$", message: result.error || "Layout validation failed" }]);
      setCodeEditorState("Layout validation failed", "invalid");
      return false;
    }
    pushHistory();
    layout = result.layout;
    selectedId = null;
    dirty = true;
    codeEditorDirty = false;
    $("layout-json").value = JSON.stringify(layout, null, 2);
    showCodeErrors();
    renderAll();
    setCodeEditorState("Valid JSON applied to the visual design", "valid");
    schedulePreview(0);
    return true;
  } catch (error) {
    showCodeErrors([{ path: "$", message: error.message }]);
    setCodeEditorState("Could not validate JSON", "invalid");
    return false;
  } finally {
    $("apply-json").disabled = false;
  }
}

function formatCodeEditor() {
  const parsed = parseCodeEditor();
  if (parsed.errors.length) {
    showCodeErrors(parsed.errors);
    setCodeEditorState("Fix the JSON syntax before formatting", "invalid");
    return;
  }
  $("layout-json").value = JSON.stringify(parsed.value, null, 2);
  codeEditorDirty = true;
  showCodeErrors();
  setCodeEditorState("Formatted · apply JSON to update the design", "pending");
  setDocumentState();
}

async function setCanvasMode(mode) {
  if (!["edit", "code", "live"].includes(mode) || mode === canvasMode) return true;
  if (canvasMode === "code" && codeEditorDirty) {
    const discard = await appConfirm({
      title: "Discard unapplied JSON?",
      message: "Your JSON editor changes have not been applied to the design.",
      confirmLabel: "Discard changes", danger: true,
    });
    if (!discard) return false;
    codeEditorDirty = false;
  }
  canvasMode = mode;
  liveMode = mode === "live";
  $("edit-mode").classList.toggle("active", mode === "edit");
  $("code-mode").classList.toggle("active", mode === "code");
  $("live-mode").classList.toggle("active", mode === "live");
  $("canvas-scroll").classList.toggle("hidden", mode === "code");
  $("code-editor-pane").classList.toggle("hidden", mode !== "code");
  $("canvas-overlay").classList.toggle("hidden", mode !== "edit");
  $("live-touch-overlay").classList.toggle("hidden", mode !== "live");
  $("zoom-out").disabled = mode === "code" || zoomIndex === 0;
  $("zoom-in").disabled = mode === "code" || zoomIndex === ZOOM_LEVELS.length - 1;
  if (mode === "code") syncCodeEditor(true);
  $("preview-status").textContent = mode === "live" ? "Interactive live render · click to touch" : mode === "code" ? "Direct JSON configuration · validate before saving" : "Authoritative server preview";
  hideContextMenu();
  setDocumentState();
  if (mode !== "code") schedulePreview(0);
  return true;
}

async function sendLiveTouch(event) {
  if (!layout || !liveMode) return;
  event.preventDefault();
  const bounds = $("live-touch-overlay").getBoundingClientRect();
  const x = Math.max(0, Math.min(displayWidth() - 1, Math.floor((event.clientX - bounds.left) * displayWidth() / bounds.width)));
  const y = Math.max(0, Math.min(displayHeight() - 1, Math.floor((event.clientY - bounds.top) * displayHeight() / bounds.height)));
  try {
    const response = await fetch(`/api/live-touch?design=${encodeURIComponent(currentDesignId)}`, {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ layout, x, y, touchType: "down" }),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || "Live touch failed");
    if (result.navigateTo) {
      await loadDesign(result.navigateTo);
      return;
    }
    refreshPreview();
    setTimeout(refreshPreview, 120);
    setTimeout(refreshPreview, 400);
  } catch (error) {
    toast(error.message, true);
  }
}

async function refreshPreview() {
  if (!layout) return;
  previewController?.abort();
  previewController = new AbortController();
  $("preview-status").textContent = "Rendering preview…";
  $("preview-status").classList.add("rendering");
  try {
    const response = await fetch(`/api/preview?design=${encodeURIComponent(currentDesignId)}${liveMode ? "&live=1" : ""}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(layout),
      signal: previewController.signal,
    });
    if (!response.ok) {
      const result = await response.json();
      throw new Error(result.errors?.[0]?.message || `Preview failed (${response.status})`);
    }
    const blob = await response.blob();
    const nextUrl = URL.createObjectURL(blob);
    $("preview").src = nextUrl;
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    previewUrl = nextUrl;
    $("preview-status").textContent = canvasMode === "code" ? "Direct JSON configuration · validate before saving" : liveMode ? "Interactive live render · click to touch" : "Authoritative server preview";
  } catch (error) {
    if (error.name !== "AbortError") $("preview-status").textContent = error.message;
  } finally {
    $("preview-status").classList.remove("rendering");
  }
}

async function saveLayout() {
  if (codeEditorDirty && !await applyCodeLayout()) {
    setDocumentState();
    return;
  }
  $("save").disabled = true;
  setDocumentState("Saving and applying…");
  try {
    const response = await fetch(`/api/layout?design=${encodeURIComponent(currentDesignId)}`, {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(layout),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.errors?.map((item) => `${item.path}: ${item.message}`).join("\n") || "Save failed");
    appliedRevision = result.revision;
    dirty = false;
    const design = studio.designs.find((item) => item.id === currentDesignId);
    if (design) {
      design.name = layout.name;
      design.revision = result.revision;
      design.orientation = layout.orientation || "portrait";
    }
    renderStudioControls();
    setDocumentState();
    toast(result.applied ? `Saved and applied to ${result.appliedScreens} screen${result.appliedScreens === 1 ? "" : "s"}` : "Saved — no assigned screen is online");
  } catch (error) {
    setDocumentState();
    toast(error.message, true);
  }
}

function renderStudioControls() {
  const screenSelect = $("screen-select");
  const designSelect = $("design-select");
  screenSelect.replaceChildren();
  if (!studio.screens.length) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "No screens discovered";
    screenSelect.append(option);
    selectedScreenId = "";
  } else {
    for (const screen of studio.screens) {
      const option = document.createElement("option");
      option.value = screen.id;
      option.textContent = `${screen.connected ? "●" : "○"} ${screen.name}`;
      screenSelect.append(option);
    }
    if (!studio.screens.some((item) => item.id === selectedScreenId)) {
      selectedScreenId = (studio.screens.find((item) => item.connected) || studio.screens[0]).id;
    }
    screenSelect.value = selectedScreenId;
  }
  designSelect.replaceChildren();
  for (const design of studio.designs) {
    const option = document.createElement("option");
    option.value = design.id;
    option.textContent = design.name;
    designSelect.append(option);
  }
  designSelect.value = currentDesignId;
  $("assign-design").disabled = !selectedScreenId;
  renderScreenList();
  renderIntegrationStatus();
}

function renderIntegrationStatus() {
  const badge = $("ha-integration-status");
  badge.className = "integration-badge";
  if (homeAssistant.error) {
    badge.textContent = "HA error";
    badge.classList.add("error");
  } else if (homeAssistant.configured) {
    badge.textContent = `HA · ${homeAssistant.entities.length} entities`;
    badge.classList.add("connected");
  } else {
    badge.textContent = "HA setup needed";
    badge.title = "Configure Home Assistant on the Settings page";
  }
}

function renderHomeAssistantOptions() {
  const options = $("ha-entity-options");
  options.replaceChildren();
  for (const entity of homeAssistant.entities) {
    const option = document.createElement("option");
    option.value = entity.entityId;
    option.label = `${entity.name} · ${entity.state}${entity.unit ? ` ${entity.unit}` : ""}`;
    options.append(option);
  }
  renderIntegrationStatus();
}

function compactNumber(value) {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1000) return `${(value / 1000).toFixed(1)}k`;
  return String(Math.round(value || 0));
}

function wifiSignalLabel(rssi) {
  if (rssi === null || rssi === undefined || rssi <= -126) return "—";
  if (rssi >= -55) return `${rssi} dBm · Excellent`;
  if (rssi >= -67) return `${rssi} dBm · Good`;
  if (rssi >= -75) return `${rssi} dBm · Fair`;
  return `${rssi} dBm · Weak`;
}

function relativeLastSeen(value) {
  if (!value) return "Never seen";
  const seconds = Math.max(0, Math.round((Date.now() - new Date(value).getTime()) / 1000));
  if (seconds < 60) return "Seen just now";
  if (seconds < 3600) return `Seen ${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `Seen ${Math.floor(seconds / 3600)}h ago`;
  return `Seen ${Math.floor(seconds / 86400)}d ago`;
}

function renderDevicePage() {
  const grid = $("device-grid");
  grid.replaceChildren();
  const online = studio.screens.filter((screen) => screen.connected).length;
  $("device-page-summary").textContent = `${online} online · ${studio.screens.length} known`;
  if (!studio.screens.length) {
    const empty = document.createElement("div");
    empty.className = "screen-empty";
    empty.textContent = "No device has connected yet. A screen will appear here automatically after its first WebSocket connection.";
    grid.append(empty);
    return;
  }
  for (const screen of studio.screens) {
    const card = document.createElement("article");
    card.className = "fleet-card";
    const head = document.createElement("div");
    head.className = "fleet-card-head";
    const dot = document.createElement("span");
    dot.className = `status-dot${screen.connected ? " connected" : ""}`;
    const title = document.createElement("div");
    title.className = "fleet-card-title";
    const strong = document.createElement("strong");
    strong.textContent = screen.name;
    const id = document.createElement("span");
    id.textContent = screen.id;
    title.append(strong, id);
    const state = document.createElement("span");
    state.className = `connection-label${screen.connected ? " online" : ""}`;
    state.textContent = screen.connected ? "Online" : relativeLastSeen(screen.lastSeen);
    head.append(dot, title, state);

    const stream = screen.stream || {};
    const heartbeat = screen.heartbeat || {};
    const activeDesign = studio.designs.find((design) => design.id === screen.activeDesignId);
    const metrics = document.createElement("div");
    metrics.className = "metric-grid";
    for (const [value, label] of [
      [screen.connected ? `${Number(stream.framesPerSecond || 0).toFixed(1)} fps` : "—", "Frame rate"],
      [screen.connected ? `${compactNumber(stream.pixelsPerSecond || 0)} px/s` : "—", "Pixel updates"],
      [screen.connected ? `${Number(stream.kilobitsPerSecond || 0).toFixed(1)} kb/s` : "—", "Bandwidth"],
      [screen.connected ? `${Number(stream.zoneMessagesPerSecond || 0).toFixed(1)}/s` : "—", "Zones"],
      [screen.connected ? `${Number(stream.lastPushMs || 0).toFixed(0)} ms` : "—", "Last push"],
      [screen.connected ? relativeLastSeen(stream.connectedSince).replace("Seen ", "") : relativeLastSeen(screen.lastSeen), "Connection"],
      [screen.connected ? wifiSignalLabel(heartbeat.wifiRssiDbm) : "—", "WiFi signal"],
      [screen.connected && heartbeat.lastPingAt ? relativeLastSeen(heartbeat.lastPingAt).replace("Seen ", "") : "Waiting…", "10s heartbeat"],
      [screen.connected && heartbeat.uptimeMs ? `${Math.floor(heartbeat.uptimeMs / 60000)} min` : "—", "Device uptime"],
      [activeDesign?.name || "—", "Active screen"],
    ]) {
      const metric = document.createElement("div");
      metric.className = "metric";
      const metricValue = document.createElement("strong");
      metricValue.textContent = value;
      const metricLabel = document.createElement("span");
      metricLabel.textContent = label;
      metric.append(metricValue, metricLabel);
      metrics.append(metric);
    }

    const form = document.createElement("form");
    form.className = "fleet-form";
    const nameLabel = document.createElement("label");
    nameLabel.textContent = "Name";
    const nameInput = document.createElement("input");
    nameInput.value = screen.name;
    nameInput.maxLength = 100;
    const designLabel = document.createElement("label");
    designLabel.textContent = "Home design";
    const designSelect = document.createElement("select");
    for (const design of studio.designs) {
      const option = document.createElement("option");
      option.value = design.id;
      option.textContent = design.name;
      designSelect.append(option);
    }
    designSelect.value = screen.designId;
    const actions = document.createElement("div");
    actions.className = "fleet-actions";
    const save = document.createElement("button");
    save.type = "submit";
    save.className = "secondary-button";
    save.textContent = "Save device";
    actions.append(save);
    if (screen.connected) {
      const restart = document.createElement("button");
      restart.type = "button";
      restart.className = "danger-button";
      restart.textContent = "Restart device";
      restart.onclick = async () => {
        const confirmed = await appConfirm({
          title: `Restart ${screen.name}?`,
          message: "The display will briefly go offline and reconnect automatically.",
          confirmLabel: "Restart device", danger: true,
        });
        if (!confirmed) return;
        restart.disabled = true;
        try {
          const response = await fetch(`/api/screens/${encodeURIComponent(screen.id)}/restart`, { method: "POST" });
          const result = await response.json();
          if (!response.ok) throw new Error(result.error || "Restart request failed");
          toast(`Restart requested for ${screen.name}`);
        } catch (error) {
          restart.disabled = false;
          toast(error.message, true);
        }
      };
      actions.prepend(restart);
    }
    form.append(nameLabel, nameInput, designLabel, designSelect, actions);
    form.onsubmit = async (event) => {
      event.preventDefault();
      save.disabled = true;
      try {
        const response = await fetch(`/api/screens/${encodeURIComponent(screen.id)}`, {
          method: "PUT", headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: nameInput.value, designId: designSelect.value }),
        });
        const result = await response.json();
        if (!response.ok) throw new Error(result.error || "Could not update device");
        screen.name = result.screen.name;
        screen.designId = result.screen.designId;
        screen.activeDesignId = result.screen.activeDesignId || result.screen.designId;
        renderStudioControls();
        renderDevicePage();
        toast(`${screen.name} updated`);
      } catch (error) {
        save.disabled = false;
        toast(error.message, true);
      }
    };
    card.append(head, metrics, form);
    grid.append(card);
  }
}

function renderDesignPage() {
  const grid = $("design-grid");
  grid.replaceChildren();
  for (const design of studio.designs) {
    const card = document.createElement("article");
    card.className = "design-card";
    const previewWrap = document.createElement("div");
    previewWrap.className = "design-preview-wrap";
    const preview = document.createElement("img");
    preview.className = "design-preview";
    if (design.orientation === "landscape") preview.classList.add("landscape");
    preview.alt = `${design.name} rendered preview`;
    preview.src = `/api/designs/${encodeURIComponent(design.id)}/preview?revision=${design.revision}`;
    previewWrap.append(preview);
    const body = document.createElement("div");
    body.className = "design-card-body";
    const name = document.createElement("h2");
    name.textContent = design.name;
    const revision = document.createElement("span");
    revision.className = "design-revision";
    revision.textContent = `Revision ${design.revision}`;
    const assignments = document.createElement("div");
    assignments.className = "assignment-list";
    if (!studio.screens.length) {
      const empty = document.createElement("p");
      empty.className = "design-empty-assignments";
      empty.textContent = "No devices discovered yet.";
      assignments.append(empty);
    } else {
      for (const screen of studio.screens) {
        const label = document.createElement("label");
        const checkbox = document.createElement("input");
        checkbox.type = "checkbox";
        checkbox.value = screen.id;
        checkbox.checked = screen.designId === design.id;
        const copy = document.createElement("span");
        copy.textContent = `${screen.connected ? "●" : "○"} ${screen.name}`;
        label.append(checkbox, copy);
        assignments.append(label);
      }
    }
    const actions = document.createElement("div");
    actions.className = "design-actions";
    const edit = document.createElement("button");
    edit.className = "secondary-button";
    edit.textContent = "Edit design";
    edit.onclick = async () => {
      try {
        await loadDesign(design.id);
        showPage("designer", true);
      } catch (error) { toast(error.message, true); }
    };
    const assign = document.createElement("button");
    assign.className = "primary-button";
    assign.textContent = "Assign selected";
    assign.disabled = !studio.screens.length;
    assign.onclick = async () => {
      const screenIds = [...assignments.querySelectorAll('input[type="checkbox"]:checked')].map((input) => input.value);
      if (!screenIds.length) {
        toast("Select at least one device", true);
        return;
      }
      assign.disabled = true;
      try {
        const response = await fetch(`/api/designs/${encodeURIComponent(design.id)}/assign`, {
          method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ screenIds }),
        });
        const result = await response.json();
        if (!response.ok) throw new Error(result.error || "Could not assign design");
        await refreshStudio();
        toast(`Assigned ${design.name} to ${result.assigned} device${result.assigned === 1 ? "" : "s"}`);
      } catch (error) {
        assign.disabled = false;
        toast(error.message, true);
      }
    };
    actions.append(edit, assign);
    body.append(name, revision, assignments, actions);
    card.append(previewWrap, body);
    grid.append(card);
  }
}

async function loadHomeAssistantSettings() {
  const response = await fetch("/api/settings/home-assistant");
  if (!response.ok) throw new Error("Could not load Home Assistant settings");
  homeAssistantSettings = await response.json();
  $("ha-url").value = homeAssistantSettings.url || "";
  $("ha-token").value = "";
  $("ha-token").placeholder = homeAssistantSettings.tokenConfigured ? "Saved — leave blank to keep it" : "Paste a long-lived access token";
  const badge = $("ha-settings-state");
  badge.className = `integration-badge${homeAssistantSettings.configured ? " connected" : ""}`;
  badge.textContent = homeAssistantSettings.configured ? "Configured" : "Not configured";
  $("ha-settings-result").textContent = homeAssistantSettings.tokenConfigured ? "A private token is stored." : "";
}

async function saveHomeAssistantSettings(clear = false) {
  const save = $("ha-save");
  save.disabled = true;
  $("ha-settings-result").textContent = clear ? "Disconnecting…" : "Testing connection…";
  try {
    const response = await fetch("/api/settings/home-assistant", {
      method: "PUT", headers: { "Content-Type": "application/json" },
      body: JSON.stringify(clear ? { url: "", clearToken: true } : { url: $("ha-url").value, token: $("ha-token").value }),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || "Could not save Home Assistant settings");
    await loadHomeAssistantSettings();
    homeAssistant = await (await fetch("/api/integrations/home-assistant")).json();
    renderHomeAssistantOptions();
    $("ha-settings-result").textContent = clear ? "Home Assistant disconnected." : result.connected ? `Connected · ${result.entityCount} entities found.` : `Saved, but the connection failed (${result.connectionError || "unknown error"}).`;
    toast(clear ? "Home Assistant disconnected" : "Home Assistant settings saved", !clear && !result.connected);
  } catch (error) {
    $("ha-settings-result").textContent = error.message;
    toast(error.message, true);
  } finally {
    save.disabled = false;
  }
}

function renderScreenList() {
  const list = $("screen-list");
  list.replaceChildren();
  const online = studio.screens.filter((screen) => screen.connected).length;
  $("screen-summary").textContent = `${online}/${studio.screens.length} online`;
  if (!studio.screens.length) {
    const empty = document.createElement("div");
    empty.className = "screen-empty";
    empty.textContent = "No device has connected yet. New screens appear here automatically.";
    list.append(empty);
    return;
  }
  for (const screen of studio.screens) {
    const design = studio.designs.find((item) => item.id === screen.designId);
    const activeDesign = studio.designs.find((item) => item.id === screen.activeDesignId);
    const card = document.createElement("button");
    card.type = "button";
    card.className = `screen-card${screen.id === selectedScreenId ? " selected" : ""}`;
    card.dataset.screenId = screen.id;
    card.title = `${screen.id}${screen.remote ? ` · ${screen.remote}` : ""}`;

    const dot = document.createElement("span");
    dot.className = `status-dot${screen.connected ? " connected" : ""}`;
    const copy = document.createElement("span");
    copy.className = "screen-card-copy";
    const name = document.createElement("span");
    name.className = "screen-card-name";
    name.textContent = screen.name;
    const assignment = document.createElement("span");
    assignment.className = "screen-card-design";
    assignment.textContent = activeDesign && activeDesign.id !== design?.id
      ? `${activeDesign.name} · home ${design?.name || "unassigned"}`
      : design?.name || "Unassigned design";
    copy.append(name, assignment);
    if (screen.stream) {
      const metrics = document.createElement("span");
      metrics.className = "screen-card-metrics";
      metrics.textContent = `${screen.stream.framesPerSecond.toFixed(1)} fps · ${compactNumber(screen.stream.pixelsPerSecond)} px/s · ${screen.stream.kilobitsPerSecond.toFixed(1)} kb/s`;
      copy.append(metrics);
      card.title += ` · ${screen.stream.zoneMessagesPerSecond.toFixed(1)} zones/s · last push ${screen.stream.lastPushMs.toFixed(1)} ms`;
    }
    const state = document.createElement("span");
    state.className = `screen-card-state${screen.connected ? " online" : ""}`;
    state.textContent = screen.connected ? "Online" : relativeLastSeen(screen.lastSeen);
    card.append(dot, copy, state);
    card.onclick = () => selectScreen(screen.id);
    list.append(card);
  }
}

function selectScreen(screenId) {
  selectedScreenId = screenId;
  renderStudioControls();
  const screen = studio.screens.find((item) => item.id === selectedScreenId);
  if (screen) loadDesign(screen.designId).catch((error) => toast(error.message, true));
}

async function loadDesign(designId, { discardConfirmed = false } = {}) {
  if (designId === currentDesignId && layout) return;
  if ((dirty || codeEditorDirty) && !discardConfirmed) {
    const discard = await appConfirm({
      title: "Discard unsaved changes?",
      message: codeEditorDirty ? "Unapplied JSON and other unsaved design changes will be lost." : "Unsaved design changes will be lost when you switch screens.",
      confirmLabel: "Discard and switch", danger: true,
    });
    if (!discard) {
      renderStudioControls();
      return false;
    }
  }
  codeEditorDirty = false;
  const response = await fetch(`/api/layout?design=${encodeURIComponent(designId)}`);
  const loaded = await response.json();
  if (!response.ok) throw new Error(loaded.error || "Could not load design");
  currentDesignId = designId;
  layout = loaded.layout;
  appliedRevision = loaded.revision;
  selectedId = null;
  dirty = false;
  undoStack = [];
  redoStack = [];
  if (canvasMode === "code") syncCodeEditor(true);
  renderStudioControls();
  renderAll();
  await refreshPreview();
  return true;
}

async function refreshStudio() {
  const response = await fetch("/api/studio");
  if (!response.ok) throw new Error("Could not load screens and designs");
  studio = await response.json();
  renderStudioControls();
  if (currentPage === "devices") renderDevicePage();
  if (currentPage === "designs") renderDesignPage();
  return studio;
}

async function assignDesign() {
  if (!selectedScreenId) return;
  const response = await fetch(`/api/screens/${encodeURIComponent(selectedScreenId)}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ designId: currentDesignId }),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || "Assignment failed");
  const screen = studio.screens.find((item) => item.id === selectedScreenId);
  if (screen) screen.designId = currentDesignId;
  renderStudioControls();
  toast(`${result.screen.name} now uses this design`);
}

async function createDesign() {
  const name = await appPrompt({
    title: "Create a screen design",
    message: "The new design starts as a copy of the design currently open.",
    inputLabel: "Design name", inputValue: "New design", inputPlaceholder: "Kitchen controls",
    confirmLabel: "Create design",
  });
  if (!name?.trim()) return;
  let discardConfirmed = false;
  if (dirty || codeEditorDirty) {
    discardConfirmed = await appConfirm({
      title: "Create from the saved design?",
      message: "The new design will copy the last saved revision. Your current unsaved changes will be discarded when it opens.",
      confirmLabel: "Create and switch", danger: true,
    });
    if (!discardConfirmed) return;
  }
  const response = await fetch("/api/designs", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: name.trim(), cloneFrom: currentDesignId }),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || "Could not create design");
  await refreshStudio();
  await loadDesign(result.design.id, { discardConfirmed });
  toast(`Created ${result.design.name}`);
}

async function updateStatus() {
  try {
    const [status] = await Promise.all([(await fetch("/api/status")).json(), refreshStudio()]);
    $("device-dot").classList.toggle("connected", status.deviceConnected);
    $("device-status").textContent = status.deviceConnected ? `${status.connectedScreens} screen${status.connectedScreens === 1 ? "" : "s"} connected` : "Screens offline";
  } catch {
    $("device-dot").classList.remove("connected");
    $("device-status").textContent = "Server unavailable";
  }
}

function applyZoom() {
  const zoom = ZOOM_LEVELS[zoomIndex];
  $("stage").style.transform = `scale(${zoom})`;
  const width = displayWidth();
  const height = displayHeight();
  $("stage").style.width = `${width}px`;
  $("stage").style.height = `${height}px`;
  $("preview").width = width;
  $("preview").height = height;
  $("preview").style.width = `${width}px`;
  $("preview").style.height = `${height}px`;
  $("device-shell").style.width = `${width * zoom}px`;
  $("device-shell").style.height = `${height * zoom}px`;
  $("zoom-label").textContent = `${Math.round(zoom * 100)}%`;
  $("zoom-out").disabled = canvasMode === "code" || zoomIndex === 0;
  $("zoom-in").disabled = canvasMode === "code" || zoomIndex === ZOOM_LEVELS.length - 1;
}

function handleKeyboard(event) {
  if (activeModal) {
    if (event.key === "Escape") {
      event.preventDefault();
      closeAppModal(null);
    }
    return;
  }
  if (document.activeElement === $("layout-json")) return;
  const editing = ["INPUT", "TEXTAREA", "SELECT"].includes(document.activeElement?.tagName);
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z") {
    event.preventDefault();
    event.shiftKey ? redo() : undo();
    return;
  }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "y") {
    event.preventDefault(); redo(); return;
  }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "d" && !editing) {
    event.preventDefault(); duplicateSelected(); return;
  }
  if (editing || !selected()) return;
  if (["Delete", "Backspace"].includes(event.key)) {
    event.preventDefault(); deleteSelected(); return;
  }
  const vectors = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
  if (vectors[event.key] && !selected().locked) {
    event.preventDefault();
    const [dx, dy] = vectors[event.key];
    const amount = event.shiftKey ? 10 : 1;
    commit(() => {
      const element = selected();
      element.frame.x = Math.max(0, Math.min(displayWidth() - element.frame.w, element.frame.x + dx * amount));
      element.frame.y = Math.max(0, Math.min(displayHeight() - element.frame.h, element.frame.y + dy * amount));
    });
  }
}

function bindEvents() {
  document.querySelectorAll("[data-route]").forEach((link) => {
    link.onclick = (event) => {
      event.preventDefault();
      showPage(link.dataset.route, true);
    };
  });
  window.onpopstate = () => showPage(pageFromPath());
  document.querySelectorAll("[data-add-type]").forEach((button) => {
    button.onclick = () => addElement(button.dataset.addType);
  });
  $("canvas-overlay").onclick = (event) => {
    if (event.target === $("canvas-overlay")) {
      selectedId = null;
      renderAll();
    }
  };
  $("canvas-overlay").oncontextmenu = (event) => {
    if (event.target === $("canvas-overlay") && selected()) showContextMenu(event, selected());
  };
  $("live-touch-overlay").onpointerdown = sendLiveTouch;
  $("edit-mode").onclick = () => setCanvasMode("edit").catch((error) => toast(error.message, true));
  $("code-mode").onclick = () => setCanvasMode("code").catch((error) => toast(error.message, true));
  $("live-mode").onclick = () => setCanvasMode("live").catch((error) => toast(error.message, true));
  $("layout-json").oninput = () => {
    codeEditorDirty = true;
    showCodeErrors();
    setCodeEditorState("Unapplied JSON changes", "pending");
    setDocumentState();
  };
  $("layout-json").onkeydown = (event) => {
    if (event.key === "Tab") {
      event.preventDefault();
      const input = event.currentTarget;
      const start = input.selectionStart;
      input.setRangeText("  ", start, input.selectionEnd, "end");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    } else if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
      event.preventDefault();
      applyCodeLayout();
    }
  };
  $("format-json").onclick = formatCodeEditor;
  $("apply-json").onclick = () => applyCodeLayout();
  $("orientation").onchange = (event) => setOrientation(event.target.value);
  $("canvas-menu").querySelectorAll("[data-context-action]").forEach((button) => {
    button.onclick = () => {
      const action = button.dataset.contextAction;
      hideContextMenu();
      if (action === "delete") deleteSelected();
      else alignSelected(action);
    };
  });
  document.addEventListener("pointerdown", (event) => {
    if (!event.target.closest("#canvas-menu")) hideContextMenu();
  });
  window.addEventListener("blur", hideContextMenu);
  window.addEventListener("resize", hideContextMenu);
  $("screen-name").onchange = (event) => commit(() => { layout.name = event.target.value; });
  $("background").onchange = (event) => commit(() => { layout.background = event.target.value; });
  $("undo").onclick = undo;
  $("redo").onclick = redo;
  $("save").onclick = saveLayout;
  $("screen-select").onchange = (event) => {
    selectScreen(event.target.value);
  };
  $("design-select").onchange = (event) => loadDesign(event.target.value).catch((error) => toast(error.message, true));
  $("assign-design").onclick = () => assignDesign().catch((error) => toast(error.message, true));
  $("new-design").onclick = () => createDesign().catch((error) => toast(error.message, true));
  $("designs-new").onclick = () => createDesign().then(renderDesignPage).catch((error) => toast(error.message, true));
  $("ha-settings-form").onsubmit = (event) => {
    event.preventDefault();
    saveHomeAssistantSettings(false);
  };
  $("ha-clear").onclick = async () => {
    const confirmed = await appConfirm({
      title: "Disconnect Home Assistant?",
      message: "The saved access token will be removed. Home Assistant blocks will show as unavailable until it is configured again.",
      confirmLabel: "Disconnect", danger: true,
    });
    if (confirmed) saveHomeAssistantSettings(true);
  };
  $("duplicate").onclick = duplicateSelected;
  $("zoom-out").onclick = () => { zoomIndex = Math.max(0, zoomIndex - 1); applyZoom(); };
  $("zoom-in").onclick = () => { zoomIndex = Math.min(ZOOM_LEVELS.length - 1, zoomIndex + 1); applyZoom(); };
  $("app-modal-cancel").onclick = () => closeAppModal(null);
  $("app-modal-backdrop").onclick = (event) => {
    if (event.target === $("app-modal-backdrop")) closeAppModal(null);
  };
  $("app-modal-input").oninput = () => $("app-modal-input").classList.remove("invalid");
  $("app-modal-form").onsubmit = (event) => {
    event.preventDefault();
    if (!activeModal) return;
    if (activeModal.hasInput) {
      const value = $("app-modal-input").value.trim();
      if (!value) {
        $("app-modal-input").classList.add("invalid");
        $("app-modal-input").focus();
        return;
      }
      closeAppModal(value);
    } else {
      closeAppModal(true);
    }
  };
  document.addEventListener("keydown", handleKeyboard);
}

async function initialize() {
  try {
    const [studioResponse, catalogResponse, homeAssistantResponse] = await Promise.all([
      fetch("/api/studio"), fetch("/api/catalog"), fetch("/api/integrations/home-assistant"),
    ]);
    studio = await studioResponse.json();
    catalog = await catalogResponse.json();
    homeAssistant = await homeAssistantResponse.json();
    renderHomeAssistantOptions();
    const initialScreen = studio.screens.find((item) => item.connected) || studio.screens[0];
    selectedScreenId = initialScreen?.id || "";
    currentDesignId = initialScreen?.designId || studio.designs[0]?.id || "default";
    const loaded = await (await fetch(`/api/layout?design=${encodeURIComponent(currentDesignId)}`)).json();
    layout = loaded.layout;
    appliedRevision = loaded.revision;
    bindEvents();
    applyZoom();
    renderStudioControls();
    renderAll();
    showPage(pageFromPath());
    await Promise.all([refreshPreview(), updateStatus()]);
    setInterval(updateStatus, 3000);
    setInterval(() => {
      if (layout?.elements.some((element) => element.type === "clock" && element.visible)) refreshPreview();
    }, 1000);
  } catch (error) {
    setDocumentState("Could not load project");
    toast(error.message, true);
  }
}

initialize();
