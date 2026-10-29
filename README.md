# LanDrop

轻量级局域网文件传输桌面应用，支持 macOS 和 Windows。

## 特性

- 🔍 自动发现局域网设备（UDP 广播）
- 📤 拖拽发送文件/文件夹
- 📥 接收确认 + 进度显示
- 🔄 自动更新（GitHub Releases）
- 🌙 深色主题
- 📦 超小体积（~12MB）

## 技术栈

- **前端**: HTML/TypeScript + Vite
- **后端**: Rust + Tauri v2
- **发现**: UDP 广播（端口 45678）
- **传输**: TCP 直连（端口 45679）

## 开发

```bash
# 安装依赖
pnpm install

# 开发模式
cargo tauri dev

# 构建
cargo tauri build

# 构建并签名更新包
TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/landrop.key) \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="your-password" \
cargo tauri build
```

## 下载

从 [Releases](https://github.com/HarmmerRay/lan-drop/releases) 下载最新版本。

## License

MIT
