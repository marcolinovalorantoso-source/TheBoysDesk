# TheBoysDesk + Feature & Quality AI Agent Team

Applicazione desktop P2P in **Rust** per il controllo remoto su Windows (zero server centrali, QUIC TLS 1.3, DXGI + OpenH264, `egui`) affiancata dal tuo **Team di 5 Agenti AI** (`.agents/skills/team-*`) con grafo parallelo, loop di qualità (max 2 round) e loop di miglioramento settimanale.

## Cosa risolve
1. **Controllo remoto P2P privato e sicuro** tra amici senza server intermedi, con PIN + channel binding TLS, accettazione esplicita e mouse/tastiera bloccati di default.
2. **Sviluppo assistito da un vero team AI**: automatizza analisi (`Scout`), design/sicurezza (`Architect`), implementazione+test (`Builder`) e review indipendente (`Checker`) sotto un unico `Lead`.

## Installazione e Avvio in 3 Step
1. **Compila ed esegui l'app desktop:**
   ```powershell
   cargo test --workspace
   cargo build --release -p desktop
   .\target\release\TheBoysDesk.exe
   ```
2. **Verifica e lancia il Team AI (1 comando):**
   ```powershell
   powershell -ExecutionPolicy Bypass -File team\run_team.ps1 -Verify -WeeklyReview
   ```
3. **Esegui un task reale in Antigravity:**
   Avvia il prompt generato da `team\run_team.ps1 "Descrivi il tuo task"` (attiva la skill `team-lead` che coordina `team-scout`, `team-architect`, `team-builder` e `team-checker`).
