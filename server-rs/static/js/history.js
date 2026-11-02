// Undo/redo history and the shared commit() mutation path.
// Every document edit goes through commit(): snapshot, mutate, mark
// dirty, re-render, re-preview.

import { $, clone } from "./util.js";
import { S, findElement } from "./state.js";
import { renderAll } from "./editor.js";
import { schedulePreview } from "./canvas.js";
import { ensureFlowSoon } from "./flow.js";

export function pushHistory(designId = S.editingDesignId ?? S.currentDesignId) {
  S.undoStack.push({ designId, layout: clone(S.designLayouts.get(designId) || S.layout) });
  if (S.undoStack.length > 80) S.undoStack.shift();
  S.redoStack = [];
}

export function commit(mutator, designId = S.editingDesignId) {
  pushHistory(designId);
  mutator();
  S.dirty = true;
  if (S.flowMode && designId) S.flowDirty.add(designId);
  renderAll();
  schedulePreview();
  if (S.flowMode) ensureFlowSoon();
}

export function undo() {
  while (S.undoStack.length) {
    const entry = S.undoStack.pop();
    if (!S.flowMode && entry.designId !== S.currentDesignId) continue;
    S.redoStack.push({ designId: S.editingDesignId ?? S.currentDesignId, layout: clone(S.layout) });
    S.editingDesignId = entry.designId;
    S.layout = entry.layout;
    S.designLayouts.set(entry.designId, S.layout);
    if (!findElement(S.selectedId)) S.selectedId = null;
    S.dirty = true;
    if (S.flowMode) S.flowDirty.add(entry.designId);
    renderAll();
    schedulePreview();
    if (S.flowMode) ensureFlowSoon();
    return;
  }
}

export function redo() {
  if (!S.redoStack.length) return;
  const entry = S.redoStack.pop();
  if (!S.flowMode && entry.designId !== S.currentDesignId) return redo();
  S.undoStack.push({ designId: S.editingDesignId ?? S.currentDesignId, layout: clone(S.layout) });
  S.editingDesignId = entry.designId;
  S.layout = entry.layout;
  S.designLayouts.set(entry.designId, S.layout);
  if (!findElement(S.selectedId)) S.selectedId = null;
  S.dirty = true;
  if (S.flowMode) S.flowDirty.add(entry.designId);
  renderAll();
  schedulePreview();
  if (S.flowMode) ensureFlowSoon();
}

export function setDocumentState(message = "") {
  const editingId = S.flowMode ? S.editingDesignId : S.currentDesignId;
  const designName = S.studio.designs.find((item) => item.id === editingId)?.name || S.layout?.name || "Design";
  const revision = S.flowMode ? S.designRevisions.get(editingId) ?? S.appliedRevision : S.appliedRevision;
  const unsavedScreens = S.flowMode ? S.flowDirty.size : (S.dirty || S.codeEditorDirty ? 1 : 0);
  const changed = unsavedScreens > 0 || S.codeEditorDirty;
  const stateText = changed
    ? S.codeEditorDirty ? "JSON changes not applied" : unsavedScreens > 1 ? `${unsavedScreens} screens unsaved` : "Unsaved changes"
    : `Revision ${revision}`;
  $("document-state").textContent = message || `${designName} · ${stateText}`;
  $("save").disabled = !changed;
  $("undo").disabled = S.undoStack.length === 0;
  $("redo").disabled = S.redoStack.length === 0;
}
