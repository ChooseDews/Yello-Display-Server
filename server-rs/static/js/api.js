// Thin fetch wrappers for the Studio HTTP API.

import { S } from "./state.js";

export async function fetchJson(url, options = {}) {
  const response = await fetch(url, options);
  const result = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(result.error || `Request failed (${response.status})`);
  }
  return result;
}

/** Load a design document into the local cache (used by flow view and
 *  live navigation). Returns true when the document is available. */
export async function fetchDesignLayout(designId, { force = false } = {}) {
  if (force) {
    S.designLayouts.delete(designId);
    S.flowMissing.delete(designId);
  } else {
    if (S.designLayouts.has(designId) || S.flowMissing.has(designId)) return false;
    const pending = S.flowFetches.get(designId);
    if (pending) return pending;
  }
  if (!S.studio.designs.some((design) => design.id === designId)) {
    S.flowMissing.add(designId);
    return false;
  }
  const load = (async () => {
    try {
      const result = await fetchJson(`/api/layout?design=${encodeURIComponent(designId)}`);
      S.designLayouts.set(designId, result.layout);
      S.designRevisions.set(designId, result.revision);
      S.flowMissing.delete(designId);
      return true;
    } catch {
      S.flowMissing.add(designId);
      return false;
    } finally {
      S.flowFetches.delete(designId);
    }
  })();
  S.flowFetches.set(designId, load);
  return load;
}

export async function postLayout(designId, doc) {
  const response = await fetch(`/api/layout?design=${encodeURIComponent(designId)}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(doc),
  });
  const result = await response.json().catch(() => ({}));
  return { response, result };
}
