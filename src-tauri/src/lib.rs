use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// === Constants ===
const DISCOVERY_PORT: u16 = 45678;
const TRANSFER_PORT: u16 = 45679;
const BROADCAST_ADDR: &str = "255.255.255.255";
const DISCOVERY_INTERVAL_SECS: u64 = 3;
const DEVICE_TIMEOUT_SECS: u64 = 10;
const CHUNK_SIZE: usize = 64 * 1024; // 64KB chunks

// === Types ===
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Device {
    id: String,
    name: String,
    ip: String,
    port: u16,
    os: String,
    #[serde(skip)]
    last_seen: Option<Instant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiscoveryMessage {
    id: String,
    name: String,
    port: u16,
    os: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileMeta {
    name: String,
    size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TransferRequest {
    from_id: String,
    from_name: String,
    files: Vec<FileMeta>,
    transfer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TransferResponse {
    transfer_id: String,
    accepted: bool,
}

#[derive(Debug, Clone, Serialize)]
struct FileInfo {
    path: String,
    name: String,
    size: u64,
}

#[derive(Debug, Clone, Serialize)]
struct TransferProgress {
    id: String,
    progress: f64,
    status: String,
    name: String,
    detail: String,
}

// === App State ===
struct AppState {
    device_id: String,
    device_name: String,
    devices: Arc<Mutex<HashMap<String, Device>>>,
    pending_transfers: Arc<Mutex<HashMap<String, PendingTransfer>>>,
}

struct PendingTransfer {
    #[allow(dead_code)]
    from_name: String,
    #[allow(dead_code)]
    files: Vec<FileMeta>,
    tx: tokio::sync::oneshot::Sender<bool>,
}

// === Tauri Commands ===

#[tauri::command]
fn get_device_name(state: State<'_, Arc<AppState>>) -> String {
    state.device_name.clone()
}

#[tauri::command]
fn get_devices(state: State<'_, Arc<AppState>>) -> Vec<Device> {
    let devices = state.devices.lock().unwrap();
    devices.values().cloned().collect()
}

#[tauri::command]
fn get_file_info(path: String) -> Result<FileInfo, String> {
    let p = Path::new(&path);
    let metadata = std::fs::metadata(p).map_err(|e| format!("Cannot read file: {}", e))?;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    Ok(FileInfo {
        path: path.clone(),
        name,
        size: metadata.len(),
    })
}

#[tauri::command]
async fn send_files(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    target_ip: String,
    target_port: u16,
    files: Vec<String>,
) -> Result<(), String> {
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let my_id = state.device_id.clone();
    let my_name = state.device_name.clone();

    // Collect file metadata
    let mut file_metas = Vec::new();
    for f in &files {
        let p = Path::new(f);
        let meta = std::fs::metadata(p).map_err(|e| format!("Cannot read {}: {}", f, e))?;
        file_metas.push(FileMeta {
            name: p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            size: meta.len(),
        });
    }

    let file_names: Vec<String> = file_metas.iter().map(|f| f.name.clone()).collect();
    let total_size: u64 = file_metas.iter().map(|f| f.size).sum();

    // Emit initial progress
    let _ = app.emit(
        "transfer-progress",
        TransferProgress {
            id: transfer_id.clone(),
            progress: 0.0,
            status: "sending".into(),
            name: file_names.join(", "),
            detail: format!("{} 个文件, {}", file_metas.len(), format_size(total_size)),
        },
    );

    // Spawn async send task
    let app_clone = app.clone();
    let tid = transfer_id.clone();
    tokio::spawn(async move {
        match do_send(
            &app_clone,
            &tid,
            &target_ip,
            target_port,
            &my_id,
            &my_name,
            &files,
            &file_metas,
        )
        .await
        {
            Ok(_) => {
                let _ = app_clone.emit(
                    "transfer-progress",
                    TransferProgress {
                        id: tid,
                        progress: 100.0,
                        status: "done".into(),
                        name: file_names.join(", "),
                        detail: "传输完成".into(),
                    },
                );
            }
            Err(e) => {
                let _ = app_clone.emit(
                    "transfer-progress",
                    TransferProgress {
                        id: tid,
                        progress: 0.0,
                        status: "error".into(),
                        name: file_names.join(", "),
                        detail: format!("发送失败: {}", e),
                    },
                );
            }
        }
    });

    Ok(())
}

async fn do_send(
    app: &AppHandle,
    transfer_id: &str,
    target_ip: &str,
    target_port: u16,
    my_id: &str,
    my_name: &str,
    files: &[String],
    file_metas: &[FileMeta],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("{}:{}", target_ip, target_port);
    let mut stream = TcpStream::connect(&addr).await?;

    // Send transfer request
    let request = TransferRequest {
        from_id: my_id.to_string(),
        from_name: my_name.to_string(),
        files: file_metas.to_vec(),
        transfer_id: transfer_id.to_string(),
    };
    let req_json = serde_json::to_string(&request)?;
    let req_bytes = req_json.as_bytes();
    stream.write_u32(req_bytes.len() as u32).await?;
    stream.write_all(req_bytes).await?;

    // Wait for response
    let resp_len = stream.read_u32().await? as usize;
    let mut resp_buf = vec![0u8; resp_len];
    stream.read_exact(&mut resp_buf).await?;
    let response: TransferResponse = serde_json::from_slice(&resp_buf)?;

    if !response.accepted {
        return Err("对方拒绝了传输".into());
    }

    // Send files
    let total_size: u64 = file_metas.iter().map(|f| f.size).sum();
    let mut sent: u64 = 0;

    for (i, file_path) in files.iter().enumerate() {
        let meta = &file_metas[i];
        let mut file = tokio::fs::File::open(file_path).await?;
        let mut buf = vec![0u8; CHUNK_SIZE];

        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            stream.write_all(&buf[..n]).await?;
            sent += n as u64;

            let progress = (sent as f64 / total_size as f64) * 100.0;
            let _ = app.emit(
                "transfer-progress",
                TransferProgress {
                    id: transfer_id.to_string(),
                    progress,
                    status: "sending".into(),
                    name: meta.name.clone(),
                    detail: format!(
                        "{} / {}",
                        format_size(sent),
                        format_size(total_size)
                    ),
                },
            );
        }
    }

    Ok(())
}

#[tauri::command]
async fn accept_transfer(
    state: State<'_, Arc<AppState>>,
    transfer_id: String,
) -> Result<(), String> {
    let mut pending = state.pending_transfers.lock().unwrap();
    if let Some(pt) = pending.remove(&transfer_id) {
        let _ = pt.tx.send(true);
        Ok(())
    } else {
        Err("Transfer not found".into())
    }
}

#[tauri::command]
async fn reject_transfer(
    state: State<'_, Arc<AppState>>,
    transfer_id: String,
) -> Result<(), String> {
    let mut pending = state.pending_transfers.lock().unwrap();
    if let Some(pt) = pending.remove(&transfer_id) {
        let _ = pt.tx.send(false);
        Ok(())
    } else {
        Err("Transfer not found".into())
    }
}

// === Discovery Service ===

async fn run_discovery(state: Arc<AppState>) {
    let socket = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to bind discovery socket: {}", e);
            return;
        }
    };

    socket.set_broadcast(true).ok();
    socket.set_nonblocking(true).ok();

    let msg = DiscoveryMessage {
        id: state.device_id.clone(),
        name: state.device_name.clone(),
        port: TRANSFER_PORT,
        os: std::env::consts::OS.to_string(),
    };
    let msg_json = serde_json::to_string(&msg).unwrap();

    // Spawn broadcaster
    let broadcast_socket = socket.try_clone().unwrap();
    let msg_clone = msg_json.clone();
    tokio::spawn(async move {
        loop {
            let addr = format!("{}:{}", BROADCAST_ADDR, DISCOVERY_PORT);
            let _ = broadcast_socket.send_to(msg_clone.as_bytes(), &addr);
            tokio::time::sleep(Duration::from_secs(DISCOVERY_INTERVAL_SECS)).await;
        }
    });

    // Listen for other devices
    let mut buf = [0u8; 1024];
    loop {
        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                if let Ok(dmsg) = serde_json::from_slice::<DiscoveryMessage>(&buf[..len]) {
                    // Skip self
                    if dmsg.id == state.device_id {
                        continue;
                    }

                    let ip = match src.ip() {
                        std::net::IpAddr::V4(v4) => v4.to_string(),
                        std::net::IpAddr::V6(v6) => v6.to_string(),
                    };

                    let mut devices = state.devices.lock().unwrap();
                    devices.insert(
                        dmsg.id.clone(),
                        Device {
                            id: dmsg.id,
                            name: dmsg.name,
                            ip,
                            port: dmsg.port,
                            os: dmsg.os,
                            last_seen: Some(Instant::now()),
                        },
                    );
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No data, sleep a bit
                tokio::time::sleep(Duration::from_millis(500)).await;

                // Cleanup stale devices
                let mut devices = state.devices.lock().unwrap();
                devices.retain(|_, d| {
                    d.last_seen
                        .map(|t| t.elapsed() < Duration::from_secs(DEVICE_TIMEOUT_SECS))
                        .unwrap_or(false)
                });
            }
            Err(e) => {
                log::error!("Discovery recv error: {}", e);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

// === Transfer Server ===

async fn run_server(app: AppHandle, state: Arc<AppState>) {
    let listener = match TcpListener::bind(("0.0.0.0", TRANSFER_PORT)).await {
        Ok(l) => l,
        Err(e) => {
            log::error!("Failed to bind transfer server: {}", e);
            return;
        }
    };

    log::info!("Transfer server listening on port {}", TRANSFER_PORT);

    loop {
        match listener.accept().await {
            Ok((socket, addr)) => {
                let app_clone = app.clone();
                let state_clone = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(socket, addr, app_clone, state_clone).await {
                        log::error!("Connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                log::error!("Accept error: {}", e);
            }
        }
    }
}

async fn handle_connection(
    mut socket: TcpStream,
    addr: SocketAddr,
    app: AppHandle,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Incoming connection from {}", addr);

    // Read transfer request
    let req_len = socket.read_u32().await? as usize;
    let mut req_buf = vec![0u8; req_len];
    socket.read_exact(&mut req_buf).await?;
    let request: TransferRequest = serde_json::from_slice(&req_buf)?;

    let transfer_id = request.transfer_id.clone();
    let from_name = request.from_name.clone();
    let files = request.files.clone();
    let total_size: u64 = files.iter().map(|f| f.size).sum();

    // Notify frontend
    let _ = app.emit("transfer-request", &request);

    // Wait for user decision
    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    {
        let mut pending = state.pending_transfers.lock().unwrap();
        pending.insert(
            transfer_id.clone(),
            PendingTransfer {
                from_name: from_name.clone(),
                files: files.clone(),
                tx,
            },
        );
    }

    let accepted = match tokio::time::timeout(Duration::from_secs(60), rx).await {
        Ok(Ok(v)) => v,
        _ => false, // Timeout = reject
    };

    // Send response
    let response = TransferResponse {
        transfer_id: transfer_id.clone(),
        accepted,
    };
    let resp_json = serde_json::to_string(&response)?;
    socket.write_u32(resp_json.len() as u32).await?;
    socket.write_all(resp_json.as_bytes()).await?;

    if !accepted {
        return Ok(());
    }

    // Receive files
    let download_dir = get_download_dir();
    std::fs::create_dir_all(&download_dir)?;

    let mut received: u64 = 0;
    let file_names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();

    let _ = app.emit(
        "transfer-progress",
        TransferProgress {
            id: transfer_id.clone(),
            progress: 0.0,
            status: "receiving".into(),
            name: file_names.join(", "),
            detail: format!("来自 {}, {}", from_name, format_size(total_size)),
        },
    );

    for file_meta in &files {
        let file_path = download_dir.join(&file_meta.name);
        let mut file = tokio::fs::File::create(&file_path).await?;
        let mut remaining = file_meta.size;
        let mut buf = vec![0u8; CHUNK_SIZE];

        while remaining > 0 {
            let to_read = std::cmp::min(remaining as usize, CHUNK_SIZE);
            let n = socket.read_exact(&mut buf[..to_read]).await?;
            file.write_all(&buf[..n]).await?;
            remaining -= n as u64;
            received += n as u64;

            let progress = (received as f64 / total_size as f64) * 100.0;
            let _ = app.emit(
                "transfer-progress",
                TransferProgress {
                    id: transfer_id.clone(),
                    progress,
                    status: "receiving".into(),
                    name: file_meta.name.clone(),
                    detail: format!(
                        "{} / {}",
                        format_size(received),
                        format_size(total_size)
                    ),
                },
            );
        }
    }

    let _ = app.emit(
        "transfer-progress",
        TransferProgress {
            id: transfer_id,
            progress: 100.0,
            status: "done".into(),
            name: file_names.join(", "),
            detail: format!("已保存到 {}", download_dir.display()),
        },
    );

    log::info!("Transfer completed from {}", addr);
    Ok(())
}

// === Helpers ===

fn get_download_dir() -> std::path::PathBuf {
    // Save to ~/Downloads/LanDrop/
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join("Downloads").join("LanDrop")
}

fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".into();
    }
    let k = 1024u64;
    let sizes = ["B", "KB", "MB", "GB"];
    let i = (bytes as f64).log(k as f64).floor() as usize;
    let i = i.min(sizes.len() - 1);
    format!("{:.1} {}", bytes as f64 / k.pow(i as u32) as f64, sizes[i])
}

fn get_device_id() -> String {
    // Generate a stable device ID
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let ip = local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "0.0.0.0".to_string());
    format!("{}-{}", hostname, ip)
}

fn get_local_ip() -> String {
    local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

// === Entry Point ===

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let device_id = get_device_id();
    let device_name = hostname::get()
        .map(|h| h.to_string_lossy().trim_end_matches(".local").to_string())
        .unwrap_or_else(|_| "Unknown".to_string());
    let local_ip = get_local_ip();

    let state = Arc::new(AppState {
        device_id: device_id.clone(),
        device_name: device_name.clone(),
        devices: Arc::new(Mutex::new(HashMap::new())),
        pending_transfers: Arc::new(Mutex::new(HashMap::new())),
    });

    log::info!(
        "Starting LanDrop: {} ({}) at {}",
        device_name,
        device_id,
        local_ip
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(state.clone())
        .setup(move |app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let app_handle = app.handle().clone();
            let state_clone = state.clone();

            // Start discovery service
            let state_disc = state_clone.clone();
            tauri::async_runtime::spawn(async move {
                run_discovery(state_disc).await;
            });

            // Start transfer server
            tauri::async_runtime::spawn(async move {
                run_server(app_handle, state_clone).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_device_name,
            get_devices,
            get_file_info,
            send_files,
            accept_transfer,
            reject_transfer,
        ])
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}
