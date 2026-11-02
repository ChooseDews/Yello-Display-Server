// Studio entry point: event wiring, keyboard shortcuts, boot.

import { $, ZOOM_LEVELS, isNarrowScreen } from "./util.js";
import { S, displayWidth, displayHeight, activePage, effectiveElements, activePageIdFor } from "./state.js";
import { toast, isModalOpen, closeAppModal, submitAppModal, appConfirm } from "./dialog.js";
import { commit, undo, redo, setDocumentState } from "./history.js";
import {
  selected, addElement, duplicateSelected, deleteSelected, alignSelected,
  setOrientation, hideContextMenu, showContextMenu, renderAll,
} from "./editor.js";
import {
  setCanvasMode, sendLiveTouch, applyCodeLayout, formatCodeEditor, applyZoom, refreshPreview,
  showCodeErrors, setCodeEditorState,
} from "./canvas.js";
import { setShowConnections, drawFlowArrows } from "./flow.js";
import {
  pageFromPath, showPage, renderStudioControls, renderHomeAssistantOptions, renderDesignPage,
  selectScreen, loadDesign, assignDesign, createDesign, saveLayout, saveHomeAssistantSettings, updateStatus,
} from "./views.js";

function handleKeyboard(event) {
  if (isModalOpen()) {
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

// Phones get one panel at a time (canvas / blocks / design) switched by
// the bottom tab bar; desktop ignores data-mobile-panel entirely.
export function setMobilePanel(name) {
  document.body.dataset.mobilePanel = name;
  document.querySelectorAll(".mobile-tabs [data-mobile-panel]").forEach((button) => {
    button.classList.toggle("active", button.dataset.mobilePanel === name);
  });
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
    button.onclick = () => {
      addElement(button.dataset.addType);
      if (isNarrowScreen()) setMobilePanel("canvas");
    };
  });
  document.querySelectorAll(".mobile-tabs [data-mobile-panel]").forEach((button) => {
    button.onclick = () => setMobilePanel(button.dataset.mobilePanel);
  });
  document.addEventListener("yello:show-canvas", () => {
    if (isNarrowScreen()) setMobilePanel("canvas");
  });
  $("add-shared-toggle").onclick = () => {
    S.addAsShared = !S.addAsShared;
    $("add-shared-toggle").classList.toggle("active", S.addAsShared);
    $("add-shared-toggle").setAttribute("aria-pressed", String(S.addAsShared));
  };
  $("canvas-overlay").onclick = (event) => {
    if (event.target === $("canvas-overlay")) {
      S.selectedId = null;
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
  $("show-connections").onclick = () => {
    hideContextMenu();
    setShowConnections(!S.flowMode).catch((error) => toast(error.message, true));
  };
  $("canvas-scroll").addEventListener("scroll", () => {
    if (S.flowMode) drawFlowArrows();
  });
  $("layout-json").oninput = () => {
    S.codeEditorDirty = true;
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
  window.addEventListener("resize", () => {
    if (S.flowMode) drawFlowArrows();
  });
  $("screen-name").onchange = (event) => commit(() => { S.layout.name = event.target.value; });
  $("page-name").onchange = (event) => commit(() => {
    const page = activePage();
    if (page) page.name = event.target.value;
  });
  $("background").onchange = (event) => commit(() => {
    const page = activePage();
    if (page) page.background = event.target.value;
  });
  $("idle-home").onchange = (event) => commit(() => {
    S.layout.idleHomeSeconds = Math.max(0, Math.min(86400, Math.round(Number(event.target.value) || 0)));
  });
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
  $("zoom-out").onclick = () => { S.zoomIndex = Math.max(0, S.zoomIndex - 1); applyZoom(); };
  $("zoom-in").onclick = () => { S.zoomIndex = Math.min(ZOOM_LEVELS.length - 1, S.zoomIndex + 1); applyZoom(); };
  $("app-modal-cancel").onclick = () => closeAppModal(null);
  $("app-modal-backdrop").onclick = (event) => {
    if (event.target === $("app-modal-backdrop")) closeAppModal(null);
  };
  $("app-modal-input").oninput = () => $("app-modal-input").classList.remove("invalid");
  $("app-modal-form").onsubmit = (event) => {
    event.preventDefault();
    submitAppModal();
  };
  document.addEventListener("keydown", handleKeyboard);
}

async function initialize() {
  try {
    const [studioResponse, catalogResponse, homeAssistantResponse] = await Promise.all([
      fetch("/api/studio"), fetch("/api/catalog"), fetch("/api/integrations/home-assistant"),
    ]);
    S.studio = await studioResponse.json();
    S.catalog = await catalogResponse.json();
    S.homeAssistant = await homeAssistantResponse.json();
    renderHomeAssistantOptions();
    const initialScreen = S.studio.screens.find((item) => item.connected) || S.studio.screens[0];
    S.selectedScreenId = initialScreen?.id || "";
    S.currentDesignId = initialScreen?.designId || S.studio.designs[0]?.id || "default";
    const loaded = await (await fetch(`/api/layout?design=${encodeURIComponent(S.currentDesignId)}`)).json();
    S.layout = loaded.layout;
    S.appliedRevision = loaded.revision;
    S.editingDesignId = S.currentDesignId;
    S.designLayouts.set(S.currentDesignId, S.layout);
    S.designRevisions.set(S.currentDesignId, loaded.revision);
    S.activePages.set(S.currentDesignId, activePageIdFor(S.currentDesignId));
    bindEvents();
    setMobilePanel("canvas");
    if (isNarrowScreen()) S.zoomIndex = 0;
    const mcpUrl = `${window.location.origin}/mcp`;
    if ($("mcp-endpoint")) $("mcp-endpoint").textContent = mcpUrl;
    if ($("mcp-config")) {
      $("mcp-config").textContent = JSON.stringify(
        { mcpServers: { yello: { url: mcpUrl } } }, null, 2);
    }
    applyZoom();
    renderStudioControls();
    renderAll();
    showPage(pageFromPath());
    await Promise.all([refreshPreview(), updateStatus()]);
    setInterval(updateStatus, 3000);
    setInterval(() => {
      if (!S.layout) return;
      const pageId = activePageIdFor(S.editingDesignId ?? S.currentDesignId);
      if (effectiveElements(S.layout, pageId).some((element) => element.type === "clock" && element.visible)) refreshPreview();
    }, 1000);
  } catch (error) {
    setDocumentState("Could not load project");
    toast(error.message, true);
  }
}

initialize();
