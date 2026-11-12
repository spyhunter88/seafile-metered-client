# FEATURE SPECIFICATION: Seafile Smart Metered Client (Windows)

## Overview
Lightweight custom Seafile desktop client for Windows built with Tauri (Rust + React). Main purpose is to automatically pause/resume file synchronization per library based on Windows Network Metered status (Mobile Hotspot) while keeping idle CPU and RAM usage minimal.

---

## 1. Core Features

### 1.1 Account & Server Management
- **Login Screen:** Input Server URL, Email, Password.
- **Authentication:** Authenticate via Seafile REST API (`POST /api2/auth-token/`) and retrieve Auth Token.
- **Credential Storage:** Securely store Auth Token and Server URL in encrypted local state.
- **Auto-login:** Automatic session restoration on app startup.

### 1.2 Library Management & Synchronization
- **Fetch Libraries:** Display all cloud libraries fetched via `GET /api2/repos/`.
- **Local Mapping:** Allow users to bind any cloud library to a specific local folder path.
- **Manual Sync Control:** Toggle Sync/Pause per library manually from the UI.
- **Status Badges:** Display current library state (`Synced`, `Syncing`, `Paused (Metered)`, `Paused (Manual)`, `Error`).

### 1.3 Smart Network Awareness (Metered Connection Handling)
- **Automatic OS Detection:** Event-driven detection of Windows Network Connection Cost (`INetworkListManager`).
- **Per-Library Metered Bypass:** Option to check/uncheck `Bypass Metered` for individual libraries.
  - If `Bypass Metered == true`: Library syncs normally regardless of network type (ideal for lightweight Word/Text document folders).
  - If `Bypass Metered == false`: Library automatically pauses upon entering a Metered Network (Hotspot) and resumes when switching back to Unmetered Network (Wi-Fi/Ethernet).
- **Force Sync Action:** Allow users to force-sync a paused library on a metered network for one-time manual sync.

### 1.4 Background Performance & System Tray
- **Background Daemon:** Runs silently in the Windows System Tray.
- **Tray Menu Options:** Open UI, Pause All, Resume All, Network Status Indicator, Exit.
- **Zero-Idle CPU:** Event-driven architecture ensures 0% CPU consumption when no active sync or network event is occurring.
- **Sidecar Process Control:** Automatic lifecycle management (Spawn/Kill) of the underlying `seaf-daemon.exe`.

---

## 2. User Interface (UI/UX) Requirements
- Clean, compact UI using TailwindCSS.
- Real-time status update for network connection (`Unmetered` vs `Metered Hotspot`).
- Toast notifications when libraries are automatically paused due to network changes.