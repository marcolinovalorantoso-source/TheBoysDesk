**Lingua:** [English](README.md) | Italiano

# TheBoysDesk

Applicazione desktop **peer-to-peer per il controllo remoto** su Windows 10/11 scritta in **Rust** — leggera, veloce, senza server centrali, cifrata end-to-end tramite **QUIC (TLS 1.3)**, **DXGI Desktop Duplication**, **OpenH264** e interfaccia nativa **`egui`** con cambio lingua istantaneo **EN / IT**.

## Funzionalità principali
- **Peer-to-Peer Puro (Zero Server Centrali):** Discovery automatica in LAN, apertura porta automatica sul router via **UPnP (IGD)**, rilevamento IP pubblico tramite **STUN** e connessione diretta QUIC.
- **Sicurezza al primo posto:**
  - Cifratura TLS 1.3 con **PIN legato al certificato TLS (channel binding)** contro attacchi MitM.
  - Rate limiting anti brute-force sul PIN (blocco 60s dopo 5 tentativi errati).
  - Popup di consenso esplicito sull'host prima di avviare la sessione.
  - **Mouse e tastiera partono disabilitati di default** (modalità solo vista) ad ogni connessione, con pulsanti visibili e shortcut (`Ctrl+Alt+M`, `Ctrl+Alt+K`, `Ctrl+Alt+B`, `Ctrl+Alt+D`).
- **Controllo remoto a bassa latenza:** Cattura DXGI, codifica OpenH264 real-time, selettore multi-monitor (`M1`, `M2`, ...), FPS e latenza RTT (`ms`) in tempo reale e input tramite scan-code (`SendInput`).
- **Cambio lingua istantaneo:** Inglese di default con switch 1-click **EN / IT** in alto a destra.
- **Team di Agenti AI incluso:** Skill native in `.agents/skills/team-*` (`team-lead`, `team-scout`, `team-architect`, `team-builder`, `team-checker`) con loop di qualità e miglioramento settimanale in `team/`.

## Avvio Rapido (3 Step)

### Opzione A — Scarica l'eseguibile portable
1. Scarica **`TheBoysDesk.exe`** dalla [Release v0.1.0](https://github.com/marcolinovalorantoso-source/TheBoysDesk/releases/tag/v0.1.0).
2. Avvia `TheBoysDesk.exe` su entrambi i PC Windows (nessuna installazione richiesta; consenti Windows Firewall al primo avvio).
3. Condividi l'indirizzo `IP:Porta` (LAN o Internet) e il `PIN di sessione` a 6 cifre, clicca **Connetti** e accetta sul popup remoto.

### Opzione B — Compila dai sorgenti
1. Esegui i test unitari del workspace:
   ```powershell
   cargo test --workspace
   ```
2. Compila la versione release:
   ```powershell
   cargo build --release -p desktop
   .\target\release\TheBoysDesk.exe
   ```
3. *(Opzionale)* Verifica il Team di Agenti AI:
   ```powershell
   powershell -ExecutionPolicy Bypass -File team\run_team.ps1 -Verify -WeeklyReview
   ```

## Requisiti e Risoluzione Problemi (Windows)
- **Eseguibile 100% Standalone (Nessuna DLL da scaricare):** `TheBoysDesk.exe` integra staticamente il runtime C++ (`libstdc++` / `libgcc`) e usa solo librerie di sistema incluse in Windows (`KERNEL32`, `USER32`, `D3D11`, `DXGI`, `OPENGL32`, `msvcrt.dll`). Se una versione precedente dava errore *"libstdc++-6.dll non trovata"*, riscarica l'ultimo `TheBoysDesk.exe` dalla [Release v0.1.0](https://github.com/marcolinovalorantoso-source/TheBoysDesk/releases/tag/v0.1.0).
- **Requisiti di sistema:** Windows 10 o Windows 11 (64-bit) con driver scheda video aggiornati con supporto **OpenGL 2.0+** e **DirectX 11** (necessari per finestra `egui` e cattura DXGI).
- **Windows SmartScreen ("PC protetto da Windows"):** essendo un eseguibile open-source senza firma a pagamento, al primo avvio clicca **"Ulteriori informazioni" → "Esegui comunque"**.
- **Windows 10 molto vecchi (Opzionale):** se un PC non aggiornato dovesse segnalare DLL `api-ms-win-crt-*` mancanti, installa il pacchetto ufficiale Microsoft [Visual C++ Redistributable x64 (`vc_redist.x64.exe`)](https://aka.ms/vs/17/release/vc_redist.x64.exe).
- **Firewall e Router:** consenti `TheBoysDesk.exe` su reti Private e Pubbliche in Windows Firewall. Per connessioni fuori casa via Internet, attiva **UPnP** sul router o apri la porta **`21116/UDP`**.

## Crediti
- Creata da **Marco Venditti** (`mark09venditti@gmail.com` • Discord: `cicciobaudo`)
- Con l'aiuto di **LatiosJet**
- Rilasciata sotto [Licenza MIT](LICENSE).
