// Flow (connections) view: every page reachable from the current page
// through Screen buttons, laid out as cards with arrows. A node is a
// (design, page) pair — pages of one design cluster as separate cards.

import { $ } from "./util.js";
import {
  S, activePageIdFor, setActivePage, docFor, pageOf, pageNameOf,
  effectiveElements, resolveLinkNode, nodeKey, designNameOf, homePageIdOf,
} from "./state.js";
import { appConfirm, toast } from "./dialog.js";
import { fetchDesignLayout } from "./api.js";
import { renderAll, renderElementOverlay, showContextMenu } from "./editor.js";
import { renderFlowPreviews, schedulePreview, applyZoom } from "./canvas.js";

export function switchEditingNode(designId, pageId) {
  if (!S.designLayouts.has(designId)) return;
  S.editingDesignId = designId;
  S.layout = S.designLayouts.get(designId);
  if (pageId) setActivePage(designId, pageId);
  S.selectedId = null;
  S.dirty = S.flowDirty.has(designId);
  renderAll();
  document.dispatchEvent(new CustomEvent("yello:show-canvas"));
  if (S.canvasMode === "edit" && S.flowMode) {
    $("preview-status").textContent = `Flow view · editing ${designNameOf(designId)} · ${pageNameOf(S.layout, activePageIdFor(designId))}`;
  }
  schedulePreview();
}

export function collectFlowOrder() {
  const root = { designId: S.currentDesignId, pageId: activePageIdFor(S.currentDesignId) };
  const order = [root];
  const seen = new Set([nodeKey(root.designId, root.pageId)]);
  for (let index = 0; index < order.length; index++) {
    const node = order[index];
    const doc = docFor(node.designId);
    if (!doc) continue;
    for (const element of effectiveElements(doc, node.pageId)) {
      if (element.type !== "design-link" || element.visible === false) continue;
      const target = element.props?.targetDesignId;
      if (!target) continue;
      const resolved = resolveLinkNode(node.designId, target);
      if (!resolved) continue;
      const key = nodeKey(resolved.designId, resolved.pageId);
      if (!seen.has(key)) {
        seen.add(key);
        order.push(resolved);
      }
    }
  }
  return order;
}

export function ensureFlowSoon(delay = 250) {
  clearTimeout(S.flowRebuildTimer);
  S.flowRebuildTimer = setTimeout(() => {
    rebuildFlow().catch((error) => toast(error.message, true));
  }, delay);
}

export async function rebuildFlow() {
  S.flowOrder = collectFlowOrder();
  const pendingDesigns = () =>
    [...new Set(S.flowOrder.map((node) => node.designId))]
      .filter((id) => !S.designLayouts.has(id) && !S.flowMissing.has(id));
  let pending = pendingDesigns();
  while (pending.length) {
    await Promise.all(pending.map((id) => fetchDesignLayout(id)));
    const next = collectFlowOrder();
    const grew = next.length > S.flowOrder.length
      || next.some((node, index) => nodeKey(node.designId, node.pageId) !== nodeKey(S.flowOrder[index]?.designId, S.flowOrder[index]?.pageId));
    S.flowOrder = next;
    pending = pendingDesigns();
    if (!pending.length && !grew) break;
  }
  S.flowOrder = collectFlowOrder();
  syncFlowCards();
  applyZoom();
  renderAll();
  await renderFlowPreviews();
}

export async function setShowConnections(enabled) {
  if (enabled === S.flowMode) return;
  if (enabled) {
    S.flowMode = true;
    S.flowMissing = new Set();
    $("show-connections").classList.add("active");
    $("show-connections").setAttribute("aria-pressed", "true");
    if (S.dirty) S.flowDirty.add(S.currentDesignId);
    if (S.canvasMode === "edit") {
      $("device-shell").classList.add("hidden");
      $("canvas-overlay").classList.add("hidden");
      $("flow-canvas").classList.remove("hidden");
      $("preview-status").textContent = `Flow view · editing ${designNameOf(S.editingDesignId ?? S.currentDesignId)}`;
    }
    await rebuildFlow();
    return;
  }
  const others = [...S.flowDirty].filter((id) => id !== S.currentDesignId);
  if (others.length) {
    const discard = await appConfirm({
      title: "Discard unsaved changes in other screens?",
      message: `${others.length} other screen${others.length === 1 ? "" : "s"} in this flow have unsaved changes. Save first, or discard them.`,
      confirmLabel: "Discard changes", danger: true,
    });
    if (!discard) {
      $("show-connections").classList.add("active");
      $("show-connections").setAttribute("aria-pressed", "true");
      return;
    }
    await Promise.all(others.map((id) => fetchDesignLayout(id, { force: true })));
  }
  S.flowMode = false;
  S.flowStructureKey = "";
  S.editingDesignId = S.currentDesignId;
  S.layout = S.designLayouts.get(S.currentDesignId) || S.layout;
  S.selectedId = null;
  S.dirty = S.flowDirty.has(S.currentDesignId);
  $("show-connections").classList.remove("active");
  $("show-connections").setAttribute("aria-pressed", "false");
  $("flow-canvas").classList.add("hidden");
  $("device-shell").classList.remove("hidden");
  $("canvas-overlay").classList.remove("hidden");
  renderAll();
  schedulePreview(0);
}

function nodeCardSelector(designId, pageId) {
  return `.flow-screen[data-design-id="${CSS.escape(designId)}"][data-page-id="${CSS.escape(pageId)}"]`;
}

export function flowCardElement(designId, pageId) {
  return $("flow-canvas")?.querySelector(nodeCardSelector(designId, pageId)) || null;
}

function buildFlowCard(node, isRoot) {
  const { designId, pageId } = node;
  const card = document.createElement("div");
  card.className = "flow-screen";
  card.dataset.designId = designId;
  card.dataset.pageId = pageId;
  const header = document.createElement("header");
  header.onclick = () => switchEditingNode(designId, pageId);
  if (isRoot) {
    const badge = document.createElement("span");
    badge.className = "flow-badge";
    badge.textContent = "Root";
    header.append(badge);
  }
  const name = document.createElement("strong");
  const doc = docFor(designId);
  name.textContent = doc ? `${designNameOf(designId)} · ${pageNameOf(doc, pageId)}` : designId;
  header.append(name);
  const dirtyDot = document.createElement("span");
  dirtyDot.className = "flow-dirty-dot";
  dirtyDot.textContent = "●";
  header.append(dirtyDot);
  card.append(header);

  if (!doc) {
    const missing = document.createElement("div");
    missing.className = "flow-missing";
    missing.textContent = `Design “${designId}” not found on the server`;
    card.append(missing);
    return card;
  }
  if (!pageOf(doc, pageId)) {
    const missing = document.createElement("div");
    missing.className = "flow-missing";
    missing.textContent = `Page “${pageId}” not found in design “${designNameOf(designId)}”`;
    card.append(missing);
    return card;
  }
  const shell = document.createElement("div");
  shell.className = "device-shell";
  const stage = document.createElement("div");
  stage.className = "stage";
  const img = document.createElement("img");
  img.id = `screen-preview-${designId}-${pageId}`;
  img.className = "screen-preview";
  img.alt = `${name.textContent} rendered preview`;
  img.draggable = false;
  const cached = S.screenPreviewUrls.get(nodeKey(designId, pageId));
  if (cached) img.src = cached;
  const overlay = document.createElement("div");
  overlay.className = "canvas-overlay";
  overlay.dataset.designId = designId;
  overlay.dataset.pageId = pageId;
  overlay.onclick = (event) => {
    if (event.target === overlay) {
      S.selectedId = null;
      renderAll();
    }
  };
  overlay.oncontextmenu = (event) => {
    if (event.target === overlay) {
      const element = effectiveElements(doc, pageId).find((item) => item.id === S.selectedId);
      if (element) showContextMenu(event, element, designId, pageId);
    }
  };
  stage.append(img, overlay);
  shell.append(stage);
  card.append(shell);
  return card;
}

export function syncFlowCards() {
  const canvas = $("flow-canvas");
  const key = `${S.currentDesignId}|${S.flowOrder.map((node) => nodeKey(node.designId, node.pageId)).join("|")}`;
  if (key !== S.flowStructureKey) {
    S.flowStructureKey = key;
    canvas.querySelectorAll(".flow-screen").forEach((card) => card.remove());
    S.flowOrder.forEach((node, index) => canvas.append(buildFlowCard(node, index === 0)));
  }
  for (const card of canvas.querySelectorAll(".flow-screen")) {
    const { designId, pageId } = card.dataset;
    card.classList.toggle("editing", designId === S.editingDesignId && pageId === activePageIdFor(designId));
    card.querySelector(".flow-dirty-dot")?.classList.toggle("visible", S.flowDirty.has(designId));
    const name = card.querySelector("header strong");
    const doc = docFor(designId);
    if (name && doc) name.textContent = `${designNameOf(designId)} · ${pageNameOf(doc, pageId)}`;
  }
}

export function renderFlow() {
  syncFlowCards();
  for (const card of $("flow-canvas").querySelectorAll(".flow-screen")) {
    const { designId, pageId } = card.dataset;
    const overlay = card.querySelector(".canvas-overlay");
    if (overlay && S.designLayouts.has(designId)) renderElementOverlay(designId, pageId, overlay);
  }
  drawFlowArrows();
}

export function drawFlowArrows() {
  const canvas = $("flow-canvas");
  const svg = $("flow-connectors");
  if (!canvas || !svg || canvas.classList.contains("hidden")) return;
  const base = canvas.getBoundingClientRect();
  svg.innerHTML = "<defs>"
    + '<marker id="flow-arrow-head" markerWidth="7" markerHeight="7" refX="6" refY="3.5" orient="auto-start-reverse"><path d="M0,0 L7,3.5 L0,7 Z" fill="#6b9cff"></path></marker>'
    + '<marker id="flow-arrow-head-missing" markerWidth="7" markerHeight="7" refX="6" refY="3.5"><path d="M0,0 L7,3.5 L0,7 Z" fill="#ff6577"></path></marker>'
    + "</defs>";
  for (const fromNode of S.flowOrder) {
    const doc = docFor(fromNode.designId);
    if (!doc) continue;
    for (const element of effectiveElements(doc, fromNode.pageId)) {
      if (element.type !== "design-link" || element.visible === false) continue;
      const target = element.props?.targetDesignId;
      if (!target) continue;
      const resolved = resolveLinkNode(fromNode.designId, target);
      if (!resolved) continue; // __back has no static destination
      if (resolved.designId === fromNode.designId && resolved.pageId === fromNode.pageId) continue;
      const card = flowCardElement(fromNode.designId, fromNode.pageId);
      const box = card?.querySelector(`.element-box[data-element-id="${CSS.escape(element.id)}"]`);
      if (!card || !box) continue;
      const start = box.getBoundingClientRect();
      const targetCard = flowCardElement(resolved.designId, resolved.pageId);
      const targetDoc = docFor(resolved.designId);
      const missing = !targetCard || !targetDoc || !pageOf(targetDoc, resolved.pageId) || S.flowMissing.has(resolved.designId);
      let end;
      if (targetCard) {
        const shell = targetCard.querySelector(".device-shell") || targetCard.querySelector(".flow-missing");
        if (!shell) continue;
        const rect = shell.getBoundingClientRect();
        end = { x: rect.left + rect.width / 2, y: rect.top };
      } else {
        end = { x: start.left + start.width / 2, y: start.bottom + 40 };
      }
      const sx = start.left + start.width / 2 - base.left;
      const sy = start.bottom - base.top;
      const tx = end.x - base.left;
      const ty = end.y - base.top;
      const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
      path.setAttribute("d", `M ${sx} ${sy} C ${sx} ${sy + 34}, ${tx} ${ty - 34}, ${tx} ${ty}`);
      path.setAttribute("fill", "none");
      path.setAttribute("stroke", missing ? "#ff6577" : "#6b9cff");
      path.setAttribute("stroke-width", "1.6");
      path.setAttribute("opacity", "0.85");
      if (missing) path.setAttribute("stroke-dasharray", "4 3");
      path.setAttribute("marker-end", missing ? "url(#flow-arrow-head-missing)" : "url(#flow-arrow-head)");
      svg.append(path);
      const dot = document.createElementNS("http://www.w3.org/2000/svg", "circle");
      dot.setAttribute("cx", `${sx}`);
      dot.setAttribute("cy", `${sy}`);
      dot.setAttribute("r", "2.6");
      dot.setAttribute("fill", missing ? "#ff6577" : "#6b9cff");
      svg.append(dot);
    }
  }
}
