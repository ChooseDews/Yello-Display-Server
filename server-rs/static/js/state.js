// Central Studio state plus the Tier 2 design/page model helpers.
//
// A *design* (schema v2) is a document:
//   { name, orientation, background, idleHomeSeconds, dataSources,
//     actions, homePageId, pages: [{id, name, background, elements}],
//     sharedElements: [element & {pages: [pageId, ...]}] }
// Shared elements are opt-in per page: a shared element renders only on
// the pages listed in its `pages` array — never implicitly.

import { LANDSCAPE_HEIGHT, LANDSCAPE_WIDTH, PORTRAIT_HEIGHT, PORTRAIT_WIDTH } from "./util.js";

export const S = {
  layout: null, // the design document being edited (=== designLayouts.get(editingDesignId))
  catalog: null,
  homeAssistant: { configured: false, entities: [] },
  selectedId: null,
  appliedRevision: 0,
  dirty: false,
  zoomIndex: 2,
  undoStack: [],
  redoStack: [],
  previewTimer: null,
  previewUrl: null,
  previewController: null,
  studio: { designs: [], screens: [] },
  currentDesignId: "default",
  selectedScreenId: "",
  currentPage: "designer",
  homeAssistantSettings: { url: "", configured: false, tokenConfigured: false },
  liveMode: false,
  canvasMode: "edit",
  codeEditorDirty: false,
  flowMode: false,
  flowOrder: [], // [{designId, pageId}] nodes in flow (connections) view
  flowStructureKey: "",
  flowMissing: new Set(), // design ids that could not be loaded
  designLayouts: new Map(), // designId -> design document
  designRevisions: new Map(),
  flowDirty: new Set(), // design ids with unsaved changes (flow mode)
  flowFetches: new Map(),
  screenPreviewUrls: new Map(), // nodeKey -> object URL
  screenPreviewTimers: new Map(),
  flowRebuildTimer: null,
  editingDesignId: null,
  activePages: new Map(), // designId -> pageId last edited for that design
  liveNav: null, // {designId, pageId, stack: [{designId, pageId}]} in live mode
  addAsShared: false, // palette toggle: new blocks become shared elements
};

export const displayWidth = () =>
  S.layout?.orientation === "landscape" ? LANDSCAPE_WIDTH : PORTRAIT_WIDTH;
export const displayHeight = () =>
  S.layout?.orientation === "landscape" ? LANDSCAPE_HEIGHT : PORTRAIT_HEIGHT;

export function designDims(designId) {
  const doc = S.designLayouts.get(designId) || S.layout;
  return doc?.orientation === "landscape"
    ? [LANDSCAPE_WIDTH, LANDSCAPE_HEIGHT]
    : [PORTRAIT_WIDTH, PORTRAIT_HEIGHT];
}

export function designNameOf(designId) {
  return S.studio.designs.find((design) => design.id === designId)?.name || designId;
}

// ---- Page model ----

export function homePageIdOf(doc) {
  if (!doc?.pages?.length) return "main";
  if (doc.pages.some((page) => page.id === doc.homePageId)) return doc.homePageId;
  return doc.pages[0].id;
}

export function pageOf(doc, pageId) {
  return doc?.pages?.find((page) => page.id === pageId) || null;
}

export function pageNameOf(doc, pageId) {
  return pageOf(doc, pageId)?.name || pageId;
}

export function docFor(designId) {
  if (designId === S.editingDesignId && S.layout) return S.layout;
  return S.designLayouts.get(designId) || null;
}

export function activePageIdFor(designId) {
  const doc = docFor(designId);
  const stored = S.activePages.get(designId);
  if (doc && stored && pageOf(doc, stored)) return stored;
  return doc ? homePageIdOf(doc) : "main";
}

export function setActivePage(designId, pageId) {
  S.activePages.set(designId, pageId);
}

/** The page object of the design currently being edited. */
export function activePage() {
  if (!S.layout?.pages?.length) return null;
  return pageOf(S.layout, activePageIdFor(S.editingDesignId ?? S.currentDesignId)) || S.layout.pages[0];
}

/**
 * Elements visible on a page, in render order: shared elements opted
 * into the page (below), then the page's own elements. Returns live
 * object references so the editor mutates the document in place.
 */
export function effectiveElements(doc, pageId) {
  const page = pageOf(doc, pageId) || doc?.pages?.[0];
  if (!page) return [];
  const shared = (doc.sharedElements || []).filter(
    (element) => Array.isArray(element.pages) && element.pages.includes(page.id),
  );
  return [...shared, ...(page.elements || [])];
}

export function isSharedElement(doc, element) {
  return Boolean(element) && (doc?.sharedElements || []).some((el) => el.id === element.id);
}

/** Find an element of the editing design: on the active page first,
 *  then anywhere in the shared pool. */
export function findElement(elementId) {
  if (!S.layout || !elementId) return null;
  const pageId = activePageIdFor(S.editingDesignId ?? S.currentDesignId);
  return (
    effectiveElements(S.layout, pageId).find((element) => element.id === elementId) ||
    (S.layout.sharedElements || []).find((element) => element.id === elementId) ||
    null
  );
}

export function allDesignElementIds(doc) {
  const ids = new Set();
  for (const page of doc?.pages || []) for (const el of page.elements || []) ids.add(el.id);
  for (const el of doc?.sharedElements || []) ids.add(el.id);
  for (const source of doc?.dataSources || []) ids.add(source.id);
  for (const action of doc?.actions || []) ids.add(action.id);
  return ids;
}

// ---- Design-link targets ----
// "__back" | "__home" | "page:<pageId>" | "<designId>" | "<designId>/<pageId>"

export function parseLinkTarget(target) {
  if (target === "__back") return { kind: "back" };
  if (target === "__home") return { kind: "home" };
  if (typeof target === "string" && target.startsWith("page:")) {
    return { kind: "page", pageId: target.slice(5) };
  }
  if (typeof target === "string" && target.includes("/")) {
    const cut = target.indexOf("/");
    return { kind: "designPage", designId: target.slice(0, cut), pageId: target.slice(cut + 1) };
  }
  return { kind: "design", designId: target };
}

/** Resolve a link target used in `fromDesignId` to a {designId, pageId}
 *  node, or null for __back / unresolvable targets. */
export function resolveLinkNode(fromDesignId, target) {
  const parsed = parseLinkTarget(target);
  if (parsed.kind === "back") return null;
  if (parsed.kind === "home") {
    const doc = docFor(fromDesignId);
    return { designId: fromDesignId, pageId: doc ? homePageIdOf(doc) : "main" };
  }
  if (parsed.kind === "page") return { designId: fromDesignId, pageId: parsed.pageId };
  if (parsed.kind === "design") {
    const doc = docFor(parsed.designId);
    const meta = S.studio.designs.find((design) => design.id === parsed.designId);
    return { designId: parsed.designId, pageId: doc ? homePageIdOf(doc) : meta?.homePageId || "main" };
  }
  return { designId: parsed.designId, pageId: parsed.pageId };
}

export function nodeKey(designId, pageId) {
  return `${designId}|${pageId}`;
}

/** Flat [value, label] options for the design-link target select. */
export function linkTargetOptions(doc, designId) {
  const options = [
    ["__back", "← Back (previous page)"],
    ["__home", "⌂ Home (this device's home page)"],
  ];
  for (const page of doc?.pages || []) {
    options.push([`page:${page.id}`, `This design · Page: ${page.name}`]);
  }
  for (const design of S.studio.designs) {
    if (design.id === designId) continue;
    options.push([design.id, `Design: ${design.name} (home page)`]);
    const otherDoc = S.designLayouts.get(design.id);
    if (otherDoc) {
      for (const page of otherDoc.pages || []) {
        if (page.id === homePageIdOf(otherDoc)) continue;
        options.push([`${design.id}/${page.id}`, `Design: ${design.name} · Page: ${page.name}`]);
      }
    }
  }
  return options;
}
