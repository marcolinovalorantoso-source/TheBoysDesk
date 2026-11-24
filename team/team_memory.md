# Team Memory — Feature & Quality Team

> Il Lead legge questo file prima di ogni run e lo aggiorna solo con lezioni verificate (max 50 righe).

## Contesto Progetto
- Workspace principale: `AnydeskPORTING` (Rust P2P Remote Desktop per Windows 10/11).
- Toolchain verificata: `rustc 1.100.0` su target `x86_64-pc-windows-gnu` (WinLibs MinGW-w64 UCRT).
- Vincolo critico Windows: PowerShell 5.1 non supporta `&&` (usare `; if ($LASTEXITCODE -eq 0) { ... }`).
- Vincolo critico Rustls/Quinn su GNU: usare sempre `ring` (disabilitare `aws-lc-rs`).

## Regole di Sicurezza & UX Consolidate
- Nessun server centrale: P2P puro (QUIC TLS 1.3 + UPnP + STUN + LAN Discovery).
- Mouse e Tastiera sull'Host devono partire sempre `OFF` (modalità solo vista) all'avvio di ogni sessione.
- Channel binding TLS obbligatorio su `pin_proof` (`SHA-256(nonce || host_cert_sha256 || pin)`).
- Limiti frame: `Control` max 64 KiB, `Input` max 4 KiB, `Video` max 16 MiB.
- Non sovrascrivere `target/release/desktop.exe` se l'app è aperta (errore OS 5).

## Metrica Settimanale
- KPI unico: `first_pass_rate` = `% di task approvati al 1° tentativo senza correzioni manuali`.
