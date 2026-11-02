// Canvas editor: element operations, page strip, layers, overlays,
// pointer interactions, inspector, and the renderAll() hub.

import { $, clone, TYPE_META, icon, isNarrowScreen } from "./util.js";
import {
  S, displayWidth, displayHeight, designDims, designNameOf,
  activePage, activePageIdFor, setActivePage, homePageIdOf, pageOf,
  effectiveElements, isSharedElement, findElement, allDesignElementIds,
  linkTargetOptions, nodeKey,
} from "./state.js";
import { toast, appConfirm, appPrompt, appForm } from "./dialog.js";
import { commit, pushHistory, setDocumentState } from "./history.js";
import { schedulePreview, scheduleScreenPreview, applyZoom, syncCodeEditor } from "./canvas.js";
import { ensureFlowSoon, renderFlow, drawFlowArrows } from "./flow.js";
import { appendImageTools } from "./assets.js";

export function selected() {
  return findElement(S.selectedId);
}

export function uniqueId(prefix) {
  let index = Date.now().toString(36);
  let candidate = `${prefix}${index}`;
  const allIds = allDesignElementIds(S.layout);
  while (allIds.has(candidate)) candidate = `${prefix}${index}_${Math.floor(Math.random() * 1000)}`;
  return candidate;
}

function uniquePageId() {
  const existing = new Set((S.layout.pages || []).map((page) => page.id));
  let index = Date.now().toString(36);
  let candidate = `page${index}`;
  while (existing.has(candidate)) candidate = `page${index}_${Math.floor(Math.random() * 1000)}`;
  return candidate;
}

/** The array that owns an element: the shared pool or the active page. */
function ownerArrayOf(element) {
  if (isSharedElement(S.layout, element)) return S.layout.sharedElements;
  return activePage()?.elements || [];
}

function makeElement(type) {
  const template = S.catalog?.elementDefaults?.[type];
  if (!template) return null;
  const element = clone(template);
  element.id = uniqueId("element");
  element.name = TYPE_META[type].label;
  return element;
}

export function addElement(type) {
  const element = makeElement(type);
  if (!element) {
    toast("This block is not available from the running server yet. Refresh after the server restarts.", true);
    return;
  }
  commit(() => {
    if (type === "external-text") {
      const sourceId = uniqueId("source");
      S.layout.dataSources.push({
        id: sourceId, name: "External text source", type: "http-text",
        url: "https://wttr.in/?format=3", interval: 300,
      });
      element.props.sourceId = sourceId;
    }
    if (type === "design-link") {
      element.props.targetDesignId = S.studio.designs.find((item) => item.id !== S.currentDesignId)?.id || S.currentDesignId;
    }
    if (S.addAsShared) {
      element.pages = [activePageIdFor(S.editingDesignId ?? S.currentDesignId)];
      S.layout.sharedElements.push(element);
    } else {
      activePage().elements.push(element);
    }
    S.selectedId = element.id;
  });
}

export function duplicateSelected() {
  const element = selected();
  if (!element) return;
  commit(() => {
    const duplicate = clone(element);
    duplicate.id = uniqueId("element");
    duplicate.name = `${element.name} copy`;
    duplicate.frame.x = Math.min(displayWidth() - duplicate.frame.w, Math.max(0, duplicate.frame.x + 8));
    duplicate.frame.y = Math.min(displayHeight() - duplicate.frame.h, Math.max(0, duplicate.frame.y + 8));
    ownerArrayOf(element).push(duplicate);
    S.selectedId = duplicate.id;
  });
}

export function deleteSelected() {
  const element = selected();
  if (!element) return;
  commit(() => {
    const owner = ownerArrayOf(element);
    const index = owner.findIndex((item) => item.id === element.id);
    if (index >= 0) owner.splice(index, 1);
    S.selectedId = null;
  });
}

export function setOrientation(orientation) {
  if (!S.layout || orientation === (S.layout.orientation || "portrait")) return;
  const oldWidth = displayWidth();
  const oldHeight = displayHeight();
  const newWidth = orientation === "landscape" ? 320 : 240;
  const newHeight = orientation === "landscape" ? 240 : 320;
  const scaleElement = (element) => {
    element.frame.x = Math.max(0, Math.round(element.frame.x * newWidth / oldWidth));
    element.frame.y = Math.max(0, Math.round(element.frame.y * newHeight / oldHeight));
    element.frame.w = Math.max(1, Math.min(newWidth, Math.round(element.frame.w * newWidth / oldWidth)));
    element.frame.h = Math.max(1, Math.min(newHeight, Math.round(element.frame.h * newHeight / oldHeight)));
    element.frame.x = Math.min(newWidth - element.frame.w, element.frame.x);
    element.frame.y = Math.min(newHeight - element.frame.h, element.frame.y);
  };
  commit(() => {
    S.layout.orientation = orientation;
    for (const page of S.layout.pages || []) for (const element of page.elements || []) scaleElement(element);
    for (const element of S.layout.sharedElements || []) scaleElement(element);
  });
}

export function alignSelected(action) {
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

export function hideContextMenu() {
  $("canvas-menu").classList.add("hidden");
}

export function showContextMenu(event, element, designId = S.currentDesignId, pageId = null) {
  event.preventDefault();
  event.stopPropagation();
  if (S.flowMode && designId !== S.editingDesignId) {
    S.editingDesignId = designId;
    S.layout = S.designLayouts.get(designId) || S.layout;
    S.dirty = S.flowDirty.has(designId);
  }
  if (pageId) setActivePage(designId, pageId);
  S.selectedId = element.id;
  renderAll();
  const menu = $("canvas-menu");
  menu.classList.remove("hidden");
  const left = Math.min(event.clientX, window.innerWidth - 188);
  const top = Math.min(event.clientY, window.innerHeight - 285);
  menu.style.left = `${Math.max(6, left)}px`;
  menu.style.top = `${Math.max(6, top)}px`;
}

export function moveLayer(direction) {
  const element = selected();
  if (!element) return;
  const owner = ownerArrayOf(element);
  const index = owner.findIndex((item) => item.id === element.id);
  const target = index + direction;
  if (index < 0 || target < 0 || target >= owner.length) return;
  commit(() => {
    [owner[index], owner[target]] = [owner[target], owner[index]];
  });
}

// ---- Document + page fields ----

export function renderDocumentFields() {
  if (!S.layout) return;
  const page = activePage();
  $("screen-name").value = S.layout.name;
  $("page-name").value = page?.name || "";
  $("background").value = page?.background || S.layout.background;
  $("background-value").textContent = page?.background || S.layout.background;
  $("orientation").value = S.layout.orientation || "portrait";
  $("idle-home").value = S.layout.idleHomeSeconds || 0;
  $("canvas-dimensions").textContent = `${displayWidth()} × ${displayHeight()}`;
  applyZoom();
}

// ---- Page strip ----

export function switchPage(pageId) {
  const designId = S.editingDesignId ?? S.currentDesignId;
  if (pageId === activePageIdFor(designId)) return;
  setActivePage(designId, pageId);
  S.selectedId = null;
  renderAll();
  schedulePreview(0);
}

export async function addPage() {
  if (!S.layout) return;
  const form = await appForm({
    title: "Add a page",
    message: "The new page starts blank with the design background. Links are optional — nothing is added unless you tick it.",
    inputLabel: "Page name", inputValue: "New page", inputPlaceholder: "Alarms",
    confirmLabel: "Add page",
    options: [
      { id: "link", label: "Add a Screen button to this page on the current page", checked: true },
      { id: "back", label: "Add a Back button on the new page", checked: true },
    ],
  });
  if (!form) return;
  const name = form.value.trim() || "New page";
  const fromPage = activePage();
  const pageId = uniquePageId();
  commit(() => {
    const page = { id: pageId, name, background: S.layout.background || "#0a0a1e", elements: [] };
    S.layout.pages.push(page);
    if (form.checks.link && fromPage) {
      const link = makeElement("design-link");
      if (link) {
        link.name = `Open ${name}`;
        link.props.label = name;
        link.props.targetDesignId = `page:${pageId}`;
        link.frame = { x: 16, y: 264, w: 132, h: 40 };
        fromPage.elements.push(link);
      }
    }
    if (form.checks.back) {
      const back = makeElement("design-link");
      if (back) {
        back.name = "Back";
        back.props.label = "← Back";
        back.props.targetDesignId = "__back";
        back.frame = { x: 8, y: 8, w: 92, h: 36 };
        page.elements.push(back);
      }
    }
    setActivePage(S.editingDesignId ?? S.currentDesignId, pageId);
    S.selectedId = null;
  });
}

export async function renamePage() {
  const page = activePage();
  if (!page) return;
  const name = await appPrompt({
    title: "Rename page",
    message: "Page names show in the page strip, the flow view, and link menus.",
    inputLabel: "Page name", inputValue: page.name, confirmLabel: "Rename",
  });
  if (!name?.trim() || name.trim() === page.name) return;
  commit(() => { page.name = name.trim(); });
}

export function duplicatePage() {
  const page = activePage();
  if (!page) return;
  const pageId = uniquePageId();
  commit(() => {
    const copy = clone(page);
    copy.id = pageId;
    copy.name = `${page.name} copy`;
    for (const element of copy.elements || []) element.id = uniqueId("element");
    const index = S.layout.pages.findIndex((item) => item.id === page.id);
    S.layout.pages.splice(index + 1, 0, copy);
    setActivePage(S.editingDesignId ?? S.currentDesignId, pageId);
    S.selectedId = null;
  });
  toast(`Duplicated page as “${page.name} copy”`);
}

export async function deletePage() {
  const page = activePage();
  if (!page || S.layout.pages.length <= 1) return;
  const confirmed = await appConfirm({
    title: `Delete page “${page.name}”?`,
    message: "Blocks on this page are deleted. Screen buttons in this design that pointed here will be retargeted to Home, and shared blocks will no longer include this page.",
    confirmLabel: "Delete page", danger: true,
  });
  if (!confirmed) return;
  commit(() => {
    S.layout.pages = S.layout.pages.filter((item) => item.id !== page.id);
    if (S.layout.homePageId === page.id) S.layout.homePageId = S.layout.pages[0].id;
    for (const shared of S.layout.sharedElements || []) {
      if (Array.isArray(shared.pages)) shared.pages = shared.pages.filter((id) => id !== page.id);
    }
    for (const other of S.layout.pages) {
      for (const element of other.elements || []) {
        if (element.type === "design-link" && element.props?.targetDesignId === `page:${page.id}`) {
          element.props.targetDesignId = "__home";
        }
      }
    }
    setActivePage(S.editingDesignId ?? S.currentDesignId, homePageIdOf(S.layout));
    S.selectedId = null;
  });
}

export function setHomePage() {
  const page = activePage();
  if (!page || S.layout.homePageId === page.id) return;
  commit(() => { S.layout.homePageId = page.id; });
  toast(`“${page.name}” is now the home page`);
}

export function renderPageStrip() {
  const strip = $("page-strip");
  if (!strip) return;
  strip.replaceChildren();
  if (!S.layout?.pages) return;
  const designId = S.editingDesignId ?? S.currentDesignId;
  const current = activePageIdFor(designId);
  for (const page of S.layout.pages) {
    const tab = document.createElement("button");
    tab.type = "button";
    tab.className = `page-tab${page.id === current ? " active" : ""}`;
    if (page.id === S.layout.homePageId) {
      const homeMark = document.createElement("span");
      homeMark.className = "tab-home";
      homeMark.innerHTML = icon("home");
      const nameSpan = document.createElement("span");
      nameSpan.textContent = page.name;
      tab.append(homeMark, nameSpan);
    } else {
      tab.textContent = page.name;
    }
    tab.title = page.id === S.layout.homePageId ? "Home page" : "Switch to this page";
    tab.onclick = () => switchPage(page.id);
    strip.append(tab);
  }
  const actions = document.createElement("div");
  actions.className = "page-actions";
  const add = (iconName, label, title, handler, disabled = false) => {
    const button = document.createElement("button");
    button.type = "button";
    const iconSpan = document.createElement("span");
    iconSpan.className = "pa-icon";
    iconSpan.setAttribute("aria-hidden", "true");
    iconSpan.innerHTML = icon(iconName);
    const labelSpan = document.createElement("span");
    labelSpan.className = "pa-label";
    labelSpan.textContent = label;
    button.append(iconSpan, labelSpan);
    button.title = title;
    button.setAttribute("aria-label", title);
    button.disabled = disabled;
    button.onclick = handler;
    actions.append(button);
  };
  add("plus", "Page", "Add a page to this design", () => addPage());
  add("pencil", "Rename", "Rename the current page", () => renamePage());
  add("copy", "Duplicate", "Duplicate the current page", () => duplicatePage());
  add("home", "Set home", "Make the current page the design's home page", () => setHomePage(), S.layout.homePageId === current);
  add("trash", "Delete", "Delete the current page", () => deletePage(), S.layout.pages.length <= 1);
  strip.append(actions);
}

// ---- Layers ----

function layerRow(element, { dimmed = false } = {}) {
  const row = document.createElement("div");
  row.className = `layer-row${element.id === S.selectedId ? " selected" : ""}${element.visible ? "" : " hidden-layer"}${dimmed ? " not-on-page" : ""}`;
  row.onclick = () => {
    S.selectedId = element.id;
    renderAll();
    // On phones the layers live in a separate panel; jump to the canvas
    // so the selection is visible (main.js only acts on small screens).
    document.dispatchEvent(new CustomEvent("yello:show-canvas"));
  };
  const type = document.createElement("span");
  type.className = "layer-type";
  type.innerHTML = icon(TYPE_META[element.type].icon);
  const name = document.createElement("span");
  name.className = "layer-name";
  name.textContent = element.name;
  const visibility = document.createElement("button");
  visibility.title = element.visible ? "Hide layer" : "Show layer";
  visibility.innerHTML = icon(element.visible ? "eye" : "eye-off");
  visibility.onclick = (event) => {
    event.stopPropagation();
    commit(() => { element.visible = !element.visible; });
  };
  const lock = document.createElement("button");
  lock.title = element.locked ? "Unlock layer" : "Lock layer";
  lock.innerHTML = icon(element.locked ? "lock" : "lock-open");
  lock.onclick = (event) => {
    event.stopPropagation();
    commit(() => { element.locked = !element.locked; });
  };
  row.append(type, name, visibility, lock);
  return row;
}

export function renderLayers() {
  const list = $("layer-list");
  list.replaceChildren();
  if (!S.layout) return;
  const page = activePage();
  const shared = S.layout.sharedElements || [];
  const sharedHere = shared.filter((el) => (el.pages || []).includes(page?.id));
  $("layer-count").textContent =
    `${page?.elements.length || 0} block${page?.elements.length === 1 ? "" : "s"}` +
    (sharedHere.length ? ` + ${sharedHere.length} shared` : "") +
    (S.flowMode ? ` · ${designNameOf(S.editingDesignId ?? S.currentDesignId)}` : "");

  const pageHeading = document.createElement("div");
  pageHeading.className = "layer-group-heading";
  pageHeading.textContent = `Page · ${page?.name || ""}`;
  list.append(pageHeading);
  [...(page?.elements || [])].reverse().forEach((element) => list.append(layerRow(element)));

  if (shared.length) {
    const sharedHeading = document.createElement("div");
    sharedHeading.className = "layer-group-heading";
    sharedHeading.textContent = "Shared blocks (opt-in per page)";
    list.append(sharedHeading);
    [...shared].reverse().forEach((element) => {
      list.append(layerRow(element, { dimmed: !(element.pages || []).includes(page?.id) }));
    });
  }
}

// ---- Canvas overlay ----

export function renderOverlay() {
  const designId = S.editingDesignId ?? S.currentDesignId;
  renderElementOverlay(designId, activePageIdFor(designId), $("canvas-overlay"));
}

export function renderElementOverlay(designId, pageId, overlay) {
  const doc = designId === (S.editingDesignId ?? S.currentDesignId) ? S.layout : S.designLayouts.get(designId);
  const elements = doc ? effectiveElements(doc, pageId) : [];
  overlay.replaceChildren();
  elements.forEach((element) => {
    if (!element.visible) return;
    const frame = element.frame;
    const box = document.createElement("div");
    const isSelected = element.id === S.selectedId && designId === S.editingDesignId && pageId === activePageIdFor(designId);
    const isLink = element.type === "design-link";
    const shared = isSharedElement(doc, element);
    box.className = `element-box${isSelected ? " selected" : ""}${element.locked ? " locked" : ""}${S.flowMode && isLink ? " link-box" : ""}${shared ? " shared-box" : ""}`;
    box.dataset.elementId = element.id;
    if (shared) box.title = "Shared block (edit once, appears on its chosen pages)";
    Object.assign(box.style, {
      left: `${frame.x}px`, top: `${frame.y}px`, width: `${frame.w}px`, height: `${frame.h}px`,
    });
    box.onpointerdown = (event) => beginPointerInteraction(event, element, null, designId, pageId);
    box.oncontextmenu = (event) => showContextMenu(event, element, designId, pageId);
    if (isSelected && !element.locked) {
      for (const handleName of ["nw", "ne", "sw", "se"]) {
        const handle = document.createElement("span");
        handle.className = "resize-handle";
        handle.dataset.handle = handleName;
        handle.onpointerdown = (event) => beginPointerInteraction(event, element, handleName, designId, pageId);
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

export function beginPointerInteraction(event, element, handle, designId = S.currentDesignId, pageId = null) {
  event.preventDefault();
  event.stopPropagation();
  const [dWidth, dHeight] = designDims(designId);
  if (S.editingDesignId !== designId) {
    S.editingDesignId = designId;
    S.layout = S.designLayouts.get(designId) || S.layout;
    S.dirty = S.flowDirty.has(designId);
    document.querySelectorAll("#flow-canvas .flow-screen").forEach((card) => {
      card.classList.toggle("editing", card.dataset.designId === designId && card.dataset.pageId === (pageId || activePageIdFor(designId)));
    });
  }
  if (pageId) setActivePage(designId, pageId);
  if (S.selectedId !== element.id) {
    S.selectedId = element.id;
    renderLayers();
    renderInspector();
    event.currentTarget.closest(".element-box")?.classList.add("selected");
  }
  if (element.locked) return;
  const zoom = S.zoom;
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
      pushHistory(designId);
      started = true;
      S.dirty = true;
      if (S.flowMode) S.flowDirty.add(designId);
    }
    if (!started) return;
    if (!handle) {
      element.frame.x = Math.max(0, Math.min(dWidth - element.frame.w, origin.x + dx));
      element.frame.y = Math.max(0, Math.min(dHeight - element.frame.h, origin.y + dy));
    } else {
      resizeFrame(element.frame, origin, handle, dx, dy, dWidth, dHeight);
    }
    Object.assign(box.style, {
      left: `${element.frame.x}px`, top: `${element.frame.y}px`,
      width: `${element.frame.w}px`, height: `${element.frame.h}px`,
    });
    const sizeTag = box.querySelector(".size-tag");
    if (sizeTag) sizeTag.textContent = `${element.frame.w} × ${element.frame.h}`;
    setDocumentState();
    scheduleScreenPreview(designId, pageId || activePageIdFor(designId));
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

export function resizeFrame(frame, origin, handle, dx, dy, dWidth = displayWidth(), dHeight = displayHeight()) {
  const minSize = 8;
  let left = origin.x;
  let top = origin.y;
  let right = origin.x + origin.w;
  let bottom = origin.y + origin.h;
  if (handle.includes("w")) left = Math.max(0, Math.min(right - minSize, origin.x + dx));
  if (handle.includes("e")) right = Math.min(dWidth, Math.max(left + minSize, origin.x + origin.w + dx));
  if (handle.includes("n")) top = Math.max(0, Math.min(bottom - minSize, origin.y + dy));
  if (handle.includes("s")) bottom = Math.min(dHeight, Math.max(top + minSize, origin.y + origin.h + dy));
  Object.assign(frame, { x: left, y: top, w: right - left, h: bottom - top });
}

// ---- Inspector ----

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

export function appendField(container, label, value, options, setter) {
  const [labelElement, input] = createInput(label, value, options);
  let historyCaptured = false;
  input.oninput = () => {
    if (!historyCaptured) {
      pushHistory();
      historyCaptured = true;
    }
    const next = input.type === "number" ? Number(input.value) : input.type === "checkbox" ? input.checked : input.value;
    setter(next);
    S.dirty = true;
    if (S.flowMode && S.editingDesignId) S.flowDirty.add(S.editingDesignId);
    if (S.flowMode && S.canvasMode === "edit") {
      const overlay = $("flow-canvas")?.querySelector(
        `.flow-screen[data-design-id="${CSS.escape(S.editingDesignId)}"][data-page-id="${CSS.escape(activePageIdFor(S.editingDesignId))}"] .canvas-overlay`,
      );
      if (overlay) renderElementOverlay(S.editingDesignId, activePageIdFor(S.editingDesignId), overlay);
      drawFlowArrows();
    } else {
      renderOverlay();
    }
    setDocumentState();
    schedulePreview();
    if (S.flowMode) ensureFlowSoon();
  };
  input.onchange = () => renderAll();
  container.append(labelElement, input);
  return input;
}

function appendHomeAssistantEntityField(container, value, setter) {
  const input = appendField(container, "Entity", value, { type: "text" }, setter);
  input.setAttribute("list", "ha-entity-options");
  input.placeholder = S.homeAssistant.configured ? "sensor.temperature" : "Configure Home Assistant in Settings";
  return input;
}

export function renderInspector() {
  const container = $("inspector");
  const element = selected();
  $("duplicate").disabled = !element;
  if (!element) {
    container.className = "inspector-empty";
    container.innerHTML = `<div class="empty-icon">${icon("pointer")}</div><p>Select a block on the canvas or in Layers.</p>`;
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
      container, "Target", element.props.targetDesignId,
      { type: "select", options: linkTargetOptions(S.layout, S.editingDesignId ?? S.currentDesignId) },
      (value) => { element.props.targetDesignId = value; },
    );
    fontFields();
    appendField(container, "Radius", element.style.radius, { type: "number", min: 0, max: 50 }, (value) => { element.style.radius = value; });
    const help = document.createElement("p");
    help.className = "integration-help";
    help.textContent = "Navigates only the device that pressed it — its assigned home design is unchanged. “Back” walks the device's own history; “Home” returns to its home page.";
    container.append(help);
  } else if (element.type === "color-block") {
    colorField();
    appendField(container, "Radius", element.style.radius, { type: "number", min: 0, max: 50 }, (value) => { element.style.radius = value; });
  } else if (element.type === "image") {
    appendField(container, "Image URL", element.props.src, { type: "text" }, (value) => { element.props.src = value; });
    appendField(container, "Fit", element.props.fit, { type: "select", options: [["contain", "Contain"], ["cover", "Cover"], ["stretch", "Stretch"]] }, (value) => { element.props.fit = value; });
    appendImageTools(container, element);
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
    const examples = [["", "Choose an example…"], ...(S.catalog.scriptExamples || []).map((item) => [item.id, item.name])];
    appendField(container, "Example", "", { type: "select", options: examples }, (value) => {
      const example = S.catalog.scriptExamples?.find((item) => item.id === value);
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
  } else if (element.type === "screen") {
    const designOptions = [["", "This design"], ...(S.studio.designs || []).map((design) => [design.id, design.name])];
    appendField(container, "Design", element.props.designId, { type: "select", options: designOptions }, (value) => {
      element.props.designId = value;
      renderInspector();
    });
    const targetDoc = !element.props.designId
      ? S.layout
      : (element.props.designId === (S.editingDesignId ?? S.currentDesignId) ? S.layout : S.designLayouts.get(element.props.designId));
    if (targetDoc?.pages) {
      appendField(
        container, "Page", element.props.pageId,
        { type: "select", options: [["", "Home page"], ...targetDoc.pages.map((page) => [page.id, page.name])] },
        (value) => { element.props.pageId = value; },
      );
    } else {
      appendField(container, "Page ID", element.props.pageId, { type: "text" }, (value) => { element.props.pageId = value; });
    }
    const help = document.createElement("p");
    help.className = "integration-help";
    help.textContent = "Shows another page rendered live inside this block — a screen within a screen. View-only: buttons inside the embedded page don't respond to touches, and embeds render one level deep.";
    container.append(help);
  } else if (element.type === "gauge") {
    appendField(container, "Source", element.props.sourceKind, { type: "select", options: [["static", "Static value"], ["external", "Data source"], ["ha", "Home Assistant"]] }, (value) => {
      element.props.sourceKind = value;
      renderInspector();
    });
    if (element.props.sourceKind === "static") {
      appendField(container, "Value", element.props.value, { type: "number" }, (value) => { element.props.value = value; });
    } else if (element.props.sourceKind === "external") {
      appendField(container, "Data source", element.props.sourceId, { type: "select", options: [["", "Choose a source…"], ...(S.layout.dataSources || []).map((source) => [source.id, source.name])] }, (value) => { element.props.sourceId = value; });
    } else if (element.props.sourceKind === "ha") {
      appendHomeAssistantEntityField(container, element.props.entityId, (value) => { element.props.entityId = value; });
      appendField(container, "Refresh (s)", element.props.refreshInterval, { type: "number", min: 1, max: 3600 }, (value) => { element.props.refreshInterval = value; });
    }
    appendField(container, "Min", element.props.min, { type: "number" }, (value) => { element.props.min = value; });
    appendField(container, "Max", element.props.max, { type: "number" }, (value) => { element.props.max = value; });
    appendField(container, "Decimals", element.props.decimals, { type: "number", min: 0, max: 6 }, (value) => { element.props.decimals = value; });
    appendField(container, "Unit", element.props.unit, { type: "text" }, (value) => { element.props.unit = value; });
    appendField(container, "Show value", element.props.showValue, { type: "checkbox" }, (value) => { element.props.showValue = value; });
    appendField(container, "Fill color", element.style.color, { type: "color" }, (value) => { element.style.color = value; });
    appendField(container, "Track color", element.style.trackColor, { type: "color" }, (value) => { element.style.trackColor = value; });
    appendField(container, "Font size", element.style.fontSize, { type: "number", min: 6, max: 120 }, (value) => { element.style.fontSize = value; });
  } else if (element.type === "qr") {
    appendField(container, "Text / URL", element.props.text, { type: "textarea" }, (value) => { element.props.text = value; });
    appendField(container, "Module color", element.style.color, { type: "color" }, (value) => { element.style.color = value; });
    appendField(container, "Background", element.style.bgColor, { type: "color" }, (value) => { element.style.bgColor = value; });
  } else if (element.type === "shape") {
    appendField(container, "Shape", element.props.shape, { type: "select", options: [["rect", "Rectangle"], ["circle", "Circle"], ["ellipse", "Ellipse"]] }, (value) => { element.props.shape = value; });
    colorField();
    appendField(container, "Radius", element.style.radius, { type: "number", min: 0, max: 50 }, (value) => { element.style.radius = value; });
    appendField(container, "Outline width", element.style.outlineWidth, { type: "number", min: 0, max: 20 }, (value) => { element.style.outlineWidth = value; });
    appendField(container, "Outline color", element.style.outlineColor || "#ffffff", { type: "color" }, (value) => { element.style.outlineColor = value; });
  }

  if ((["ha-state", "ha-toggle", "script"].includes(element.type) || (element.type === "gauge" && element.props.sourceKind === "ha")) && !S.homeAssistant.configured) {
    const help = document.createElement("p");
    help.className = "integration-help";
    help.textContent = "Home Assistant is not configured. Add its URL and token on the Settings page to use ha().";
    container.append(help);
  }

  if (isSharedElement(S.layout, element)) {
    const heading = document.createElement("div");
    heading.className = "layer-group-heading";
    heading.textContent = "Shared block · appears on";
    container.append(heading);
    const help = document.createElement("p");
    help.className = "integration-help";
    help.textContent = "Tick the pages this block appears on. It is never added to a page automatically — unticked pages stay untouched, and edits here update every ticked page.";
    container.append(help);
    for (const page of S.layout.pages || []) {
      const row = document.createElement("label");
      row.className = "shared-page-row";
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = (element.pages || []).includes(page.id);
      checkbox.onchange = () => commit(() => {
        const pages = new Set(element.pages || []);
        if (checkbox.checked) pages.add(page.id);
        else pages.delete(page.id);
        element.pages = [...pages];
      });
      const copy = document.createElement("span");
      copy.textContent = page.id === activePage()?.id ? `${page.name} (this page)` : page.name;
      row.append(checkbox, copy);
      container.append(row);
    }
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

export function renderResourceEditor(element) {
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
    const source = S.layout.dataSources.find((item) => item.id === element.props.sourceId);
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
    const action = S.layout.actions.find((item) => item.id === element.props.actionId);
    if (!action) {
      const create = document.createElement("button");
      create.className = "primary-button wide";
      create.textContent = "Create HTTP action";
      create.onclick = () => commit(() => {
        const actionId = uniqueId("action");
        S.layout.actions.push({ id: actionId, name: "Button action", type: "http", method: "GET", url: "", body: "" });
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

export function updateSelectionBar() {
  const bar = $("selection-bar");
  if (!bar) return;
  const show = isNarrowScreen() && Boolean(S.selectedId) && !$("designer-page").classList.contains("hidden");
  bar.classList.toggle("hidden", !show);
  if (show) {
    const lockButton = $("sel-lock");
    const element = selected();
    if (lockButton && element) {
      const locked = Boolean(element.locked);
      lockButton.setAttribute("aria-pressed", String(locked));
      lockButton.setAttribute("aria-label", locked ? "Unlock element" : "Lock element");
      lockButton.title = locked ? "Unlock" : "Lock";
      lockButton.querySelector("use")?.setAttribute("href", locked ? "#i-lock" : "#i-lock-open");
    }
  }
}

export function renderAll() {
  renderDocumentFields();
  renderPageStrip();
  renderLayers();
  if (S.flowMode && S.canvasMode === "edit") renderFlow();
  else renderOverlay();
  renderInspector();
  if (S.canvasMode === "code" && !S.codeEditorDirty) syncCodeEditor(true);
  setDocumentState();
  updateSelectionBar();
}
