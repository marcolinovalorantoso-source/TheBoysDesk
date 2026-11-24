**Language:** English | [Italiano](README.it.md)

# TheBoysDesk

Lightweight, fast, and secure **peer-to-peer remote desktop** application for Windows 10/11 written in **Rust** — zero central servers, end-to-end encrypted via **QUIC (TLS 1.3)**, **DXGI Desktop Duplication**, **OpenH264**, and native **`egui`** UI with instant **EN / IT** language switching.

## Key Features
- **Pure Peer-to-Peer (No Central Servers):** Automatic LAN discovery, automatic router port mapping via **UPnP (IGD)**, public IP discovery via **STUN**, and direct QUIC connection.
- **Security First:**
  - TLS 1.3 encryption with **TLS channel-bound PIN verification** (protects against MitM certificate substitution).
  - Brute-force PIN rate limiting (60s lockout after 5 failed attempts).
  - Explicit manual consent popup on the host before any session starts.
  - **Mouse and keyboard start disabled by default** (view-only) on every new session, with visible on-screen toggles and shortcuts (`Ctrl+Alt+M`, `Ctrl+Alt+K`, `Ctrl+Alt+B`, `Ctrl+Alt+D`).
- **Low-Latency Remote Control:** DXGI capture, OpenH264 real-time encoding, multi-monitor selector (`M1`, `M2`, ...), live FPS & RTT (`ms`) indicator, and hardware scan-code input injection (`SendInput`).
- **Instant Language Switch:** English by default with a 1-click **EN / IT** toggle in the header.
- **AI Agent Team Included:** Workspace skills in `.agents/skills/team-*` (`team-lead`, `team-scout`, `team-architect`, `team-builder`, `team-checker`) with quality & weekly improvement loops in `team/`.

## Quick Start (3 Steps)

### Option A — Download Portable Binary
1. Download **`TheBoysDesk.exe`** from the [v0.1.0 Release](https://github.com/marcolinovalorantoso-source/TheBoysDesk/releases/tag/v0.1.0).
2. Run `TheBoysDesk.exe` on both Windows PCs (no installation required; allow Windows Firewall prompt on first launch).
3. Share your LAN or Internet `IP:Port` and 6-digit `Session PIN`, click **Connect**, and accept the prompt on the host.

### Option B — Build from Source
1. Run the workspace unit tests:
   ```powershell
   cargo test --workspace
   ```
2. Build the release executable:
   ```powershell
   cargo build --release -p desktop
   .\target\release\TheBoysDesk.exe
   ```
3. *(Optional)* Verify the AI Agent Team setup:
   ```powershell
   powershell -ExecutionPolicy Bypass -File team\run_team.ps1 -Verify -WeeklyReview
   ```

## Requirements & Troubleshooting (Windows)
- **Zero External DLLs Required (100% Standalone):** `TheBoysDesk.exe` statically links the C++ runtime (`libstdc++` / `libgcc`) and uses only built-in Windows DLLs (`KERNEL32`, `USER32`, `D3D11`, `DXGI`, `OPENGL32`, `msvcrt.dll`). If an earlier build failed with *"libstdc++-6.dll was not found"*, re-download the latest `TheBoysDesk.exe` from [Release v0.1.0](https://github.com/marcolinovalorantoso-source/TheBoysDesk/releases/tag/v0.1.0).
- **System Requirements:** Windows 10 or Windows 11 (64-bit) with up-to-date GPU drivers supporting **OpenGL 2.0+** and **DirectX 11** (needed for the `egui` window and DXGI screen capture).
- **Windows SmartScreen ("Windows protected your PC"):** Because the binary is unsigned, click **"More info" → "Run anyway"** on first launch.
- **Older Windows 10 Systems (Optional):** If Windows reports a missing `api-ms-win-crt-*` / Universal CRT library on an unpatched machine, install Microsoft's official [Visual C++ Redistributable x64 (`vc_redist.x64.exe`)](https://aka.ms/vs/17/release/vc_redist.x64.exe).
- **Firewall & Router:** Allow `TheBoysDesk.exe` on Private & Public networks in Windows Firewall. For connections outside your LAN, enable **UPnP** on your router or forward UDP port **`21116`**.

## Credits
- Created by **Marco Venditti** (`mark09venditti@gmail.com` • Discord: `cicciobaudo`)
- With help from **LatiosJet**
- Licensed under the [MIT License](LICENSE).
