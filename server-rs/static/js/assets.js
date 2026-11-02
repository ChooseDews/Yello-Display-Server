// Image assets: upload or import an image, downscale it in the browser to
// at most 1.5x the screen's largest dimension, and store it on the server
// under a name. Stored images are referenced by image elements as
// /assets/<name>.<ext> and served back by the server, so designs stay
// self-contained and the device renderer loads them from disk.

import { S, displayWidth, displayHeight } from "./state.js";
import { pushHistory, setDocumentState } from "./history.js";
import { renderAll } from "./editor.js";
import { schedulePreview } from "./canvas.js";
import { toast, appPrompt } from "./dialog.js";
import { icon } from "./util.js";

/** Largest stored dimension: 1.5x the current design's screen max side. */
export function maxImageDimension() {
  return Math.round(Math.max(displayWidth(), displayHeight()) * 1.5);
}

function slugify(name) {
  return String(name ?? "")
    .replace(/\.[a-z0-9]+$/i, "")
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}

async function downscaleToBlob(blob) {
  const bitmap = await createImageBitmap(blob);
  const maxDim = maxImageDimension();
  const scale = Math.min(1, maxDim / Math.max(bitmap.width, bitmap.height));
  const width = Math.max(1, Math.round(bitmap.width * scale));
  const height = Math.max(1, Math.round(bitmap.height * scale));
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  canvas.getContext("2d").drawImage(bitmap, 0, 0, width, height);
  bitmap.close?.();
  const type = blob.type === "image/png" ? "image/png" : "image/jpeg";
  return new Promise((resolve) => canvas.toBlob(resolve, type, 0.87));
}

function blobToBase64(blob) {
  return blob.arrayBuffer().then((buffer) => {
    const bytes = new Uint8Array(buffer);
    let binary = "";
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    }
    return btoa(binary);
  });
}

/** Downscale + name + store an image blob. Returns the saved asset or null. */
async function storeImage(blob, suggestedName) {
  if (!["image/png", "image/jpeg"].includes(blob.type)) {
    toast("Only JPG and PNG images are supported.", true);
    return null;
  }
  const name = await appPrompt({
    title: "Save image",
    message: `Stored at up to ${maxImageDimension()} px on the longest side. Saving replaces an image with the same name.`,
    inputLabel: "Image name",
    inputValue: slugify(suggestedName) || "image",
    confirmLabel: "Save image",
  });
  if (!name) return null;
  let scaled;
  try {
    scaled = await downscaleToBlob(blob);
  } catch {
    scaled = null;
  }
  if (!scaled) {
    toast("That image could not be read.", true);
    return null;
  }
  const response = await fetch("/api/assets", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name, data: await blobToBase64(scaled) }),
  });
  const result = await response.json().catch(() => ({}));
  if (!response.ok) {
    toast(result.error || "Image upload failed.", true);
    return null;
  }
  toast(`Saved ${result.file} (${result.width}×${result.height}).`);
  return result;
}

async function fetchAssetList() {
  const response = await fetch("/api/assets");
  if (!response.ok) return [];
  const data = await response.json().catch(() => ({}));
  return Array.isArray(data.assets) ? data.assets : [];
}

/**
 * Append the image tools (upload, import link, saved-image picker) to an
 * image element's inspector fields. Choosing any source points the
 * element's src at the stored asset.
 */
export function appendImageTools(container, element) {
  const apply = (url) => {
    pushHistory();
    element.props.src = url;
    S.dirty = true;
    setDocumentState();
    renderAll();
    schedulePreview();
  };

  const tools = document.createElement("div");
  tools.className = "image-tools";

  const uploadLabel = document.createElement("label");
  uploadLabel.className = "secondary-button image-upload-button";
  uploadLabel.innerHTML = `${icon("i-upload")} Upload JPG/PNG`;
  const fileInput = document.createElement("input");
  fileInput.type = "file";
  fileInput.accept = "image/png,image/jpeg";
  fileInput.hidden = true;
  fileInput.onchange = async () => {
    const file = fileInput.files?.[0];
    fileInput.value = "";
    if (!file) return;
    const saved = await storeImage(file, file.name);
    if (saved) apply(saved.url);
  };
  uploadLabel.append(fileInput);

  const importButton = document.createElement("button");
  importButton.type = "button";
  importButton.className = "secondary-button";
  importButton.innerHTML = `${icon("i-link")} Import link`;
  importButton.onclick = async () => {
    const url = String(element.props.src ?? "").trim();
    if (!/^https?:\/\//i.test(url)) {
      toast("Put an http(s) image link in the Image URL field first.", true);
      return;
    }
    importButton.disabled = true;
    try {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const blob = await response.blob();
      const fallbackName = url.split("/").pop() || "image";
      const saved = await storeImage(blob, fallbackName);
      if (saved) apply(saved.url);
    } catch {
      toast("Could not fetch that link — the host may block cross-origin downloads.", true);
    } finally {
      importButton.disabled = false;
    }
  };

  const picker = document.createElement("select");
  picker.className = "image-picker";
  picker.setAttribute("aria-label", "Saved images");
  const placeholder = document.createElement("option");
  placeholder.value = "";
  placeholder.textContent = "Saved images…";
  picker.append(placeholder);
  const fillPicker = async () => {
    const assets = await fetchAssetList();
    picker.replaceChildren(placeholder);
    for (const asset of assets) {
      const option = document.createElement("option");
      option.value = asset.url;
      option.textContent = asset.name;
      picker.append(option);
    }
    picker.value = element.props.src?.startsWith("/assets/") ? element.props.src : "";
  };
  picker.onfocus = fillPicker;
  picker.onchange = () => {
    if (picker.value) apply(picker.value);
  };
  fillPicker();

  const row = document.createElement("div");
  row.className = "image-tools-row";
  row.append(uploadLabel, importButton);
  tools.append(row, picker);
  container.append(tools);
}
