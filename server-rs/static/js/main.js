// Studio entry point: event wiring, keyboard shortcuts, boot.

import { $, isNarrowScreen } from "./util.js";
import { S, displayWidth, displayHeight, activePage, effectiveElements, activePageIdFor } from "./state.js";
import { toast, isModalOpen, closeAppModal, submitAppModal, appConfirm } from "./dialog.js";
import { commit, undo, redo, setDocumentState } from "./history.js";
import {
  selected, addElement, duplicateSelected, deleteSelected, alignSelected,
  setOrientation, hideContextMenu, showContextMenu, renderAll,
} from "./editor.js";
import {
  setCanvasMode, sendLiveTouch, applyCodeLayout, formatCodeEditor, applyZoom, refreshPreview,
  showCodeErrors, setCodeEditorState, stepZoom, setZoom, initCanvasGestures,
} from "./canvas.js";
import { setShowConnections, drawFlowArrows } from "./flow.js";
import {
  pageFromPath, showPage, renderStudioControls, renderHomeAssistantOptions, renderDesignPage,
  selectScreen, loadDesign, assignDesign, createDesign, saveLayout, saveHomeAssistantSettings, startHomeAssistantOAuth, updateStatus,
} from "./views.js?v=3";

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

// Phones get slide-over drawers: Blocks from the left, Attributes from
// the right, toggled from the top bar; the canvas is always underneath.
// Desktop ignores data-drawer entirely.
export function setDrawer(name) {
  document.body.dataset.drawer = name || "";
  const blocks = $("toggle-blocks");
  const attributes = $("toggle-attributes");
  if (blocks) blocks.setAttribute("aria-pressed", String(name === "blocks"));
  if (attributes) attributes.setAttribute("aria-pressed", String(name === "attributes"));
}

// On phones there is no header at all: the undo/redo/save buttons live
// in the page bar (next to the page actions) and the screen/design
// deployment controls live in the canvas toolbar. The same DOM nodes are
// moved back into the header on wide screens, so ids, bindings, and the
// designer-actions visibility toggling keep working in both layouts.
const narrowLayoutQuery = typeof window !== "undefined" && window.matchMedia
  ? window.matchMedia("(max-width: 860px)")
  : { matches: false, addEventListener: () => {} };
export function placeHeaderControls() {
  const undo = $("undo");
  const redo = $("redo");
  const save = $("save");
  const deployment = document.querySelector(".deployment-controls");
  if (!undo || !redo || !save || !deployment) return;
  if (narrowLayoutQuery.matches) {
    $("edit-actions")?.append(undo, redo, save);
    $("deploy-slot")?.append(deployment);
  } else {
    document.querySelector(".header-actions")?.append(undo, redo, save);
    document.querySelector(".sub-bar")?.append(deployment);
  }
}

function bindEvents() {
  placeHeaderControls();
  narrowLayoutQuery.addEventListener("change", placeHeaderControls);
  document.querySelectorAll("[data-copy-target]").forEach((button) => {
    button.onclick = async () => {
      const target = $(button.dataset.copyTarget);
      if (!target) return;
      const text = target.textContent;
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        const range = document.createRange();
        range.selectNodeContents(target);
        const selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        document.execCommand("copy");
        selection.removeAllRanges();
      }
      toast("Copied to clipboard");
    };
  });
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
      if (isNarrowScreen()) setDrawer("");
    };
  });
  $("toggle-blocks").onclick = () => setDrawer(document.body.dataset.drawer === "blocks" ? "" : "blocks");
  $("toggle-attributes").onclick = () => setDrawer(document.body.dataset.drawer === "attributes" ? "" : "attributes");
  document.querySelectorAll(".drawer-close").forEach((button) => {
    button.onclick = () => setDrawer("");
  });
  $("drawer-backdrop").onclick = () => setDrawer("");
  document.addEventListener("yello:show-canvas", () => {
    if (isNarrowScreen()) setDrawer("");
  });
  initCanvasGestures();
  // The page itself must never pinch-zoom (the canvas has its own zoom);
  // iOS Safari-family browsers expose this as a gesture event.
  document.addEventListener("gesturestart", (event) => event.preventDefault());
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
  $("ha-oauth").onclick = () => startHomeAssistantOAuth();
  $("duplicate").onclick = duplicateSelected;
  $("zoom-out").onclick = () => stepZoom(1 / 1.25);
  $("zoom-in").onclick = () => stepZoom(1.25);
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
    setDrawer("");
    if (isNarrowScreen()) S.zoom = 1;
    const mcpUrl = `${window.location.origin}/mcp`;
    if ($("mcp-endpoint")) $("mcp-endpoint").textContent = mcpUrl;
    if ($("mcp-config")) {
      $("mcp-config").textContent = JSON.stringify(
        { mcpServers: { yello: { url: mcpUrl } } }, null, 2);
    }
    if ($("mcp-prompt")) {
      $("mcp-prompt").textContent =
`Connect to my Yello Studio display server so you can manage my screen designs.

Add this MCP server (Streamable HTTP, no authentication — it runs on my private network):
{"mcpServers": {"yello": {"url": "${mcpUrl}"}}}

Once connected you have these tools: yello_status, yello_list_designs, yello_get_design, yello_save_design, yello_validate_design, yello_preview_design (renders a design page to a PNG image so you can see it), yello_list_screens, yello_assign_design.

Designs are schema v2 documents: {schemaVersion: 2, name, profile, orientation, background, homePageId, pages: [{id, name, background, elements}], sharedElements, dataSources, actions}. Each element is {id, type, frame: {x, y, w, h}, style, props}. Supported types: text, clock, button, image, container, divider, toggle, slider, nav, screen (embeds another design's page inside the frame), gauge, qr, shape.

Workflow: first list my designs and screens so you know what exists. Before saving any change, run yello_validate_design, then yello_preview_design and look at the rendered image to check the layout. Save with yello_save_design — saves are pushed to the screens using that design.`;
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
