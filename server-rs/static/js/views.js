// Management views: header controls, screens list, Devices page,
// Designs library, Settings, design loading/saving, and routing.

import { $, compactNumber, wifiSignalLabel, relativeLastSeen } from "./util.js";
import { S, activePageIdFor, homePageIdOf } from "./state.js";
import { toast, appConfirm, appPrompt } from "./dialog.js";
import { postLayout, fetchDesignLayout } from "./api.js";
import { renderAll } from "./editor.js";
import {
  schedulePreview, refreshPreview, refreshScreenPreview, applyCodeLayout, syncCodeEditor,
} from "./canvas.js";
import { rebuildFlow } from "./flow.js";
import { setDocumentState } from "./history.js";

export function pageFromPath() {
  const name = window.location.pathname.replace(/^\//, "");
  return ["designer", "devices", "designs", "settings"].includes(name) ? name : "designer";
}

export function showPage(page, push = false) {
  S.currentPage = page;
  document.body.classList.toggle("route-designer", page === "designer");
  for (const name of ["designer", "devices", "designs", "settings"]) {
    $(`${name}-page`).classList.toggle("hidden", name !== page);
  }
  document.querySelectorAll(".designer-actions").forEach((el) => el.classList.toggle("hidden", page !== "designer"));
  document.querySelectorAll("[data-route]").forEach((link) => {
    link.classList.toggle("active", link.dataset.route === page);
  });
  if (push && window.location.pathname !== `/${page}`) history.pushState({ page }, "", `/${page}`);
  if (page === "devices") renderDevicePage();
  if (page === "designs") renderDesignPage();
  if (page === "settings") loadHomeAssistantSettings().catch((error) => toast(error.message, true));
}

export function renderStudioControls() {
  const screenSelect = $("screen-select");
  const designSelect = $("design-select");
  screenSelect.replaceChildren();
  if (!S.studio.screens.length) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "No screens discovered";
    screenSelect.append(option);
    S.selectedScreenId = "";
  } else {
    for (const screen of S.studio.screens) {
      const option = document.createElement("option");
      option.value = screen.id;
      option.textContent = `${screen.connected ? "●" : "○"} ${screen.name}`;
      screenSelect.append(option);
    }
    if (!S.studio.screens.some((item) => item.id === S.selectedScreenId)) {
      S.selectedScreenId = (S.studio.screens.find((item) => item.connected) || S.studio.screens[0]).id;
    }
    screenSelect.value = S.selectedScreenId;
  }
  designSelect.replaceChildren();
  for (const design of S.studio.designs) {
    const option = document.createElement("option");
    option.value = design.id;
    option.textContent = design.name;
    designSelect.append(option);
  }
  designSelect.value = S.currentDesignId;
  $("assign-design").disabled = !S.selectedScreenId;
  renderScreenList();
  renderIntegrationStatus();
}

export function renderIntegrationStatus() {
  const badge = $("ha-integration-status");
  badge.className = "integration-badge";
  if (S.homeAssistant.error) {
    badge.textContent = "HA error";
    badge.classList.add("error");
  } else if (S.homeAssistant.configured) {
    badge.textContent = `HA · ${S.homeAssistant.entities.length} entities`;
    badge.classList.add("connected");
  } else {
    badge.textContent = "HA setup needed";
    badge.title = "Configure Home Assistant on the Settings page";
  }
}

export function renderHomeAssistantOptions() {
  const options = $("ha-entity-options");
  options.replaceChildren();
  for (const entity of S.homeAssistant.entities) {
    const option = document.createElement("option");
    option.value = entity.entityId;
    option.label = `${entity.name} · ${entity.state}${entity.unit ? ` ${entity.unit}` : ""}`;
    options.append(option);
  }
  renderIntegrationStatus();
}

export function renderScreenList() {
  const list = $("screen-list");
  list.replaceChildren();
  const online = S.studio.screens.filter((screen) => screen.connected).length;
  $("screen-summary").textContent = `${online}/${S.studio.screens.length} online`;
  if (!S.studio.screens.length) {
    const empty = document.createElement("div");
    empty.className = "screen-empty";
    empty.textContent = "No device has connected yet. New screens appear here automatically.";
    list.append(empty);
    return;
  }
  for (const screen of S.studio.screens) {
    const design = S.studio.designs.find((item) => item.id === screen.designId);
    const activeDesign = S.studio.designs.find((item) => item.id === screen.activeDesignId);
    const card = document.createElement("button");
    card.type = "button";
    card.className = `screen-card${screen.id === S.selectedScreenId ? " selected" : ""}`;
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

export function selectScreen(screenId) {
  S.selectedScreenId = screenId;
  renderStudioControls();
  const screen = S.studio.screens.find((item) => item.id === S.selectedScreenId);
  if (screen) loadDesign(screen.designId).catch((error) => toast(error.message, true));
}

export function renderDevicePage() {
  const grid = $("device-grid");
  grid.replaceChildren();
  const online = S.studio.screens.filter((screen) => screen.connected).length;
  $("device-page-summary").textContent = `${online} online · ${S.studio.screens.length} known`;
  if (!S.studio.screens.length) {
    const empty = document.createElement("div");
    empty.className = "screen-empty";
    empty.textContent = "No device has connected yet. A screen will appear here automatically after its first WebSocket connection.";
    grid.append(empty);
    return;
  }
  for (const screen of S.studio.screens) {
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
    const activeDesign = S.studio.designs.find((design) => design.id === screen.activeDesignId);
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
    for (const design of S.studio.designs) {
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

export function renderDesignPage() {
  const grid = $("design-grid");
  grid.replaceChildren();
  for (const design of S.studio.designs) {
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
    const pageCount = design.pageCount ?? 1;
    revision.textContent = `Revision ${design.revision} · ${pageCount} page${pageCount === 1 ? "" : "s"}`;
    const assignments = document.createElement("div");
    assignments.className = "assignment-list";
    if (!S.studio.screens.length) {
      const empty = document.createElement("p");
      empty.className = "design-empty-assignments";
      empty.textContent = "No devices discovered yet.";
      assignments.append(empty);
    } else {
      for (const screen of S.studio.screens) {
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
    assign.disabled = !S.studio.screens.length;
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

export async function loadHomeAssistantSettings() {
  const response = await fetch("/api/settings/home-assistant");
  if (!response.ok) throw new Error("Could not load Home Assistant settings");
  S.homeAssistantSettings = await response.json();
  $("ha-url").value = S.homeAssistantSettings.url || "";
  $("ha-token").value = "";
  $("ha-token").placeholder = S.homeAssistantSettings.tokenConfigured ? "Saved — leave blank to keep it" : "Paste a long-lived access token";
  const badge = $("ha-settings-state");
  badge.className = `integration-badge${S.homeAssistantSettings.configured ? " connected" : ""}`;
  badge.textContent = S.homeAssistantSettings.configured ? "Configured" : "Not configured";
  $("ha-settings-result").textContent = S.homeAssistantSettings.tokenConfigured ? "A private token is stored." : "";
}

export async function saveHomeAssistantSettings(clear = false) {
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
    S.homeAssistant = await (await fetch("/api/integrations/home-assistant")).json();
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

export async function loadDesign(designId, { discardConfirmed = false } = {}) {
  if (designId === S.currentDesignId && S.layout) {
    if (S.flowMode) S.editingDesignId = designId;
    return true;
  }
  const unsaved = S.dirty || S.codeEditorDirty || (S.flowMode && S.flowDirty.size > 0);
  if (unsaved && !discardConfirmed) {
    const discard = await appConfirm({
      title: "Discard unsaved changes?",
      message: S.flowMode && S.flowDirty.size > 1
        ? `${S.flowDirty.size} screens in this flow have unsaved changes. They will be lost when you switch screens.`
        : S.codeEditorDirty ? "Unapplied JSON and other unsaved design changes will be lost." : "Unsaved design changes will be lost when you switch screens.",
      confirmLabel: "Discard and switch", danger: true,
    });
    if (!discard) {
      renderStudioControls();
      return false;
    }
  }
  S.codeEditorDirty = false;
  S.flowDirty.delete(designId);
  for (const id of S.flowDirty) await fetchDesignLayout(id, { force: true });
  S.flowDirty.clear();
  const response = await fetch(`/api/layout?design=${encodeURIComponent(designId)}`);
  const loaded = await response.json();
  if (!response.ok) throw new Error(loaded.error || "Could not load design");
  S.currentDesignId = designId;
  S.editingDesignId = designId;
  S.layout = loaded.layout;
  S.appliedRevision = loaded.revision;
  S.designLayouts.set(designId, S.layout);
  S.designRevisions.set(designId, loaded.revision);
  if (!S.activePages.has(designId)) S.activePages.set(designId, homePageIdOf(S.layout));
  S.selectedId = null;
  S.dirty = false;
  S.undoStack = [];
  S.redoStack = [];
  if (S.flowMode) {
    S.flowStructureKey = "";
    await rebuildFlow();
  } else {
    if (S.canvasMode === "code") syncCodeEditor(true);
    renderStudioControls();
    renderAll();
    await refreshPreview();
  }
  return true;
}

export async function refreshStudio() {
  const response = await fetch("/api/studio");
  if (!response.ok) throw new Error("Could not load screens and designs");
  S.studio = await response.json();
  renderStudioControls();
  if (S.currentPage === "devices") renderDevicePage();
  if (S.currentPage === "designs") renderDesignPage();
  return S.studio;
}

export async function saveLayout() {
  if (S.codeEditorDirty && !await applyCodeLayout()) {
    setDocumentState();
    return;
  }
  const targets = S.flowMode
    ? [...new Set([...S.flowDirty, ...(S.dirty && S.editingDesignId ? [S.editingDesignId] : [])])]
    : [S.currentDesignId];
  if (!targets.length) {
    toast("Nothing to save");
    return;
  }
  $("save").disabled = true;
  setDocumentState(targets.length === 1 ? "Saving and applying…" : `Saving ${targets.length} designs…`);
  let appliedScreens = 0;
  try {
    for (const designId of targets) {
      const doc = S.designLayouts.get(designId);
      if (!designId || !doc) continue;
      const { response, result } = await postLayout(designId, doc);
      if (!response.ok) {
        throw new Error((result.details || result.errors)?.map((item) => `${item.path}: ${item.message}`).join("\n") || `Save failed for ${designId}`);
      }
      S.designRevisions.set(designId, result.revision);
      S.flowDirty.delete(designId);
      const design = S.studio.designs.find((item) => item.id === designId);
      if (design) {
        design.name = doc.name;
        design.revision = result.revision;
        design.orientation = doc.orientation || "portrait";
        design.pageCount = doc.pages?.length ?? design.pageCount;
        design.homePageId = doc.homePageId;
      }
      if (designId === S.currentDesignId) S.appliedRevision = result.revision;
      appliedScreens += result.applied ? result.appliedScreens : 0;
      refreshScreenPreview(designId, activePageIdFor(designId));
    }
    S.dirty = S.flowMode ? S.flowDirty.has(S.editingDesignId) : false;
    renderStudioControls();
    setDocumentState();
    toast(appliedScreens
      ? `Saved ${targets.length} design${targets.length === 1 ? "" : "s"} · applied to ${appliedScreens} screen${appliedScreens === 1 ? "" : "s"}`
      : `Saved ${targets.length} design${targets.length === 1 ? "" : "s"} — no assigned screen is online`);
  } catch (error) {
    if (S.flowMode) S.dirty = S.flowDirty.has(S.editingDesignId);
    setDocumentState();
    toast(error.message, true);
  }
}

export async function assignDesign() {
  if (!S.selectedScreenId) return;
  const response = await fetch(`/api/screens/${encodeURIComponent(S.selectedScreenId)}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ designId: S.currentDesignId }),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || "Assignment failed");
  const screen = S.studio.screens.find((item) => item.id === S.selectedScreenId);
  if (screen) screen.designId = S.currentDesignId;
  renderStudioControls();
  toast(`${result.screen.name} now uses this design`);
}

export async function createDesign() {
  const name = await appPrompt({
    title: "Create a screen design",
    message: "The new design starts as a copy of the design currently open.",
    inputLabel: "Design name", inputValue: "New design", inputPlaceholder: "Kitchen controls",
    confirmLabel: "Create design",
  });
  if (!name?.trim()) return;
  let discardConfirmed = false;
  if (S.dirty || S.codeEditorDirty) {
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
    body: JSON.stringify({ name: name.trim(), cloneFrom: S.currentDesignId }),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || "Could not create design");
  await refreshStudio();
  await loadDesign(result.design.id, { discardConfirmed });
  toast(`Created ${result.design.name}`);
}

export async function updateStatus() {
  try {
    const [status] = await Promise.all([(await fetch("/api/status")).json(), refreshStudio()]);
    $("device-dot").classList.toggle("connected", status.deviceConnected);
    $("device-status").textContent = status.deviceConnected ? `${status.connectedScreens} screen${status.connectedScreens === 1 ? "" : "s"} connected` : "Screens offline";
  } catch {
    $("device-dot").classList.remove("connected");
    $("device-status").textContent = "Server unavailable";
  }
}
