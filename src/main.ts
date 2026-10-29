import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

// === Types ===
interface Device {
  id: string;
  name: string;
  ip: string;
  port: number;
  os: string;
}

interface FileEntry {
  path: string;
  name: string;
  size: number;
}

interface TransferRequest {
  from_id: string;
  from_name: string;
  files: { name: string; size: number }[];
  transfer_id: string;
}

// === State ===
let devices: Device[] = [];
let selectedDevice: Device | null = null;
let selectedFiles: FileEntry[] = [];
let myName = "";

// === DOM refs ===
const $ = (s: string) => document.querySelector(s) as HTMLElement;
const deviceList = $("#deviceList");
const emptyDevices = $("#emptyDevices");
const dropZone = $("#dropZone");
const fileInput = $("#fileInput") as HTMLInputElement;
const selectedFilesEl = $("#selectedFiles");
const fileList = $("#fileList");
const btnSend = $("#btnSend") as HTMLButtonElement;
const transfersEl = $("#transfers");
const transferList = $("#transferList");
const receiveModal = $("#receiveModal");
const receiveInfo = $("#receiveInfo");
const toast = $("#toast");
const settingsOverlay = $("#settingsOverlay");
const updateModal = $("#updateModal");
const updateInfo = $("#updateInfo");
const updateProgress = $("#updateProgress");
const updateProgressFill = $("#updateProgressFill");
const updateProgressText = $("#updateProgressText");
const updateStatus = $("#updateStatus");

// === Init ===
async function init() {
  try {
    myName = await invoke("get_device_name");
    $("#myDeviceName").textContent = myName;
    $("#settingsDeviceName").textContent = myName;
  } catch {
    $("#myDeviceName").textContent = "Unknown";
  }

  try {
    const version: string = await invoke("get_app_version");
    $("#appVersion").textContent = `v${version}`;
  } catch {
    $("#appVersion").textContent = "v0.1.0";
  }

  await refreshDevices();
  setupEventListeners();
  startDiscoveryLoop();
}

// === Discovery ===
async function refreshDevices() {
  try {
    devices = await invoke("get_devices");
    renderDevices();
  } catch (e) {
    console.error("Failed to get devices:", e);
  }
}

function startDiscoveryLoop() {
  setInterval(refreshDevices, 3000);
}

// === Render devices ===
function renderDevices() {
  if (devices.length === 0) {
    emptyDevices.style.display = "block";
    deviceList.querySelectorAll(".device-item").forEach((el) => el.remove());
    return;
  }

  emptyDevices.style.display = "none";
  deviceList.querySelectorAll(".device-item").forEach((el) => el.remove());

  devices.forEach((d) => {
    const el = document.createElement("div");
    el.className = "device-item" + (selectedDevice?.id === d.id ? " selected" : "");
    el.innerHTML = `
      <div class="device-icon">${getOsIcon(d.os)}</div>
      <div class="device-info">
        <div class="name">${escHtml(d.name)}</div>
        <div class="ip">${d.ip}</div>
      </div>
    `;
    el.addEventListener("click", () => selectDevice(d));
    deviceList.appendChild(el);
  });
}

function getOsIcon(os: string): string {
  if (os.includes("macos") || os.includes("darwin")) return "🍎";
  if (os.includes("windows")) return "🪟";
  if (os.includes("linux")) return "🐧";
  return "💻";
}

function escHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

// === Select device ===
function selectDevice(d: Device) {
  selectedDevice = selectedDevice?.id === d.id ? null : d;
  renderDevices();
  updateSendButton();
}

// === File selection ===
async function openFilePicker() {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({ multiple: true, directory: false });
    if (selected) {
      const paths = Array.isArray(selected) ? selected : [selected];
      await addFiles(paths);
    }
  } catch (e) {
    console.error("Tauri dialog failed, falling back:", e);
    fileInput.click();
  }
}

// dropZone 点击 = 打开文件选择；按钮阻止冒泡，避免双弹窗
dropZone.addEventListener("click", () => openFilePicker());

$("#btnSelectFile").addEventListener("click", (e) => {
  e.stopPropagation();
  openFilePicker();
});

fileInput.addEventListener("change", async () => {
  if (fileInput.files) {
    const paths = Array.from(fileInput.files).map((f) => (f as any).path || f.name);
    await addFiles(paths);
    fileInput.value = "";
  }
});

async function addFiles(paths: string[]) {
  for (const p of paths) {
    try {
      const info: FileEntry = await invoke("get_file_info", { path: p });
      if (!selectedFiles.find((f) => f.path === info.path)) {
        selectedFiles.push(info);
      }
    } catch (e) {
      showToast(`无法读取文件: ${p}`, "error");
    }
  }
  renderSelectedFiles();
}

function renderSelectedFiles() {
  if (selectedFiles.length === 0) {
    selectedFilesEl.style.display = "none";
    return;
  }
  selectedFilesEl.style.display = "block";
  fileList.innerHTML = "";

  selectedFiles.forEach((f, i) => {
    const el = document.createElement("div");
    el.className = "file-item";
    el.innerHTML = `
      <span class="file-icon">📄</span>
      <span class="file-name">${escHtml(f.name)}</span>
      <span class="file-size">${formatSize(f.size)}</span>
      <button class="btn-remove" data-idx="${i}" title="移除">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
        </svg>
      </button>
    `;
    fileList.appendChild(el);
  });

  fileList.querySelectorAll(".btn-remove").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      const idx = parseInt((e.currentTarget as HTMLElement).dataset.idx || "0");
      selectedFiles.splice(idx, 1);
      renderSelectedFiles();
      updateSendButton();
    });
  });

  updateSendButton();
}

function updateSendButton() {
  btnSend.disabled = !selectedDevice || selectedFiles.length === 0;
}

// === Send files ===
btnSend.addEventListener("click", async () => {
  if (!selectedDevice || selectedFiles.length === 0) return;
  try {
    await invoke("send_files", {
      targetIp: selectedDevice.ip,
      targetPort: selectedDevice.port,
      files: selectedFiles.map((f) => f.path),
    });
    showToast("文件发送请求已发出", "success");
  } catch (e) {
    showToast(`发送失败: ${e}`, "error");
  }
});

// === Receive modal ===
let currentTransferId = "";

async function showReceiveModal(req: TransferRequest) {
  currentTransferId = req.transfer_id;
  receiveInfo.innerHTML = `
    <div class="from">来自 <strong>${escHtml(req.from_name)}</strong></div>
    <div class="files">
      ${req.files
        .map(
          (f) =>
            `<div class="file-row"><span>${escHtml(f.name)}</span><span>${formatSize(f.size)}</span></div>`
        )
        .join("")}
    </div>
  `;
  receiveModal.style.display = "flex";
}

$("#btnAccept").addEventListener("click", async () => {
  receiveModal.style.display = "none";
  try {
    await invoke("accept_transfer", { transferId: currentTransferId });
  } catch (e) {
    showToast(`接受失败: ${e}`, "error");
  }
});

$("#btnReject").addEventListener("click", async () => {
  receiveModal.style.display = "none";
  try {
    await invoke("reject_transfer", { transferId: currentTransferId });
  } catch {}
});

// === Settings ===
$("#btnSettings").addEventListener("click", () => {
  settingsOverlay.style.display = "block";
});

$("#btnCloseSettings").addEventListener("click", () => {
  settingsOverlay.style.display = "none";
});

settingsOverlay.addEventListener("click", (e) => {
  if (e.target === settingsOverlay) {
    settingsOverlay.style.display = "none";
  }
});

// === Update check ===
$("#btnCheckUpdate").addEventListener("click", async () => {
  const btn = $("#btnCheckUpdate") as HTMLButtonElement;
  btn.disabled = true;
  updateStatus.className = "update-status checking";
  updateStatus.textContent = "正在检查更新...";

  try {
    const update = await check();

    if (!update) {
      updateStatus.className = "update-status success";
      updateStatus.textContent = "✓ 当前已是最新版本";
      showToast("当前已是最新版本", "success");
    } else {
      updateStatus.className = "update-status success";
      updateStatus.textContent = `发现新版本 v${update.version}`;

      // Show update modal
      updateInfo.innerHTML = `
        <div class="version-row">
          <span class="label">当前版本</span>
          <span class="value">v${update.currentVersion}</span>
        </div>
        <div class="version-row">
          <span class="label">最新版本</span>
          <span class="value">v${update.version}</span>
        </div>
        ${update.date ? `<div class="version-row"><span class="label">发布日期</span><span class="value">${new Date(update.date).toLocaleDateString("zh-CN")}</span></div>` : ""}
        ${update.body ? `<div class="notes">${escHtml(update.body)}</div>` : ""}
      `;
      updateModal.style.display = "flex";

      // Setup update buttons
      $("#btnUpdateNow").onclick = async () => {
        ($("#btnUpdateNow") as HTMLButtonElement).disabled = true;
        ($("#btnUpdateLater") as HTMLButtonElement).disabled = true;
        updateProgress.style.display = "block";
        updateProgressFill.style.width = "0%";
        updateProgressText.textContent = "下载中... 0%";

        try {
          let downloaded = 0;
          let contentLength = 0;

          await update.downloadAndInstall((event) => {
            switch (event.event) {
              case "Started":
                contentLength = event.data.contentLength || 0;
                updateProgressText.textContent = "开始下载...";
                break;
              case "Progress":
                downloaded += event.data.chunkLength;
                if (contentLength > 0) {
                  const pct = Math.round((downloaded / contentLength) * 100);
                  updateProgressFill.style.width = `${pct}%`;
                  updateProgressText.textContent = `下载中... ${pct}%`;
                } else {
                  updateProgressText.textContent = `已下载 ${formatSize(downloaded)}`;
                }
                break;
              case "Finished":
                updateProgressFill.style.width = "100%";
                updateProgressText.textContent = "安装完成，即将重启...";
                break;
            }
          });

          // Relaunch after install
          setTimeout(async () => {
            await relaunch();
          }, 1000);
        } catch (e) {
          updateProgressText.textContent = `更新失败: ${e}`;
          ($("#btnUpdateNow") as HTMLButtonElement).disabled = false;
          ($("#btnUpdateLater") as HTMLButtonElement).disabled = false;
        }
      };

      $("#btnUpdateLater").onclick = () => {
        updateModal.style.display = "none";
        update.close();
      };
    }
  } catch (e: any) {
    updateStatus.className = "update-status error";
    if (e?.message?.includes("Could not fetch")) {
      updateStatus.textContent = "✗ 无法连接更新服务器";
    } else {
      updateStatus.textContent = `✗ 检查失败: ${e?.message || e}`;
    }
    showToast("检查更新失败", "error");
  } finally {
    btn.disabled = false;
  }
});

// === Transfer progress ===
function addTransferItem(id: string, name: string, detail: string, status: string) {
  transfersEl.style.display = "block";
  const el = document.createElement("div");
  el.className = "transfer-item";
  el.id = `transfer-${id}`;
  el.innerHTML = `
    <span class="transfer-icon">${status === "sending" ? "📤" : "📥"}</span>
    <div class="transfer-info">
      <div class="transfer-name">${escHtml(name)}</div>
      <div class="transfer-detail">${escHtml(detail)}</div>
      <div class="progress-bar"><div class="progress-fill" style="width:0%"></div></div>
    </div>
    <span class="transfer-status ${status}">${status === "sending" ? "发送中" : "接收中"}</span>
  `;
  transferList.prepend(el);
}

function updateTransferItem(id: string, progress: number, status: string, statusText: string) {
  const el = document.getElementById(`transfer-${id}`);
  if (!el) return;
  const fill = el.querySelector(".progress-fill") as HTMLElement;
  const statusEl = el.querySelector(".transfer-status") as HTMLElement;
  fill.style.width = `${progress}%`;
  statusEl.textContent = statusText;
  statusEl.className = `transfer-status ${status}`;
}

// === Toast ===
function showToast(msg: string, type: "success" | "error" = "success") {
  toast.textContent = msg;
  toast.className = `toast show ${type}`;
  setTimeout(() => toast.classList.remove("show"), 3000);
}

// === Format helpers ===
function formatSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + " " + sizes[i];
}

// === Event listeners ===
function setupEventListeners() {
  listen<TransferRequest>("transfer-request", (e) => {
    showReceiveModal(e.payload);
  });

  listen<{ id: string; progress: number; status: string; name: string; detail: string }>(
    "transfer-progress",
    (e) => {
      const { id, progress, status, name, detail } = e.payload;
      if (!document.getElementById(`transfer-${id}`)) {
        addTransferItem(id, name, detail, status);
      }
      updateTransferItem(id, progress, status, status === "done" ? "已完成" : status === "error" ? "失败" : `${Math.round(progress)}%`);
    }
  );

  $("#btnRefresh").addEventListener("click", refreshDevices);

  // Tauri 原生文件拖放（HTML5 drop 在 Tauri webview 中不触发）
  listen<{ paths: string[] }>("tauri://drag-drop", async (e) => {
    dropZone.classList.remove("dragover");
    await addFiles(e.payload.paths);
  });
  listen("tauri://drag-enter", () => dropZone.classList.add("dragover"));
  listen("tauri://drag-leave", () => dropZone.classList.remove("dragover"));
}

// === Start ===
init();
