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
import { initAgentPage, tryDockAgent, placeAgent } from "./agent.js";

export { initAgentPage };

export function pageFromPath() {
  const name = window.location.pathname.replace(/^\//, "");
  return ["designer", "devices", "designs", "settings", "server", "about", "agent"].includes(name) ? name : "designer";
}

export function showPage(page, push = false) {
  // On desktop the Agent opens as a docked right-hand column instead of a
  // page; tryDockAgent returns false on narrow screens (normal page then).
  if (page === "agent" && tryDockAgent()) return;
  S.currentPage = page;
  document.body.classList.toggle("route-designer", page === "designer");
  for (const name of ["designer", "devices", "designs", "settings", "server", "about", "agent"]) {
    $(`${name}-page`).classList.toggle("hidden", name !== page);
  }
  if (page !== "server") stopServerStats();
  document.querySelectorAll(".designer-actions").forEach((el) => el.classList.toggle("hidden", page !== "designer"));
  document.querySelectorAll("[data-route]").forEach((link) => {
    link.classList.toggle("active", link.dataset.route === page);
  });
  if (push && window.location.pathname !== `/${page}`) history.pushState({ page }, "", `/${page}`);
  if (page === "devices") renderDevicePage();
  if (page === "designs") renderDesignPage();
  if (page === "settings") loadHomeAssistantSettings().catch((error) => toast(error.message, true));
  if (page === "server") renderServerPage();
  if (page === "about") renderAboutPage();
  if (page === "agent") initAgentPage();
  placeAgent();
}

// The About page: project links and a live status blurb.
export async function renderAboutPage() {
  try {
    const status = await (await fetch("/api/status")).json();
    $("about-status").textContent =
      `Visual designer + MCP access for 240 × 320 ESP32 touch displays. ` +
      `This server holds ${status.designsCount} design(s), knows ${status.knownScreens} screen(s), and has ${status.connectedScreens} connected right now.`;
  } catch { /* the static blurb stays */ }
}

// The ESP32 web flasher lives on the Devices page. Primary path: the
// server bundles the latest firmware — CI pulls the repo's rolling
// firmware-latest GitHub release into the image (docker.yml waits for
// that release to be fresh for HEAD before downloading, after a Nov 4,
// 2026 race shipped a stale bin), and the internal flasher flashes that
// bundled manifest (/static/firmware/manifest.json) directly, no
// download or file picking needed.
// Fallback path: the user can still provide their own .bin — the card
// links the GitHub release to download it from, the user picks the
// file(s) here, and the page builds an ESP Web Tools manifest in memory
// (blob URLs) from what was picked. Two shapes are accepted:
//   - yello_device.bin alone: an app-only update at 0x10000, for a board
//     already running Yello (bootloader + partitions are unchanged).
//   - the full release set incl. flasher_args.json (bootloader.bin,
//     partition-table.bin, yello_device.bin): offsets come from
//     flasher_args.json, so a blank board works too.
// The flashing itself is ESP Web Tools (Web Serial), vendored locally
// and loaded lazily: the library installs document-level handlers that
// swallow clicks on the page's other links, so it loads only when the
// user actually goes to flash.
const FIRMWARE_RELEASE_PAGE = "https://github.com/ChooseDews/Yello-Display-Server/releases/tag/firmware-latest";
const FIRMWARE_APP_BIN_URL = "https://github.com/ChooseDews/Yello-Display-Server/releases/download/firmware-latest/yello_device.bin";
const APP_BIN_OFFSET = 65536; // 0x10000 per flasher_args.json
const APP_BIN_MAX_BYTES = 4 * 1024 * 1024;
let flashReady = false;
let flashParts = []; // [{ file, offset }]
let flashBlobUrls = [];

function flashClearBlobs() {
  for (const url of flashBlobUrls) URL.revokeObjectURL(url);
  flashBlobUrls = [];
}

async function flashParseSelection(fileList) {
  const files = [...fileList];
  const byName = new Map(files.map((f) => [f.name, f]));
  const argsFile = byName.get("flasher_args.json");
  if (argsFile) {
    const args = JSON.parse(await argsFile.text());
    const entries = Object.entries(args.flash_files || {});
    if (!entries.length) throw new Error("flasher_args.json lists no flash files.");
    const parts = [];
    for (const [offsetHex, path] of entries) {
      const name = path.split("/").pop();
      const file = byName.get(name);
      if (!file) throw new Error(`Missing ${name} — select every file from the release, including flasher_args.json.`);
      parts.push({ file, offset: parseInt(offsetHex, 16) });
    }
    parts.sort((a, b) => a.offset - b.offset);
    return { parts, full: true };
  }
  if (files.length === 1 && files[0].name === "yello_device.bin") {
    if (!files[0].size || files[0].size > APP_BIN_MAX_BYTES) {
      throw new Error(`That file is ${files[0].size} bytes — yello_device.bin should be a few hundred KB.`);
    }
    return { parts: [{ file: files[0], offset: APP_BIN_OFFSET }], full: false };
  }
  throw new Error("Pick either yello_device.bin on its own (app-only update), or the full release set including flasher_args.json.");
}

let flashScriptLoading = null;
function flashLoadScript() {
  if (!flashScriptLoading) {
    flashScriptLoading = new Promise((resolve, reject) => {
      if (customElements.get("esp-web-install-button")) { resolve(); return; }
      const script = document.createElement("script");
      script.type = "module";
      script.src = "/static/vendor/esp-web-tools/dist/web/install-button.js";
      script.addEventListener("error", reject);
      document.head.append(script);
      customElements.whenDefined("esp-web-install-button").then(resolve, reject);
    });
  }
  return flashScriptLoading;
}

// Swap the card body for the ESP Web Tools install button (pointed at
// manifestUrl) and open its dialog. Used by both the bundled-firmware
// button and the user-provided-files button.
function flashLaunchInstall(region, badge, noteEl, button, manifestUrl) {
  const original = button.textContent;
  button.disabled = true;
  button.textContent = "Loading the flasher\u2026";
  const fail = (message) => {
    badge.textContent = "Flasher unavailable";
    if (noteEl) noteEl.textContent = message;
    button.disabled = false;
    button.textContent = original;
  };
  flashLoadScript().then(() => {
    const install = document.createElement("esp-web-install-button");
    install.setAttribute("manifest", manifestUrl);
    const activate = document.createElement("button");
    activate.slot = "activate";
    activate.className = "primary-button";
    activate.type = "button";
    activate.textContent = "Connect & install firmware";
    const unsupported = document.createElement("span");
    unsupported.slot = "unsupported";
    unsupported.className = "form-help";
    unsupported.textContent = "This browser cannot flash over USB: Web Serial needs Chrome or Edge on a computer.";
    const notAllowed = document.createElement("span");
    notAllowed.slot = "not-allowed";
    notAllowed.className = "form-help";
    notAllowed.textContent = "Browser flashing needs a secure context (HTTPS or localhost).";
    install.append(activate, unsupported, notAllowed);
    region.replaceChildren(install);
    activate.click();
  }).catch(() => fail("The flasher failed to start. The same files flash fine with esptool."));
}

async function initFlasher() {
  if (flashReady) return;
  flashReady = true;
  const region = $("flash-region");
  const badge = $("flash-state");
  badge.textContent = "Checking firmware\u2026";
  region.replaceChildren();

  // Bundled firmware first: probe the manifest the image carries.
  let bundledManifest = null;
  try {
    const response = await fetch("/static/firmware/manifest.json", { cache: "no-store" });
    if (response.ok) bundledManifest = await response.json();
  } catch { /* treated as not bundled */ }
  const bundledParts = bundledManifest?.builds?.[0]?.parts;
  const hasBundled = Array.isArray(bundledParts) && bundledParts.length > 0;

  // Primary path: the bundled firmware, one stacked block.
  const bundledBox = document.createElement("div");
  bundledBox.className = "flash-section";
  if (!hasBundled) badge.textContent = "You provide the .bin";
  if (hasBundled) {
    badge.textContent = "Firmware ready";
    const intro = document.createElement("p");
    intro.className = "form-help";
    intro.textContent = "Bundled: the latest firmware-latest build \u2014 bootloader, partition table and app, no download needed.";
    const otaNote = document.createElement("p");
    otaNote.className = "form-help";
    otaNote.textContent = "Updating from an older version? Flash the bundled firmware in full (not an app-only .bin) \u2014 it installs the new partition layout that enables over-the-air updates.";
    const bundledNote = document.createElement("p");
    bundledNote.className = "form-help";
    const bundledGo = document.createElement("button");
    bundledGo.type = "button";
    bundledGo.className = "primary-button";
    bundledGo.textContent = "Connect & install firmware";
    bundledGo.addEventListener("click", () => {
      flashLaunchInstall(bundledBox, badge, bundledNote, bundledGo, "/static/firmware/manifest.json");
    });
    bundledBox.append(intro, otaNote, bundledGo, bundledNote);
  }

  // Fallback path: the user's own .bin, behind a disclosure.
  const customDetails = document.createElement("details");
  customDetails.className = "flash-custom";
  const customSummary = document.createElement("summary");
  customSummary.textContent = "Use your own .bin instead";
  const customBox = document.createElement("div");
  customBox.className = "flash-section";
  customDetails.append(customSummary, customBox);
  if (!hasBundled) customDetails.open = true;

  const intro = document.createElement("p");
  intro.className = "form-help";
  intro.textContent = hasBundled
    ? "Download from GitHub, then pick the file(s) here. yello_device.bin alone updates a board already running Yello; for a blank board pick the full set, including flasher_args.json."
    : "The server doesn't carry firmware. Download the .bin from GitHub, then pick the file(s) here. yello_device.bin alone updates a board already running Yello; for a blank board pick the full set, including flasher_args.json.";
  const download = document.createElement("div");
  download.className = "flash-downloads";
  const dlApp = document.createElement("a");
  dlApp.className = "secondary-button";
  dlApp.href = FIRMWARE_APP_BIN_URL;
  dlApp.textContent = "Download yello_device.bin";
  const dlRelease = document.createElement("a");
  dlRelease.className = "assign-button";
  dlRelease.href = FIRMWARE_RELEASE_PAGE;
  dlRelease.target = "_blank";
  dlRelease.rel = "noopener";
  dlRelease.textContent = "Release page (full set for a blank board)";
  download.append(dlApp, dlRelease);

  const picker = document.createElement("input");
  picker.type = "file";
  picker.multiple = true;
  picker.accept = ".bin,.json";
  picker.setAttribute("aria-label", "Firmware files");
  const summary = document.createElement("p");
  summary.className = "form-help";
  summary.textContent = "No file chosen yet.";

  const go = document.createElement("button");
  go.type = "button";
  go.className = "primary-button";
  go.textContent = "Connect & install firmware";
  go.disabled = true;

  picker.addEventListener("change", async () => {
    flashParts = [];
    go.disabled = true;
    badge.textContent = hasBundled ? "Firmware ready" : "You provide the .bin";
    if (!picker.files.length) return;
    try {
      const { parts, full } = await flashParseSelection(picker.files);
      flashParts = parts;
      summary.textContent = parts
        .map((p) => `${p.file.name} → 0x${p.offset.toString(16)} (${Math.round(p.file.size / 1024)} KB)`)
        .join(" · ") + (full ? "" : " — app-only update; the board must already run Yello.");
      badge.textContent = "Ready to flash";
      go.disabled = false;
    } catch (error) {
      summary.textContent = error.message;
      badge.textContent = "Check the files";
    }
  });

  // The button stays enabled once files parse: when Web Serial is
  // unavailable (Safari, or an insecure origin) the loaded install
  // button explains that itself via its slots.
  go.addEventListener("click", () => {
    if (!flashParts.length) return;
    flashClearBlobs();
    const parts = flashParts.map((p) => {
      const url = URL.createObjectURL(p.file);
      flashBlobUrls.push(url);
      return { path: url, offset: p.offset };
    });
    const manifest = {
      name: "Yello Device",
      version: "user-provided",
      builds: [{ chipFamily: "ESP32", parts }],
    };
    const manifestUrl = URL.createObjectURL(new Blob([JSON.stringify(manifest)], { type: "application/json" }));
    flashBlobUrls.push(manifestUrl);
    flashLaunchInstall(customBox, badge, summary, go, manifestUrl);
  });

  customBox.append(intro, download, picker, summary, go);
  region.append(bundledBox, customDetails);
}

// ---------- USB serial device configuration ----------
// Companion to the firmware's serial_config listener (firmware 2.1.0+,
// see firmware/main/serial_config.h): a running device accepts its
// Wi-Fi credentials and server address over the same USB cable used
// for flashing (UART0, 115200 8N1, line protocol) and saves them to
// NVS, then reboots. This uses Web Serial directly — a separate port
// session from the ESP Web Tools flasher, so it runs after that
// dialog has closed and the device has booted into the app.
const USB_CFG_STORAGE_KEY = "yelloUsbConfig";

class UsbLineSession {
  constructor(port) {
    this.port = port;
    this.buffer = "";
    this.lines = [];
    this.waiters = [];
    this.closed = false;
  }

  async open() {
    await this.port.open({ baudRate: 115200 });
    this.writer = this.port.writable.getWriter();
    this.reader = this.port.readable.getReader();
    this.pumpDone = (async () => {
      const decoder = new TextDecoder();
      try {
        while (!this.closed) {
          const { value, done } = await this.reader.read();
          if (done) break;
          if (!value) continue;
          this.buffer += decoder.decode(value, { stream: true });
          let idx;
          while ((idx = this.buffer.indexOf("\n")) >= 0) {
            const line = this.buffer.slice(0, idx).replace(/\r$/, "");
            this.buffer = this.buffer.slice(idx + 1);
            const waiter = this.waiters.shift();
            if (waiter) waiter(line); else this.lines.push(line);
          }
        }
      } catch { /* port closed, or the device rebooted mid-session */ }
    })();
  }

  nextLine(timeoutMs) {
    if (this.lines.length) return Promise.resolve(this.lines.shift());
    return new Promise((resolve) => {
      const waiter = (line) => { clearTimeout(timer); resolve(line); };
      const timer = setTimeout(() => {
        const at = this.waiters.indexOf(waiter);
        if (at >= 0) this.waiters.splice(at, 1);
        resolve(null);
      }, timeoutMs);
      this.waiters.push(waiter);
    });
  }

  /* Wait for a line starting with one of prefixes; other lines (boot
   * logs share the UART) are skipped. Optionally resend a command once
   * after ~3 s — the device may still have been booting (or rebooted
   * by the port opening) when the first copy was sent. */
  async waitForReply(prefixes, timeoutMs, resendText) {
    const deadline = Date.now() + timeoutMs;
    let resent = false;
    while (Date.now() < deadline) {
      const line = await this.nextLine(Math.min(3000, deadline - Date.now()));
      if (line === null) {
        if (resendText && !resent) { resent = true; await this.send(resendText); }
        continue;
      }
      if (prefixes.some((prefix) => line.startsWith(prefix))) return line;
    }
    return null;
  }

  async send(text) {
    await this.writer.write(new TextEncoder().encode(text));
  }

  async close() {
    this.closed = true;
    try { await this.reader.cancel(); } catch { /* already closed */ }
    try { this.reader.releaseLock(); } catch { /* ignore */ }
    try { await this.writer.close(); } catch { /* ignore */ }
    try { this.writer.releaseLock(); } catch { /* ignore */ }
    try { await this.port.close(); } catch { /* ignore */ }
  }
}

function usbParseReply(line) {
  const at = line.indexOf(" ");
  if (at < 0) return null;
  try { return JSON.parse(line.slice(at + 1)); } catch { return null; }
}

/* Opens a port the user picks, runs fn(session), always closes. The
 * hello handshake runs first: a device that never answers is either
 * still booting, not a Yello board, or on pre-2.1.0 firmware. */
async function usbWithDevice(fn) {
  const port = await navigator.serial.requestPort();
  const session = new UsbLineSession(port);
  await session.open();
  try {
    await session.send("YELLO_HELLO\n");
    const helloLine = await session.waitForReply(["YELLO_STATUS", "YELLO_ERROR"], 9000, "YELLO_HELLO\n");
    const hello = helloLine && helloLine.startsWith("YELLO_STATUS") ? usbParseReply(helloLine) : null;
    if (!hello) {
      throw new Error("The device didn't answer. Give it a few seconds after flashing to boot, and make sure it runs firmware 2.1.0 or newer (older firmware has no USB configuration).");
    }
    return await fn(session, hello);
  } finally {
    await session.close();
  }
}

let usbCfgReady = false;
function initUsbConfig() {
  if (usbCfgReady) return;
  usbCfgReady = true;
  const badge = $("usb-config-state");
  const result = $("usb-config-result");
  const writeBtn = $("usb-write");
  const readBtn = $("usb-read");
  const ssidInput = $("usb-ssid");
  const passwordInput = $("usb-password");
  const hostInput = $("usb-host");
  const portInput = $("usb-port");

  // Prefill: last-used values (never the password), else guess the
  // server address from how this page was reached — a LAN address or
  // .local name is exactly what the device needs; a public hostname
  // is not, so leave the field to the placeholder hint then.
  let saved = null;
  try { saved = JSON.parse(localStorage.getItem(USB_CFG_STORAGE_KEY) || "null"); } catch { saved = null; }
  if (saved) {
    ssidInput.value = saved.ssid || "";
    hostInput.value = saved.host || "";
    portInput.value = saved.port || 8885;
  } else {
    const host = window.location.hostname;
    if (/^\d+\.\d+\.\d+\.\d+$/.test(host) || host.endsWith(".local") || !host.includes(".")) {
      hostInput.value = host;
    }
    portInput.value = 8885;
  }

  if (!("serial" in navigator)) {
    badge.textContent = "Needs Chrome or Edge";
    result.textContent = "USB configuration needs Web Serial (Chrome or Edge on a computer).";
    writeBtn.disabled = true;
    readBtn.disabled = true;
    return;
  }
  badge.textContent = "Ready";

  const setBusy = (busy, label) => {
    writeBtn.disabled = busy;
    readBtn.disabled = busy;
    if (label) result.textContent = label;
  };

  readBtn.addEventListener("click", async () => {
    setBusy(true, "Talking to the device\u2026");
    try {
      const status = await usbWithDevice(async (_session, hello) => hello);
      ssidInput.value = status.ssid || "";
      hostInput.value = status.host || "";
      portInput.value = status.port || 8885;
      badge.textContent = `Firmware ${status.fw || "?"}`;
      result.textContent = status.configured
        ? `This device is configured: ${status.ssid} \u2192 ${status.host}:${status.port}${status.passwordSet ? " (password saved)" : " (no password saved)"}.`
        : "This device has no saved settings yet — write some below.";
    } catch (error) {
      result.textContent = error.name === "NotFoundError" ? "No device selected." : error.message;
    } finally {
      setBusy(false);
    }
  });

  $("usb-config-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const ssid = ssidInput.value.trim();
    const host = hostInput.value.trim();
    const port = Number(portInput.value);
    if (!ssid || !host || !Number.isInteger(port) || port < 1 || port > 65535) {
      result.textContent = "SSID, server host, and a port between 1 and 65535 are all required.";
      return;
    }
    const payload = { ssid, host, port };
    if (passwordInput.value) payload.password = passwordInput.value;
    setBusy(true, "Talking to the device\u2026");
    try {
      await usbWithDevice(async (session, hello) => {
        await session.send(`YELLO_SET_CONFIG ${JSON.stringify(payload)}\n`);
        const line = await session.waitForReply(["YELLO_OK", "YELLO_ERROR"], 6000);
        if (!line) throw new Error("The device didn't confirm the save — unplug it, plug it back in, and try Read from device to see what it has.");
        if (line.startsWith("YELLO_ERROR")) {
          const parsed = usbParseReply(line);
          throw new Error(`The device refused the settings: ${parsed?.error || "unknown error"}`);
        }
        return hello;
      });
      try { localStorage.setItem(USB_CFG_STORAGE_KEY, JSON.stringify({ ssid, host, port })); } catch { /* private mode */ }
      passwordInput.value = "";
      badge.textContent = "Settings written";
      result.textContent = `Saved. The device is rebooting — it will join \u201c${ssid}\u201d and connect to ${host}:${port}. Watch the Devices list above for it to come online.`;
    } catch (error) {
      result.textContent = error.name === "NotFoundError" ? "No device selected." : error.message;
    } finally {
      setBusy(false);
    }
  });
}

// ---------- Server page ----------
// Live stats from /api/server-stats, polled every 2 s while the page is
// open; showPage stops the polling when the user navigates away.
let serverStatsTimer = null;

function stopServerStats() {
  if (serverStatsTimer) {
    clearInterval(serverStatsTimer);
    serverStatsTimer = null;
  }
}

function fmtBytes(n) {
  if (n === null || n === undefined) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1073741824) return `${(n / 1048576).toFixed(1)} MB`;
  return `${(n / 1073741824).toFixed(2)} GB`;
}

function fmtDuration(totalSeconds) {
  const s = Math.floor(totalSeconds || 0);
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${m}m`;
  if (m) return `${m}m ${s % 60}s`;
  return `${s}s`;
}

function statTile(grid, value, label) {
  const tile = document.createElement("div");
  tile.className = "metric";
  const v = document.createElement("strong");
  v.textContent = value;
  const l = document.createElement("span");
  l.textContent = label;
  tile.append(v, l);
  grid.append(tile);
}

export function renderServerPage() {
  refreshServerStats();
  if (!serverStatsTimer) serverStatsTimer = setInterval(refreshServerStats, 2000);
}

async function refreshServerStats() {
  if (S.currentPage !== "server") {
    stopServerStats();
    return;
  }
  let stats;
  try {
    stats = await (await fetch("/api/server-stats", { cache: "no-store" })).json();
  } catch {
    return;
  }
  $("server-page-summary").textContent = `v${stats.version} · up ${fmtDuration(stats.uptimeSeconds)}`;
  $("server-version").textContent = `v${stats.version}`;
  const grid = $("server-stat-grid");
  grid.replaceChildren();
  statTile(grid, fmtBytes(stats.processMemoryBytes), "Process memory (RSS)");
  statTile(grid, fmtDuration(stats.uptimeSeconds), "Server uptime");
  statTile(
    grid,
    stats.systemMemory?.totalBytes ? `${fmtBytes(stats.systemMemory.availableBytes)} free` : "—",
    `Host memory (${fmtBytes(stats.systemMemory?.totalBytes)} total)`,
  );
  statTile(grid, `${stats.connectedScreens} connected`, `${stats.knownScreens} known screens`);
  const lib = $("server-lib-grid");
  lib.replaceChildren();
  statTile(lib, String(stats.designsCount), "Designs");
  statTile(lib, String(stats.cachedImages), "Cached images");
  statTile(lib, String(stats.externalValues), "External data values");
  statTile(lib, stats.homeAssistant?.configured ? `${stats.homeAssistant.entities} entities` : "Not connected", "Home Assistant");
  $("server-connections-badge").textContent = `${stats.connectedScreens} connected`;
  const rows = $("server-connections");
  rows.replaceChildren();
  if (!stats.connections?.length) {
    const empty = document.createElement("p");
    empty.className = "form-help";
    empty.textContent = "No screens are connected right now.";
    rows.append(empty);
  }
  for (const conn of stats.connections || []) {
    const row = document.createElement("div");
    row.className = "ha-summary-row";
    const label = document.createElement("span");
    label.textContent = conn.screenId;
    const detail = document.createElement("strong");
    detail.textContent = `${conn.remote} · up ${fmtDuration(conn.connectedSeconds)} · ${fmtBytes(conn.wireBytes)} pushed · ${compactNumber(conn.pixels)} px · ${compactNumber(conn.zoneMessages)} zones`;
    row.append(label, detail);
    rows.append(row);
  }
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
      metrics.textContent = `${Number(screen.stream.framesPerSecond || 0).toFixed(1)} fps · ${compactNumber(screen.stream.pixelsPerSecond || 0)} px/s · ${Number(screen.stream.kilobitsPerSecond || 0).toFixed(1)} kb/s`;
      copy.append(metrics);
      card.title += ` · ${Number(screen.stream.zoneMessagesPerSecond || 0).toFixed(1)} zones/s · last push ${Number(screen.stream.lastPushMs || 0).toFixed(1)} ms`;
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

function fwIsBehind(deviceFw, bundledFw) {
  if (!deviceFw || !bundledFw || deviceFw === bundledFw) return false;
  const parse = (s) => {
    const parts = String(s).split(".").map((p) => parseInt(p, 10));
    return parts.some((n) => Number.isNaN(n)) ? null : [parts[0] || 0, parts[1] || 0, parts[2] || 0];
  };
  const d = parse(deviceFw); const b = parse(bundledFw);
  if (!d || !b) return false;
  for (let i = 0; i < 3; i++) { if (d[i] !== b[i]) return d[i] < b[i]; }
  return false;
}

function renderOtaSection(screen) {
  const bundled = S.studio.bundledFirmwareVersion || screen.bundledFirmwareVersion;
  if (!screen.connected || !bundled) return null;
  const behind = screen.otaUpdateAvailable ?? fwIsBehind(screen.firmwareVersion, bundled);
  const ota = screen.ota || {};
  const wrap = document.createElement("div");
  wrap.className = "ota-section";
  if (ota.inFlight || ["offered", "accepted", "downloading", "verifying", "rebooting"].includes(ota.state)) {
    const text = document.createElement("p");
    text.className = "form-help";
    const progress = ota.progress !== null && ota.progress !== undefined && ota.state === "downloading" ? ` ${ota.progress}%` : "";
    text.textContent = `Updating: ${ota.state}${progress}\u2026`;
    wrap.append(text);
    return wrap;
  }
  if (ota.state === "failed") {
    const text = document.createElement("p");
    text.className = "form-help";
    text.textContent = `Update failed${ota.error ? `: ${ota.error}` : ""}`;
    wrap.append(text);
    if (!behind) return wrap;
  }
  if (!behind) return ota.state === "failed" ? wrap : null;
  if (screen.supportsOta) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "primary-button";
    btn.textContent = `Update to v${bundled}`;
    btn.addEventListener("click", async () => {
      btn.disabled = true;
      try {
        const response = await fetch(`/api/devices/${encodeURIComponent(screen.id)}/update`, { method: "POST" });
        const result = await response.json();
        if (result.result !== "offered" && result.status !== "offered") throw new Error(result.result || result.status || "Update was not offered");
        toast(`Update to v${bundled} offered to ${screen.name}`);
        await refreshStudio();
      } catch (error) {
        btn.disabled = false;
        toast(error.message, true);
      }
    });
    wrap.append(btn);
    return wrap;
  }
  const note = document.createElement("p");
  note.className = "form-help";
  note.textContent = `Needs the one-time USB update to ${bundled} for over-the-air updates`;
  wrap.append(note);
  return wrap;
}

export function renderDevicePage() {
  initFlasher().catch(() => {});
  initUsbConfig();
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
    const player = document.createElement("a");
    player.className = "secondary-button";
    player.textContent = "Open player";
    player.href = `/player?screen=${encodeURIComponent(screen.id)}`;
    player.target = "_blank";
    player.rel = "noopener";
    const embed = document.createElement("button");
    embed.type = "button";
    embed.className = "secondary-button";
    embed.textContent = "Copy embed";
    embed.onclick = async () => {
      const snippet = `<iframe src="${window.location.origin}/player?screen=${encodeURIComponent(screen.id)}" width="480" height="640" style="border:0;border-radius:12px" title="${screen.name} — Yello live view"></iframe>`;
      try {
        await navigator.clipboard.writeText(snippet);
        toast("Embed code copied — paste it anywhere an iframe fits");
      } catch {
        toast("Clipboard is unavailable in this browser", true);
      }
    };
    actions.append(player, embed);
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
    const otaSection = renderOtaSection(screen);
    if (otaSection) card.append(head, metrics, otaSection, form);
    else card.append(head, metrics, form);
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

// Returning from Home Assistant's OAuth approval lands back on /settings
// with ?code=. The HA base URL and client_id (this origin) were stashed in
// sessionStorage before leaving, because the settings form reloads empty.
async function completeHomeAssistantOAuth() {
  const params = new URLSearchParams(window.location.search);
  const code = params.get("code");
  if (!code) return;
  history.replaceState({}, "", "/settings");
  let stash = {};
  try {
    stash = JSON.parse(sessionStorage.getItem("yello-ha-oauth") || "{}");
    sessionStorage.removeItem("yello-ha-oauth");
  } catch { /* fall through with form values */ }
  const url = stash.url || $("ha-url").value.trim();
  const clientId = stash.clientId || window.location.origin;
  $("ha-settings-result").textContent = "Completing Home Assistant sign-in…";
  try {
    const response = await fetch("/api/settings/home-assistant/oauth/exchange", {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ url, code, clientId }),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || "Sign-in exchange failed");
    S.homeAssistant = await (await fetch("/api/integrations/home-assistant")).json();
    renderHomeAssistantOptions();
    toast(result.connected
      ? `Home Assistant connected · ${result.entityCount} entities found`
      : "Home Assistant signed in, but the connection test failed", !result.connected);
  } catch (error) {
    toast(error.message, true);
  }
}

export function startHomeAssistantOAuth() {
  const url = $("ha-url").value.trim().replace(/\/+$/, "");
  if (!url) {
    toast("Enter your Home Assistant URL first, then sign in.", true);
    $("ha-url").focus();
    return;
  }
  $("ha-url").value = url;
  const clientId = window.location.origin;
  try {
    sessionStorage.setItem("yello-ha-oauth", JSON.stringify({ url, clientId }));
  } catch { /* the exchange falls back to the saved settings URL */ }
  const authorize = `${url}/auth/authorize?client_id=${encodeURIComponent(clientId)}` +
    `&redirect_uri=${encodeURIComponent(clientId + "/settings")}&response_type=code`;
  window.location.assign(authorize);
}

export async function loadHomeAssistantSettings() {
  await completeHomeAssistantOAuth();
  const response = await fetch("/api/settings/home-assistant");
  if (!response.ok) throw new Error("Could not load Home Assistant settings");
  S.homeAssistantSettings = await response.json();
  $("ha-url").value = S.homeAssistantSettings.url || "";
  $("ha-token").value = "";
  $("ha-token").placeholder = S.homeAssistantSettings.tokenConfigured ? "Saved — leave blank to keep it" : "Paste a long-lived access token";
  const badge = $("ha-settings-state");
  const method = S.homeAssistantSettings.authMethod;
  badge.className = `integration-badge${S.homeAssistantSettings.configured ? " connected" : ""}`;
  badge.textContent = !S.homeAssistantSettings.configured
    ? "Not configured"
    : method === "oauth" ? "Connected · OAuth" : "Connected · Token";
  // Connected: collapse the entry form to a summary (URL, method, status)
  // with just a Disconnect action; the form returns on disconnect.
  const connected = Boolean(S.homeAssistantSettings.configured);
  $("ha-settings-form").classList.toggle("hidden", connected);
  document.querySelector(".oauth-block").classList.toggle("hidden", connected);
  $("ha-summary").classList.toggle("hidden", !connected);
  if (connected) {
    $("ha-summary-url").textContent = S.homeAssistantSettings.url || "";
    $("ha-summary-method").textContent = method === "oauth" ? "OAuth sign-in" : "Long-lived token";
    $("ha-summary-status").textContent = "Connected";
  }
  $("ha-settings-result").textContent = S.homeAssistantSettings.tokenConfigured
    ? "A private token is stored."
    : method === "oauth" ? "Signed in with Home Assistant (OAuth) — no token is stored." : "";
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
