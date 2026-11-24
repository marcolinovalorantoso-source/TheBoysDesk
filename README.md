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

## Credits
- Created by **Marco Venditti** (`mark09venditti@gmail.com` • Discord: `cicciobaudo`)
- With help from **LatiosJet**
- Licensed under the [MIT License](LICENSE).
