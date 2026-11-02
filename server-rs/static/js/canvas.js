// Canvas modes (edit / code / live), server previews, zoom, the JSON
// code editor, and live-mode touch navigation.

import { $, MIN_ZOOM, MAX_ZOOM, clampZoom } from "./util.js";
import {
  S, designDims, activePageIdFor, docFor,
  pageOf, homePageIdOf, parseLinkTarget, resolveLinkNode, nodeKey,
} from "./state.js";
import { toast, appConfirm } from "./dialog.js";
import { pushHistory, setDocumentState } from "./history.js";
import { fetchDesignLayout } from "./api.js";
import { renderAll, hideContextMenu } from "./editor.js";
import { drawFlowArrows } from "./flow.js";

/** The (design, page) the main canvas currently shows. */
export function viewTarget() {
  if (S.liveMode && S.liveNav) return S.liveNav;
  const designId = S.editingDesignId ?? S.currentDesignId;
  return { designId, pageId: activePageIdFor(designId) };
}

// ---- Previews ----

export function schedulePreview(delay = 100) {
  clearTimeout(S.previewTimer);
  S.previewTimer = setTimeout(refreshPreview, delay);
}

export async function refreshPreview() {
  if (S.flowMode && S.canvasMode === "edit") {
    const designId = S.editingDesignId ?? S.currentDesignId;
    await refreshScreenPreview(designId, activePageIdFor(designId));
    return;
  }
  const target = viewTarget();
  const doc = docFor(target.designId);
  if (!doc) return;
  S.previewController?.abort();
  S.previewController = new AbortController();
  $("preview-status").textContent = "Rendering preview…";
  $("preview-status").classList.add("rendering");
  try {
    const params = new URLSearchParams({ design: target.designId, page: target.pageId });
    if (S.liveMode) params.set("live", "1");
    const response = await fetch(`/api/preview?${params}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(doc),
      signal: S.previewController.signal,
    });
    if (!response.ok) {
      const result = await response.json();
      throw new Error((result.details || result.errors)?.[0]?.message || `Preview failed (${response.status})`);
    }
    const blob = await response.blob();
    const nextUrl = URL.createObjectURL(blob);
    $("preview").src = nextUrl;
    if (S.previewUrl) URL.revokeObjectURL(S.previewUrl);
    S.previewUrl = nextUrl;
    $("preview-status").textContent = S.canvasMode === "code"
      ? "Direct JSON configuration · validate before applying"
      : S.liveMode ? "Interactive live render · click to touch" : "Authoritative server preview";
  } catch (error) {
    if (error.name !== "AbortError") $("preview-status").textContent = error.message;
  } finally {
    $("preview-status").classList.remove("rendering");
  }
}

export async function refreshScreenPreview(designId, pageId) {
  const doc = S.designLayouts.get(designId);
  if (!doc || S.flowMissing.has(designId)) return;
  const img = document.getElementById(`screen-preview-${designId}-${pageId}`);
  if (!img) return;
  try {
    const params = new URLSearchParams({ design: designId, page: pageId });
    const response = await fetch(`/api/preview?${params}`, {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(doc),
    });
    if (!response.ok) return;
    const blob = await response.blob();
    const url = URL.createObjectURL(blob);
    img.src = url;
    const key = nodeKey(designId, pageId);
    const previous = S.screenPreviewUrls.get(key);
    S.screenPreviewUrls.set(key, url);
    if (previous && previous !== url) URL.revokeObjectURL(previous);
  } catch {
    // Preview failures are non-fatal; the next edit retries.
  }
}

export function scheduleScreenPreview(designId, pageId, delay = 120) {
  const key = nodeKey(designId, pageId);
  clearTimeout(S.screenPreviewTimers.get(key));
  S.screenPreviewTimers.set(key, setTimeout(() => refreshScreenPreview(designId, pageId), delay));
}

export async function renderFlowPreviews() {
  await Promise.all(
    S.flowOrder
      .filter((node) => S.designLayouts.has(node.designId))
      .map((node) => refreshScreenPreview(node.designId, node.pageId)),
  );
}

// ---- Zoom ----

export function setZoom(z) {
  S.zoom = clampZoom(z);
  applyZoom();
}

export function stepZoom(factor) {
  setZoom(S.zoom * factor);
}

// Pan / pinch / double-tap on the canvas background (anywhere outside the
// screen itself) moves the view instead of editing: one pointer drags to
// pan, two pointers pinch to zoom, double-tap toggles 100% / 200%. The
// page itself never zooms (viewport is locked), so these gestures belong
// to the canvas. Mouse works too: drag to pan, Ctrl+wheel to zoom.
export function initCanvasGestures() {
  const scroller = $("canvas-scroll");
  if (!scroller || scroller.dataset.gesturesBound) return;
  scroller.dataset.gesturesBound = "1";
  const pointers = new Map();
  let startScroll = null;
  let startDist = 0;
  let startZoom = 1;
  let startMid = null;
  let moved = false;
  let lastTap = 0;
  let suppressClickUntil = 0;

  const onBackground = (event) => !event.target.closest("#stage");
  const midpoint = () => {
    const [a, b] = [...pointers.values()];
    return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
  };
  const distance = () => {
    const [a, b] = [...pointers.values()];
    return Math.hypot(a.x - b.x, a.y - b.y);
  };

  scroller.addEventListener("pointerdown", (event) => {
    if (!onBackground(event)) return;
    try { scroller.setPointerCapture(event.pointerId); } catch { /* synthetic or already-released pointer */ }
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    moved = false;
    if (pointers.size === 1) {
      startScroll = { left: scroller.scrollLeft, top: scroller.scrollTop, x: event.clientX, y: event.clientY };
    } else if (pointers.size === 2) {
      startDist = distance();
      startZoom = S.zoom;
      startMid = midpoint();
      startScroll = { left: scroller.scrollLeft, top: scroller.scrollTop };
    }
  });
  scroller.addEventListener("pointermove", (event) => {
    if (!pointers.has(event.pointerId)) return;
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pointers.size === 1 && startScroll) {
      const dx = event.clientX - startScroll.x;
      const dy = event.clientY - startScroll.y;
      if (Math.abs(dx) + Math.abs(dy) > 6) moved = true;
      if (moved) {
        scroller.scrollLeft = startScroll.left - dx;
        scroller.scrollTop = startScroll.top - dy;
      }
    } else if (pointers.size === 2 && startDist > 0) {
      moved = true;
      const mid = midpoint();
      scroller.scrollLeft = startScroll.left - (mid.x - startMid.x);
      scroller.scrollTop = startScroll.top - (mid.y - startMid.y);
      setZoom(startZoom * (distance() / startDist));
    }
  });
  const endPointer = (event) => {
    if (!pointers.has(event.pointerId)) return;
    pointers.delete(event.pointerId);
    if (moved) suppressClickUntil = Date.now() + 350;
    if (pointers.size === 0) {
      if (!moved && event.type === "pointerup") {
        const now = Date.now();
        if (now - lastTap < 320) {
          setZoom(S.zoom > 1.05 ? 1 : 2);
          lastTap = 0;
        } else {
          lastTap = now;
        }
      }
      startScroll = null;
      startDist = 0;
    }
  };
  scroller.addEventListener("pointerup", endPointer);
  scroller.addEventListener("pointercancel", endPointer);
  scroller.addEventListener("click", (event) => {
    if (Date.now() < suppressClickUntil) {
      event.stopPropagation();
      event.preventDefault();
    }
  }, true);
  scroller.addEventListener("wheel", (event) => {
    if (!event.ctrlKey && !event.metaKey) return;
    event.preventDefault();
    stepZoom(event.deltaY < 0 ? 1.1 : 1 / 1.1);
  }, { passive: false });
}

export function applyZoom() {
  const zoom = S.zoom;
  if (S.flowMode && S.canvasMode === "edit") {
    for (const card of document.querySelectorAll("#flow-canvas .flow-screen")) {
      const designId = card.dataset.designId;
      const stage = card.querySelector(".stage");
      if (!stage) continue;
      const [width, height] = designDims(designId);
      const img = card.querySelector(".screen-preview");
      const shell = card.querySelector(".device-shell");
      stage.style.transform = `scale(${zoom})`;
      stage.style.width = `${width}px`;
      stage.style.height = `${height}px`;
      if (img) {
        img.width = width;
        img.height = height;
        img.style.width = `${width}px`;
        img.style.height = `${height}px`;
      }
      shell.style.width = `${width * zoom}px`;
      shell.style.height = `${height * zoom}px`;
    }
    $("zoom-label").textContent = `${Math.round(zoom * 100)}%`;
    $("zoom-out").disabled = S.zoom <= MIN_ZOOM;
    $("zoom-in").disabled = S.zoom >= MAX_ZOOM;
    drawFlowArrows();
    return;
  }
  const target = viewTarget();
  const [width, height] = designDims(target.designId);
  $("stage").style.transform = `scale(${zoom})`;
  $("stage").style.width = `${width}px`;
  $("stage").style.height = `${height}px`;
  $("preview").width = width;
  $("preview").height = height;
  $("preview").style.width = `${width}px`;
  $("preview").style.height = `${height}px`;
  $("device-shell").style.width = `${width * zoom}px`;
  $("device-shell").style.height = `${height * zoom}px`;
  $("zoom-label").textContent = `${Math.round(zoom * 100)}%`;
  $("zoom-out").disabled = S.canvasMode === "code" || S.zoom <= MIN_ZOOM;
  $("zoom-in").disabled = S.canvasMode === "code" || S.zoom >= MAX_ZOOM;
}

// ---- Code editor ----

export function setCodeEditorState(message, state = "") {
  const label = $("code-editor-state");
  label.textContent = message;
  label.className = state;
}

export function showCodeErrors(errors = []) {
  const container = $("code-errors");
  if (!errors.length) {
    container.classList.add("hidden");
    container.textContent = "";
    return;
  }
  container.textContent = errors.map((error) => `${error.path || "$"}: ${error.message || error}`).join("\n");
  container.classList.remove("hidden");
}

export function syncCodeEditor(force = false) {
  if (!S.layout || (S.codeEditorDirty && !force)) return;
  $("layout-json").value = JSON.stringify(S.layout, null, 2);
  S.codeEditorDirty = false;
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

export async function applyCodeLayout() {
  const parsed = parseCodeEditor();
  if (parsed.errors.length) {
    showCodeErrors(parsed.errors);
    setCodeEditorState("JSON syntax error", "invalid");
    return false;
  }
  $("apply-json").disabled = true;
  setCodeEditorState("Validating against the device layout schema…", "pending");
  try {
    const response = await fetch(`/api/layout/validate?design=${encodeURIComponent(S.currentDesignId)}`, {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(parsed.value),
    });
    const result = await response.json();
    if (!response.ok) {
      showCodeErrors(result.details || result.errors || [{ path: "$", message: result.error || "Layout validation failed" }]);
      setCodeEditorState("Layout validation failed", "invalid");
      return false;
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
    S.codeEditorDirty = false;
    $("layout-json").value = JSON.stringify(S.layout, null, 2);
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

export function formatCodeEditor() {
  const parsed = parseCodeEditor();
  if (parsed.errors.length) {
    showCodeErrors(parsed.errors);
    setCodeEditorState("Fix the JSON syntax before formatting", "invalid");
    return;
  }
  $("layout-json").value = JSON.stringify(parsed.value, null, 2);
  S.codeEditorDirty = true;
  showCodeErrors();
  setCodeEditorState("Formatted · apply JSON to update the design", "pending");
  setDocumentState();
}

// ---- Canvas modes + live navigation ----

export async function setCanvasMode(mode) {
  if (!["edit", "code", "live"].includes(mode) || mode === S.canvasMode) return true;
  if (S.canvasMode === "code" && S.codeEditorDirty) {
    const discard = await appConfirm({
      title: "Discard unapplied JSON?",
      message: "Your JSON editor changes have not been applied to the design.",
      confirmLabel: "Discard changes", danger: true,
    });
    if (!discard) return false;
    S.codeEditorDirty = false;
  }
  S.canvasMode = mode;
  S.liveMode = mode === "live";
  const flowVisible = S.flowMode && mode === "edit";
  $("edit-mode").classList.toggle("active", mode === "edit");
  $("code-mode").classList.toggle("active", mode === "code");
  $("live-mode").classList.toggle("active", mode === "live");
  $("canvas-scroll").classList.toggle("hidden", mode === "code");
  $("code-editor-pane").classList.toggle("hidden", mode !== "code");
  $("flow-canvas").classList.toggle("hidden", !flowVisible);
  $("device-shell").classList.toggle("hidden", flowVisible);
  $("canvas-overlay").classList.toggle("hidden", mode !== "edit");
  $("live-touch-overlay").classList.toggle("hidden", mode !== "live");
  $("zoom-out").disabled = mode === "code" || S.zoom <= MIN_ZOOM;
  $("zoom-in").disabled = mode === "code" || S.zoom >= MAX_ZOOM;
  if (mode === "live") {
    if (S.flowMode && S.editingDesignId !== S.currentDesignId) {
      S.editingDesignId = S.currentDesignId;
      S.layout = S.designLayouts.get(S.currentDesignId) || S.layout;
      S.selectedId = null;
      S.dirty = S.flowDirty.has(S.currentDesignId);
      renderAll();
    }
    S.liveNav = {
      designId: S.currentDesignId,
      pageId: activePageIdFor(S.currentDesignId),
      homeDesignId: S.currentDesignId,
      stack: [],
    };
  }
  if (mode === "code") syncCodeEditor(true);
  if (flowVisible) applyZoom();
  $("preview-status").textContent = mode === "live"
    ? "Interactive live render · click to touch"
    : mode === "code"
      ? "Direct JSON configuration · validate before applying"
      : S.flowMode ? `Flow view · editing ${S.studio.designs.find((item) => item.id === S.editingDesignId)?.name || "screen"}` : "Authoritative server preview";
  hideContextMenu();
  setDocumentState();
  if (mode !== "code") schedulePreview(0);
  return true;
}

/** Mirror of the server-side navigation, for the live preview only. */
async function followLiveNavigation(target) {
  const nav = S.liveNav;
  if (!nav) return;
  const parsed = parseLinkTarget(target);
  if (parsed.kind === "back") {
    const previous = nav.stack.pop();
    if (previous) {
      nav.designId = previous.designId;
      nav.pageId = previous.pageId;
    } else {
      const doc = docFor(nav.designId);
      nav.pageId = doc ? homePageIdOf(doc) : "main";
    }
  } else if (parsed.kind === "home") {
    nav.stack = [];
    nav.designId = nav.homeDesignId;
    const doc = docFor(nav.designId);
    nav.pageId = doc ? homePageIdOf(doc) : "main";
  } else {
    const resolved = resolveLinkNode(nav.designId, target);
    if (!resolved) return;
    if (resolved.designId !== nav.designId || resolved.pageId !== nav.pageId) {
      nav.stack.push({ designId: nav.designId, pageId: nav.pageId });
    }
    nav.designId = resolved.designId;
    if (!docFor(nav.designId)) await fetchDesignLayout(nav.designId);
    const doc = docFor(nav.designId);
    nav.pageId = doc && pageOf(doc, resolved.pageId) ? resolved.pageId : doc ? homePageIdOf(doc) : "main";
  }
  applyZoom();
  refreshPreview();
}

export async function sendLiveTouch(event) {
  if (!S.liveNav || !S.liveMode) return;
  event.preventDefault();
  const nav = S.liveNav;
  const [width, height] = designDims(nav.designId);
  const bounds = $("live-touch-overlay").getBoundingClientRect();
  const x = Math.max(0, Math.min(width - 1, Math.floor((event.clientX - bounds.left) * width / bounds.width)));
  const y = Math.max(0, Math.min(height - 1, Math.floor((event.clientY - bounds.top) * height / bounds.height)));
  const doc = docFor(nav.designId);
  if (!doc) return;
  try {
    const response = await fetch(`/api/live-touch?design=${encodeURIComponent(nav.designId)}`, {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ layout: doc, pageId: nav.pageId, x, y, touchType: "down" }),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || "Live touch failed");
    if (result.navigateTo) {
      await followLiveNavigation(result.navigateTo);
      return;
    }
    refreshPreview();
    setTimeout(refreshPreview, 120);
    setTimeout(refreshPreview, 400);
  } catch (error) {
    toast(error.message, true);
  }
}
