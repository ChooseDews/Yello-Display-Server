// Toasts and the app modal (confirm / prompt / small form dialogs).

import { $ } from "./util.js";

let activeModal = null;

export function isModalOpen() {
  return Boolean(activeModal);
}

export function toast(message, error = false) {
  const item = document.createElement("div");
  item.className = `toast${error ? " error" : ""}`;
  item.textContent = message;
  $("toast-region").append(item);
  setTimeout(() => item.remove(), 3500);
}

export function closeAppModal(value) {
  if (!activeModal) return;
  const { resolve, returnFocus } = activeModal;
  activeModal = null;
  $("app-modal-backdrop").classList.add("hidden");
  $("app-modal-confirm").classList.remove("app-modal-confirm-danger");
  $("app-modal-options").classList.add("hidden");
  $("app-modal-options").replaceChildren();
  resolve(value);
  if (returnFocus?.isConnected) returnFocus.focus();
}

function showAppModal({
  title, message, confirmLabel = "Confirm", cancelLabel = "Cancel", danger = false,
  inputLabel = "", inputValue = "", inputPlaceholder = "", inputMaxLength = 100,
  options = null, // [{id, label, checked}] checkbox rows (form dialogs)
}) {
  if (activeModal) closeAppModal(null);
  const hasInput = Boolean(inputLabel);
  const hasOptions = Array.isArray(options) && options.length > 0;
  $("app-modal-title").textContent = title;
  $("app-modal-message").textContent = message;
  $("app-modal-cancel").textContent = cancelLabel;
  $("app-modal-confirm").textContent = confirmLabel;
  $("app-modal-confirm").classList.toggle("app-modal-confirm-danger", danger);
  $("app-modal-input-label").textContent = inputLabel;
  $("app-modal-input-label").classList.toggle("hidden", !hasInput);
  $("app-modal-input").classList.toggle("hidden", !hasInput);
  $("app-modal-input").value = inputValue;
  $("app-modal-input").placeholder = inputPlaceholder;
  $("app-modal-input").maxLength = inputMaxLength;
  const optionsBox = $("app-modal-options");
  optionsBox.replaceChildren();
  optionsBox.classList.toggle("hidden", !hasOptions);
  if (hasOptions) {
    for (const option of options) {
      const label = document.createElement("label");
      label.className = "app-modal-option";
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.dataset.optionId = option.id;
      checkbox.checked = Boolean(option.checked);
      const copy = document.createElement("span");
      copy.textContent = option.label;
      label.append(checkbox, copy);
      optionsBox.append(label);
    }
  }
  $("app-modal-backdrop").classList.remove("hidden");
  return new Promise((resolve) => {
    activeModal = { resolve, returnFocus: document.activeElement, hasInput, hasOptions };
    requestAnimationFrame(() => (hasInput ? $("app-modal-input") : $("app-modal-confirm")).focus());
  });
}

export async function appConfirm(options) {
  return Boolean(await showAppModal(options));
}

export async function appPrompt(options) {
  const value = await showAppModal(options);
  return typeof value === "string" ? value : null;
}

/** Prompt with a text input plus checkboxes. Resolves to
 *  { value, checks: {optionId: bool} } or null when cancelled. */
export async function appForm(options) {
  const value = await showAppModal(options);
  return value && typeof value === "object" ? value : null;
}

/** Called by the modal form's submit handler in main.js. */
export function submitAppModal() {
  if (!activeModal) return;
  if (activeModal.hasOptions) {
    const checks = {};
    for (const checkbox of $("app-modal-options").querySelectorAll("input[type=checkbox]")) {
      checks[checkbox.dataset.optionId] = checkbox.checked;
    }
    const value = activeModal.hasInput ? $("app-modal-input").value.trim() : "";
    if (activeModal.hasInput && !value) {
      $("app-modal-input").classList.add("invalid");
      $("app-modal-input").focus();
      return;
    }
    closeAppModal({ value, checks });
    return;
  }
  if (activeModal.hasInput) {
    const value = $("app-modal-input").value.trim();
    if (!value) {
      $("app-modal-input").classList.add("invalid");
      $("app-modal-input").focus();
      return;
    }
    closeAppModal(value);
  } else {
    closeAppModal(true);
  }
}
