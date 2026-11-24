//! Host and Client P2P session orchestration over QUIC.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capture::{list_monitors, CaptureError, DxgiCapturer};
use codec::{DecodedFrame, H264Decoder, H264Encoder};
use crypto::{cert_fingerprint_sha256, compute_pin_proof, generate_nonce, verify_pin_proof};
use input::{is_input_event_permitted, WindowsInputInjector};
use net::{accept_bi_stream, accept_uni_stream, open_bi_stream, open_uni_stream, P2pEndpoint};
use protocol::{
    read_frame, read_frame_limited, write_frame, ControlMessage, InputEvent, MonitorInfo,
    StreamKind, VideoCodec, VideoFrame, MAX_CONTROL_FRAME_SIZE, MAX_INPUT_FRAME_SIZE,
    PROTOCOL_VERSION,
};
use rustls::pki_types::CertificateDer;
use tokio::sync::{mpsc, oneshot, watch, RwLock};
use tracing::{error, info, warn};

/// Pending incoming connection request awaiting explicit user consent on the host.
pub struct PendingConsent {
    pub peer_name: String,
    pub peer_id: String,
    pub peer_addr: SocketAddr,
    pub reply: oneshot::Sender<bool>,
}

/// Tracks failed PIN attempts to block brute-force attacks.
#[derive(Default)]
pub struct PinRateLimiter {
    failed_attempts: u32,
    locked_until: Option<Instant>,
}

impl PinRateLimiter {
    pub fn check_locked(&mut self) -> Option<u64> {
        if let Some(until) = self.locked_until {
            let now = Instant::now();
            if now < until {
                return Some((until - now).as_secs().max(1));
            }
            self.locked_until = None;
            self.failed_attempts = 0;
        }
        None
    }

    pub fn record_failure(&mut self) {
        self.failed_attempts += 1;
        if self.failed_attempts >= 5 {
            self.locked_until = Some(Instant::now() + Duration::from_secs(60));
        }
    }

    pub fn reset(&mut self) {
        self.failed_attempts = 0;
        self.locked_until = None;
    }
}

/// Shared host state observed by the GUI.
pub struct HostRuntimeState {
    pub pin: RwLock<String>,
    pub active_peer: Mutex<Option<String>>,
    pub pending_consent: Mutex<Option<PendingConsent>>,
    pub disconnect_tx: Mutex<Option<watch::Sender<bool>>>,
    pub allow_mouse: Arc<AtomicBool>,
    pub allow_keyboard: Arc<AtomicBool>,
    pub disconnect_notice: Mutex<Option<String>>,
    pub rate_limiter: Mutex<PinRateLimiter>,
}

impl HostRuntimeState {
    pub fn new(initial_pin: String) -> Self {
        Self {
            pin: RwLock::new(initial_pin),
            active_peer: Mutex::new(None),
            pending_consent: Mutex::new(None),
            disconnect_tx: Mutex::new(None),
            allow_mouse: Arc::new(AtomicBool::new(false)),
            allow_keyboard: Arc::new(AtomicBool::new(false)),
            disconnect_notice: Mutex::new(None),
            rate_limiter: Mutex::new(PinRateLimiter::default()),
        }
    }

    pub fn disconnect_active(&self) {
        if let Some(tx) = self.disconnect_tx.lock().unwrap().as_ref() {
            let _ = tx.send(true);
        }
    }
}

/// Runs the host QUIC accept loop in the background.
pub async fn run_host_listener(
    endpoint: Arc<P2pEndpoint>,
    device_id: String,
    host_name: String,
    host_cert_sha256: [u8; 32],
    state: Arc<HostRuntimeState>,
    egui_ctx: egui::Context,
) {
    while let Some(incoming) = endpoint.accept().await {
        let state = state.clone();
        let device_id = device_id.clone();
        let host_name = host_name.clone();
        let ctx = egui_ctx.clone();

        tokio::spawn(async move {
            match tokio::time::timeout(Duration::from_secs(6), incoming).await {
                Ok(Ok(conn)) => {
                    if let Err(e) = handle_host_connection(
                        conn,
                        device_id,
                        host_name,
                        host_cert_sha256,
                        state,
                        ctx,
                    )
                    .await
                    {
                        warn!("Host session ended: {e}");
                    }
                }
                Ok(Err(e)) => warn!("Incoming QUIC handshake failed: {e}"),
                Err(_) => warn!("Incoming QUIC handshake timed out"),
            }
        });
    }
}

fn sanitize_label(s: &str, max_len: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max_len)
        .collect()
}

async fn handle_host_connection(
    conn: quinn::Connection,
    device_id: String,
    host_name: String,
    host_cert_sha256: [u8; 32],
    state: Arc<HostRuntimeState>,
    ctx: egui::Context,
) -> anyhow::Result<()> {
    let remote_addr = conn.remote_address();
    info!("Incoming connection from {remote_addr}");

    let (kind, mut ctrl_send, mut ctrl_recv) =
        tokio::time::timeout(Duration::from_secs(5), accept_bi_stream(&conn))
            .await
            .map_err(|_| anyhow::anyhow!("control stream timeout"))??;
    if kind != StreamKind::Control {
        anyhow::bail!("expected Control stream first");
    }

    let hello: ControlMessage = tokio::time::timeout(
        Duration::from_secs(5),
        read_frame_limited(&mut ctrl_recv, MAX_CONTROL_FRAME_SIZE),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Hello timeout"))??;
    let (client_name, client_id) = match hello {
        ControlMessage::Hello {
            protocol_version,
            client_name,
            device_id,
        } => {
            if protocol_version != PROTOCOL_VERSION {
                write_frame(
                    &mut ctrl_send,
                    &ControlMessage::AuthResult {
                        accepted: false,
                        reason: Some("Protocol version mismatch".into()),
                        monitors: vec![],
                    },
                )
                .await?;
                return Ok(());
            }
            (
                sanitize_label(&client_name, 64),
                sanitize_label(&device_id, 32),
            )
        }
        _ => anyhow::bail!("expected Hello message"),
    };

    let locked_secs = state.rate_limiter.lock().unwrap().check_locked();
    if let Some(wait_secs) = locked_secs {
        write_frame(
            &mut ctrl_send,
            &ControlMessage::AuthResult {
                accepted: false,
                reason: Some(format!(
                    "Troppi tentativi errati. Riprova tra {wait_secs}s"
                )),
                monitors: vec![],
            },
        )
        .await?;
        return Ok(());
    }

    let nonce = generate_nonce();
    write_frame(
        &mut ctrl_send,
        &ControlMessage::AuthChallenge {
            nonce,
            host_name: host_name.clone(),
            device_id,
            requires_pin: true,
            requires_manual_accept: true,
        },
    )
    .await?;

    let auth_resp: ControlMessage = tokio::time::timeout(
        Duration::from_secs(5),
        read_frame_limited(&mut ctrl_recv, MAX_CONTROL_FRAME_SIZE),
    )
    .await
    .map_err(|_| anyhow::anyhow!("AuthResponse timeout"))??;
    let pin_proof = match auth_resp {
        ControlMessage::AuthResponse { pin_proof } => pin_proof,
        _ => anyhow::bail!("expected AuthResponse"),
    };

    let expected_pin = state.pin.read().await.clone();
    if !verify_pin_proof(&nonce, &host_cert_sha256, &expected_pin, &pin_proof) {
        state.rate_limiter.lock().unwrap().record_failure();
        tokio::time::sleep(Duration::from_millis(500)).await;
        write_frame(
            &mut ctrl_send,
            &ControlMessage::AuthResult {
                accepted: false,
                reason: Some("PIN non valido".into()),
                monitors: vec![],
            },
        )
        .await?;
        return Ok(());
    }
    state.rate_limiter.lock().unwrap().reset();

    // Request explicit manual consent from the host user via the GUI.
    let (reply_tx, reply_rx) = oneshot::channel();
    let is_busy = {
        let mut pending = state.pending_consent.lock().unwrap();
        if pending.is_some() || state.active_peer.lock().unwrap().is_some() {
            true
        } else {
            *pending = Some(PendingConsent {
                peer_name: client_name.clone(),
                peer_id: client_id.clone(),
                peer_addr: remote_addr,
                reply: reply_tx,
            });
            false
        }
    };
    if is_busy {
        write_frame(
            &mut ctrl_send,
            &ControlMessage::AuthResult {
                accepted: false,
                reason: Some("Host occupato in un'altra sessione".into()),
                monitors: vec![],
            },
        )
        .await?;
        return Ok(());
    }
    ctx.request_repaint();

    // Notify the client that the connection & PIN succeeded and the popup is now shown.
    write_frame(
        &mut ctrl_send,
        &ControlMessage::AuthPendingConsent { host_name },
    )
    .await?;

    let accepted = tokio::time::timeout(Duration::from_secs(45), reply_rx)
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or(false);

    if !accepted {
        *state.pending_consent.lock().unwrap() = None;
        write_frame(
            &mut ctrl_send,
            &ControlMessage::AuthResult {
                accepted: false,
                reason: Some("Connessione rifiutata dall'utente remoto".into()),
                monitors: vec![],
            },
        )
        .await?;
        ctx.request_repaint();
        return Ok(());
    }

    let monitors = list_monitors().unwrap_or_default();
    write_frame(
        &mut ctrl_send,
        &ControlMessage::AuthResult {
            accepted: true,
            reason: None,
            monitors: monitors.clone(),
        },
    )
    .await?;

    let (disc_tx, mut disc_rx) = watch::channel(false);
    // Start in view-only mode for safety: host user must explicitly enable mouse/keyboard.
    state.allow_mouse.store(false, Ordering::Relaxed);
    state.allow_keyboard.store(false, Ordering::Relaxed);
    *state.active_peer.lock().unwrap() = Some(format!("{client_name} ({remote_addr})"));
    *state.disconnect_tx.lock().unwrap() = Some(disc_tx);
    ctx.request_repaint();

    let initial_monitor = monitors
        .iter()
        .find(|m| m.is_primary)
        .or_else(|| monitors.first())
        .map(|m| m.id)
        .unwrap_or(0);
    let active_monitor = Arc::new(AtomicU32::new(initial_monitor));
    let force_keyframe = Arc::new(AtomicBool::new(true));
    let stop_flag = Arc::new(AtomicBool::new(false));

    // Spawn video capture + H.264 encode thread.
    let video_stream = open_uni_stream(&conn, StreamKind::Video).await?;
    let fk_clone = force_keyframe.clone();
    let stop_clone = stop_flag.clone();
    let mon_clone = active_monitor.clone();
    let rt_handle = tokio::runtime::Handle::current();

    std::thread::spawn(move || {
        run_capture_encode_loop(video_stream, mon_clone, fk_clone, stop_clone, rt_handle);
    });

    // Spawn input receiver task with strict permission gating & release-on-disable.
    let conn_in = conn.clone();
    let stop_in = stop_flag.clone();
    let allow_mouse = state.allow_mouse.clone();
    let allow_keyboard = state.allow_keyboard.clone();
    tokio::spawn(async move {
        let injector = WindowsInputInjector;
        let mut prev_mouse = allow_mouse.load(Ordering::Relaxed);
        let mut prev_kbd = allow_keyboard.load(Ordering::Relaxed);

        while !stop_in.load(Ordering::Relaxed) {
            match accept_uni_stream(&conn_in).await {
                Ok((StreamKind::Input, mut recv)) => {
                    while let Ok(ev) =
                        read_frame_limited::<_, InputEvent>(&mut recv, MAX_INPUT_FRAME_SIZE).await
                    {
                        let mouse_now = allow_mouse.load(Ordering::Relaxed);
                        let kbd_now = allow_keyboard.load(Ordering::Relaxed);
                        if prev_mouse && !mouse_now {
                            injector.release_mouse_buttons();
                        }
                        if prev_kbd && !kbd_now {
                            let _ = injector.inject(&InputEvent::ReleaseAllKeys);
                        }
                        prev_mouse = mouse_now;
                        prev_kbd = kbd_now;

                        if is_input_event_permitted(&ev, mouse_now, kbd_now) {
                            if let Err(e) = injector.inject(&ev) {
                                warn!("Input inject error: {e}");
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        injector.release_all_inputs();
    });

    // Control loop + immediate disconnect watch.
    let end_notice: Option<String>;
    let mut last_keyframe_req = Instant::now() - Duration::from_secs(1);
    let mut last_ping_req = Instant::now() - Duration::from_secs(1);
    let mut last_monitor_switch = Instant::now() - Duration::from_secs(1);
    loop {
        tokio::select! {
            _ = disc_rx.changed() => {
                if *disc_rx.borrow() {
                    let _ = write_frame(&mut ctrl_send, &ControlMessage::Disconnect {
                        reason: "Sei stato disconnesso dall'host".into(),
                    }).await;
                    let _ = ctrl_send.finish();
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    end_notice = None;
                    break;
                }
            }
            msg = read_frame_limited::<_, ControlMessage>(&mut ctrl_recv, MAX_CONTROL_FRAME_SIZE) => {
                match msg {
                    Ok(ControlMessage::SelectMonitor { monitor_id }) => {
                        if last_monitor_switch.elapsed() >= Duration::from_millis(250) {
                            if let Some(valid_id) = validate_monitor_selection(&monitors, monitor_id) {
                                active_monitor.store(valid_id, Ordering::Relaxed);
                                force_keyframe.store(true, Ordering::Relaxed);
                                last_monitor_switch = Instant::now();
                            }
                        }
                    }
                    Ok(ControlMessage::RequestKeyframe) => {
                        if last_keyframe_req.elapsed() >= Duration::from_millis(500) {
                            force_keyframe.store(true, Ordering::Relaxed);
                            last_keyframe_req = Instant::now();
                        }
                    }
                    Ok(ControlMessage::Ping { seq, sent_us }) => {
                        if last_ping_req.elapsed() >= Duration::from_millis(500) {
                            let _ = write_frame(&mut ctrl_send, &ControlMessage::Pong { seq, sent_us }).await;
                            last_ping_req = Instant::now();
                        }
                    }
                    Ok(ControlMessage::Disconnect { .. }) => {
                        end_notice = Some(format!("{client_name} ha terminato la connessione."));
                        break;
                    }
                    Err(_) => {
                        end_notice = Some(format!("{client_name} ha perso la connessione (rete caduta o timeout)."));
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    stop_flag.store(true, Ordering::Relaxed);
    conn.close(0u32.into(), b"session ended");
    *state.active_peer.lock().unwrap() = None;
    *state.disconnect_tx.lock().unwrap() = None;
    if let Some(notice) = end_notice {
        *state.disconnect_notice.lock().unwrap() = Some(notice);
    }
    ctx.request_repaint();
    Ok(())
}

fn run_capture_encode_loop(
    mut stream: quinn::SendStream,
    active_monitor: Arc<AtomicU32>,
    force_keyframe: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    rt: tokio::runtime::Handle,
) {
    let mut current_monitor_id = active_monitor.load(Ordering::Relaxed);
    let mut capturer = match DxgiCapturer::new(current_monitor_id) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to initialize DXGI capture: {e}");
            return;
        }
    };
    let mut encoder = match H264Encoder::new(3_500_000, 30.0) {
        Ok(e) => e,
        Err(e) => {
            error!("Failed to initialize OpenH264 encoder: {e}");
            return;
        }
    };

    let mut frame_id = 0u64;
    let start = Instant::now();
    let frame_interval = Duration::from_millis(33);

    while !stop_flag.load(Ordering::Relaxed) {
        let tick_start = Instant::now();
        let desired = active_monitor.load(Ordering::Relaxed);
        if desired != current_monitor_id {
            match DxgiCapturer::new(desired) {
                Ok(new_cap) => {
                    capturer = new_cap;
                    current_monitor_id = desired;
                    force_keyframe.store(true, Ordering::Relaxed);
                }
                Err(e) => {
                    warn!("Failed to switch to monitor {desired}: {e}");
                    active_monitor.store(current_monitor_id, Ordering::Relaxed);
                }
            }
        }

        match capturer.capture_frame(30) {
            Ok(Some(captured)) => {
                let fk = force_keyframe.swap(false, Ordering::Relaxed) || frame_id == 0;
                match encoder.encode_bgra(captured.width, captured.height, &captured.bgra, fk) {
                    Ok((data, is_keyframe)) => {
                        let pkt = VideoFrame {
                            frame_id,
                            timestamp_us: start.elapsed().as_micros() as u64,
                            monitor_id: current_monitor_id,
                            width: captured.width,
                            height: captured.height,
                            is_keyframe,
                            codec: VideoCodec::H264,
                            data,
                        };
                        frame_id += 1;
                        if rt.block_on(write_frame(&mut stream, &pkt)).is_err() {
                            break;
                        }
                    }
                    Err(e) => warn!("Encode error: {e}"),
                }
            }
            Ok(None) => {}
            Err(CaptureError::AccessLost) => {
                std::thread::sleep(Duration::from_millis(200));
                if let Ok(new_cap) = DxgiCapturer::new(current_monitor_id) {
                    capturer = new_cap;
                    force_keyframe.store(true, Ordering::Relaxed);
                }
            }
            Err(e) => {
                warn!("Capture error: {e}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        let elapsed = tick_start.elapsed();
        if elapsed < frame_interval {
            std::thread::sleep(frame_interval - elapsed);
        }
    }
}

/// Validates that a requested monitor ID exists in the host's monitor list.
pub fn validate_monitor_selection(monitors: &[MonitorInfo], requested_id: u32) -> Option<u32> {
    monitors.iter().find(|m| m.id == requested_id).map(|m| m.id)
}

/// Active client-side remote viewer state shared with `egui`.
pub struct ClientSessionHandle {
    pub remote_addr: SocketAddr,
    pub monitors: Vec<MonitorInfo>,
    pub selected_monitor: u32,
    pub latest_frame: Arc<Mutex<Option<DecodedFrame>>>,
    pub input_tx: mpsc::UnboundedSender<InputEvent>,
    pub control_tx: mpsc::UnboundedSender<ControlMessage>,
    pub status_text: Arc<Mutex<String>>,
    pub fps: Arc<Mutex<f32>>,
    pub rtt_ms: Arc<Mutex<Option<u32>>>,
    pub disconnect_tx: watch::Sender<bool>,
    pub ended_reason: Arc<Mutex<Option<String>>>,
}

/// Computes round-trip latency in milliseconds from monotonic microsecond timestamps.
pub fn compute_rtt_ms(now_us: u64, sent_us: u64) -> Option<u32> {
    if now_us < sent_us {
        return None;
    }
    let diff_ms = (now_us - sent_us) / 1000;
    u32::try_from(diff_ms).ok()
}

/// Connects to a remote host, authenticates with the session PIN, and starts
/// receiving video + sending input.
pub async fn connect_client_session(
    endpoint: Arc<P2pEndpoint>,
    addr: SocketAddr,
    pin: String,
    local_name: String,
    local_id: String,
    ctx: egui::Context,
    progress: Arc<Mutex<Option<String>>>,
) -> anyhow::Result<ClientSessionHandle> {
    let set_progress = |msg: &str| {
        *progress.lock().unwrap() = Some(msg.to_string());
        ctx.request_repaint();
    };

    set_progress(&format!("Connessione QUIC a {addr}..."));
    let conn = tokio::time::timeout(Duration::from_secs(6), endpoint.connect(addr))
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Timeout di connessione (6s): impossibile raggiungere {addr}. Verifica IP, porta UDP o firewall."
            )
        })??;

    // TLS Channel Binding: extract the host's presented certificate SHA-256.
    let host_cert_sha256 = conn
        .peer_identity()
        .and_then(|id| id.downcast::<Vec<CertificateDer<'static>>>().ok())
        .and_then(|certs| certs.into_iter().next())
        .map(|cert| cert_fingerprint_sha256(cert.as_ref()))
        .ok_or_else(|| anyhow::anyhow!("certificato TLS dell'host mancante"))?;

    set_progress("Negoziazione canale di controllo...");
    let (mut ctrl_send, mut ctrl_recv) =
        tokio::time::timeout(Duration::from_secs(5), open_bi_stream(&conn, StreamKind::Control))
            .await
            .map_err(|_| anyhow::anyhow!("Timeout apertura stream di controllo"))??;

    write_frame(
        &mut ctrl_send,
        &ControlMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: local_name,
            device_id: local_id,
        },
    )
    .await?;

    let challenge = match tokio::time::timeout(
        Duration::from_secs(5),
        read_frame_limited::<_, ControlMessage>(&mut ctrl_recv, MAX_CONTROL_FRAME_SIZE),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Timeout attesa challenge dall'host"))??
    {
        ControlMessage::AuthChallenge { nonce, .. } => nonce,
        _ => anyhow::bail!("risposta inattesa dall'host"),
    };

    set_progress("Verifica PIN in corso...");
    let pin_proof = compute_pin_proof(&challenge, &host_cert_sha256, &pin);
    write_frame(&mut ctrl_send, &ControlMessage::AuthResponse { pin_proof }).await?;

    let monitors = loop {
        let msg = tokio::time::timeout(
            Duration::from_secs(48),
            read_frame_limited::<_, ControlMessage>(&mut ctrl_recv, MAX_CONTROL_FRAME_SIZE),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Timeout: nessuna risposta dall'host (45s scaduti)"))??;

        match msg {
            ControlMessage::AuthPendingConsent { host_name } => {
                set_progress(&format!(
                    "Connessione stabilita! Richiesta arrivata a \"{host_name}\" — in attesa di accettazione..."
                ));
            }
            ControlMessage::AuthResult {
                accepted: true,
                monitors,
                ..
            } => break monitors,
            ControlMessage::AuthResult {
                accepted: false,
                reason,
                ..
            } => {
                anyhow::bail!(reason.unwrap_or_else(|| "Accesso negato".into()));
            }
            _ => anyhow::bail!("risposta di autenticazione non valida"),
        }
    };

    let selected_monitor = monitors
        .iter()
        .find(|m| m.is_primary)
        .or_else(|| monitors.first())
        .map(|m| m.id)
        .unwrap_or(0);
    let latest_frame = Arc::new(Mutex::new(None));
    let status_text = Arc::new(Mutex::new(format!("Connesso a {addr}")));
    let fps = Arc::new(Mutex::new(0.0f32));
    let rtt_ms: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<InputEvent>();
    let (control_tx, mut control_rx) = mpsc::unbounded_channel::<ControlMessage>();
    let (disconnect_tx, mut disconnect_rx) = watch::channel(false);
    let ended_reason: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // Input sender task
    let mut input_stream = open_uni_stream(&conn, StreamKind::Input).await?;
    tokio::spawn(async move {
        while let Some(ev) = input_rx.recv().await {
            if write_frame(&mut input_stream, &ev).await.is_err() {
                break;
            }
        }
    });

    // Video receiver + decoder task
    let conn_vid = conn.clone();
    let frame_slot = latest_frame.clone();
    let fps_slot = fps.clone();
    let status_slot = status_text.clone();
    let ended_vid = ended_reason.clone();
    let ctx_vid = ctx.clone();

    tokio::spawn(async move {
        let mut decoder = match H264Decoder::new() {
            Ok(d) => d,
            Err(e) => {
                *status_slot.lock().unwrap() = format!("Errore decoder H264: {e}");
                return;
            }
        };

        let mut frames_in_sec = 0u32;
        let mut sec_timer = Instant::now();

        if let Ok((StreamKind::Video, mut recv)) = accept_uni_stream(&conn_vid).await {
            while let Ok(pkt) = read_frame::<_, VideoFrame>(&mut recv).await {
                if let Ok(Some(decoded)) = decoder.decode_rgba(&pkt.data) {
                    *frame_slot.lock().unwrap() = Some(decoded);
                    frames_in_sec += 1;
                    if sec_timer.elapsed() >= Duration::from_secs(1) {
                        *fps_slot.lock().unwrap() =
                            frames_in_sec as f32 / sec_timer.elapsed().as_secs_f32();
                        frames_in_sec = 0;
                        sec_timer = Instant::now();
                    }
                    ctx_vid.request_repaint();
                }
            }
        }
        let mut end_guard = ended_vid.lock().unwrap();
        if end_guard.is_none() {
            *end_guard = Some("Hai perso la connessione con il PC remoto.".into());
        }
        *status_slot.lock().unwrap() = "Sessione terminata".into();
        ctx_vid.request_repaint();
    });

    // Control loop (with periodic Ping/Pong RTT measurement)
    let status_ctrl = status_text.clone();
    let ended_ctrl = ended_reason.clone();
    let rtt_ctrl = rtt_ms.clone();
    let ctx_ctrl = ctx.clone();
    tokio::spawn(async move {
        let session_start = Instant::now();
        let mut ping_interval = tokio::time::interval(Duration::from_secs(2));
        let mut ping_seq: u64 = 0;

        loop {
            tokio::select! {
                _ = ping_interval.tick() => {
                    ping_seq = ping_seq.wrapping_add(1);
                    let sent_us = session_start.elapsed().as_micros() as u64;
                    if write_frame(&mut ctrl_send, &ControlMessage::Ping { seq: ping_seq, sent_us }).await.is_err() {
                        break;
                    }
                }
                Some(ctrl_msg) = control_rx.recv() => {
                    if write_frame(&mut ctrl_send, &ctrl_msg).await.is_err() {
                        break;
                    }
                }
                _ = disconnect_rx.changed() => {
                    if *disconnect_rx.borrow() {
                        let _ = write_frame(&mut ctrl_send, &ControlMessage::Disconnect {
                            reason: "Disconnesso dal client".into(),
                        }).await;
                        let _ = ctrl_send.finish();
                        tokio::time::sleep(Duration::from_millis(80)).await;
                        break;
                    }
                }
                msg = read_frame_limited::<_, ControlMessage>(&mut ctrl_recv, MAX_CONTROL_FRAME_SIZE) => {
                    match msg {
                        Ok(ControlMessage::Pong { seq, sent_us }) => {
                            if seq == ping_seq {
                                let now_us = session_start.elapsed().as_micros() as u64;
                                if let Some(ms) = compute_rtt_ms(now_us, sent_us) {
                                    *rtt_ctrl.lock().unwrap() = Some(ms);
                                    ctx_ctrl.request_repaint();
                                }
                            }
                        }
                        Ok(ControlMessage::Disconnect { reason }) => {
                            *status_ctrl.lock().unwrap() = reason.clone();
                            *ended_ctrl.lock().unwrap() = Some(reason);
                            break;
                        }
                        Err(_) => {
                            let mut end_guard = ended_ctrl.lock().unwrap();
                            if end_guard.is_none() {
                                *end_guard = Some("Hai perso la connessione con il PC remoto.".into());
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
        conn.close(0u32.into(), b"client closed");
        ctx_ctrl.request_repaint();
    });

    Ok(ClientSessionHandle {
        remote_addr: addr,
        monitors,
        selected_monitor,
        latest_frame,
        input_tx,
        control_tx,
        status_text,
        fps,
        rtt_ms,
        disconnect_tx,
        ended_reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_locks_after_five_failures() {
        let mut rl = PinRateLimiter::default();
        assert!(rl.check_locked().is_none());
        for _ in 0..4 {
            rl.record_failure();
            assert!(rl.check_locked().is_none());
        }
        rl.record_failure();
        assert!(rl.check_locked().is_some());
        rl.reset();
        assert!(rl.check_locked().is_none());
    }

    #[test]
    fn compute_rtt_ms_handles_normal_and_invalid_timestamps() {
        assert_eq!(compute_rtt_ms(25_400, 10_000), Some(15));
        assert_eq!(compute_rtt_ms(10_000, 10_000), Some(0));
        // Future/tampered sent_us from peer must be rejected safely
        assert_eq!(compute_rtt_ms(5_000, 10_000), None);
    }

    #[test]
    fn validate_monitor_selection_accepts_known_and_rejects_unknown_id() {
        let monitors = vec![
            MonitorInfo {
                id: 0,
                name: "DISPLAY1".into(),
                width: 1920,
                height: 1080,
                is_primary: true,
            },
            MonitorInfo {
                id: 1,
                name: "DISPLAY2".into(),
                width: 2560,
                height: 1440,
                is_primary: false,
            },
        ];
        assert_eq!(validate_monitor_selection(&monitors, 0), Some(0));
        assert_eq!(validate_monitor_selection(&monitors, 1), Some(1));
        assert_eq!(validate_monitor_selection(&monitors, 99), None);
    }
}
