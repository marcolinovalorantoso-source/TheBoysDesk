//! Main desktop application (Host + Client) built with `egui` (`eframe` + `glow`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod key_map;
mod session;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::{cert_fingerprint_sha256, generate_session_pin, DeviceIdentity};
use eframe::egui;
use net::discovery::{start_lan_discovery, DiscoveredPeer};
use net::{local_lan_ipv4, stun, upnp, P2pEndpoint};
use protocol::{ControlMessage, InputEvent, MouseButton};
use tokio::sync::RwLock;

use crate::key_map::egui_key_to_scancode;
use crate::session::{
    connect_client_session, run_host_listener, ClientSessionHandle, HostRuntimeState,
};

const BG_DARK: egui::Color32 = egui::Color32::from_rgb(16, 19, 25);
const CARD_BG: egui::Color32 = egui::Color32::from_rgb(26, 32, 42);
const CARD_BORDER: egui::Color32 = egui::Color32::from_rgb(43, 52, 66);
const ACCENT_BLUE: egui::Color32 = egui::Color32::from_rgb(37, 128, 235);
const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(148, 163, 184);
const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(74, 222, 128);
const WARN_AMBER: egui::Color32 = egui::Color32::from_rgb(251, 191, 36);
const ERR_RED: egui::Color32 = egui::Color32::from_rgb(248, 113, 113);

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,quinn=warn".into()),
        )
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size([860.0, 560.0])
        .with_title("TheBoysDesk — Controllo Remoto P2P");
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_opts = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "TheBoysDesk",
        native_opts,
        Box::new(move |cc| {
            setup_visuals(&cc.egui_ctx);
            Ok(Box::new(RemoteDesktopApp::new(cc, rt)))
        }),
    )
}

fn load_app_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

fn setup_visuals(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(12.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG_DARK;
    visuals.window_fill = CARD_BG;
    visuals.extreme_bg_color = egui::Color32::from_rgb(13, 16, 22);
    visuals.widgets.noninteractive.bg_fill = CARD_BG;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, CARD_BORDER);
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(8.0);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(38, 45, 59);
    visuals.widgets.inactive.rounding = egui::Rounding::same(6.0);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(51, 61, 80);
    visuals.widgets.hovered.rounding = egui::Rounding::same(6.0);
    visuals.widgets.active.bg_fill = ACCENT_BLUE;
    visuals.widgets.active.rounding = egui::Rounding::same(6.0);
    visuals.window_rounding = egui::Rounding::same(10.0);
    style.visuals = visuals;
    ctx.set_style(style);
}

fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(CARD_BG)
        .stroke(egui::Stroke::new(1.0_f32, CARD_BORDER))
        .rounding(egui::Rounding::same(10.0))
        .inner_margin(egui::Margin::same(20.0))
}

fn draw_monitor_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::hover());
    let p = ui.painter();
    let stroke = egui::Stroke::new(2.0_f32, ACCENT_BLUE);
    let screen = egui::Rect::from_min_max(rect.min + egui::vec2(1.0, 2.0), rect.max - egui::vec2(1.0, 6.0));
    p.rect_stroke(screen, 3.0, stroke);
    let cx = rect.center().x;
    p.line_segment([egui::pos2(cx, screen.max.y), egui::pos2(cx, rect.max.y - 2.0)], stroke);
    p.line_segment([egui::pos2(cx - 6.0, rect.max.y - 2.0), egui::pos2(cx + 6.0, rect.max.y - 2.0)], stroke);
}

fn draw_arrows_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::hover());
    let p = ui.painter();
    let stroke = egui::Stroke::new(2.0_f32, ACCENT_BLUE);
    let y1 = rect.min.y + 7.0;
    let y2 = rect.max.y - 7.0;
    let l = rect.min.x + 2.0;
    let r = rect.max.x - 2.0;
    // Top arrow right
    p.line_segment([egui::pos2(l, y1), egui::pos2(r, y1)], stroke);
    p.line_segment([egui::pos2(r - 5.0, y1 - 4.0), egui::pos2(r, y1)], stroke);
    p.line_segment([egui::pos2(r - 5.0, y1 + 4.0), egui::pos2(r, y1)], stroke);
    // Bottom arrow left
    p.line_segment([egui::pos2(l, y2), egui::pos2(r, y2)], stroke);
    p.line_segment([egui::pos2(l + 5.0, y2 - 4.0), egui::pos2(l, y2)], stroke);
    p.line_segment([egui::pos2(l + 5.0, y2 + 4.0), egui::pos2(l, y2)], stroke);
}

fn draw_lan_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 20.0), egui::Sense::hover());
    let p = ui.painter();
    let stroke = egui::Stroke::new(1.8_f32, ACCENT_BLUE);
    let cx = rect.center().x;
    let top = egui::Rect::from_center_size(egui::pos2(cx, rect.min.y + 4.0), egui::vec2(6.0, 5.0));
    let bl = egui::Rect::from_center_size(egui::pos2(rect.min.x + 4.0, rect.max.y - 4.0), egui::vec2(6.0, 5.0));
    let br = egui::Rect::from_center_size(egui::pos2(rect.max.x - 4.0, rect.max.y - 4.0), egui::vec2(6.0, 5.0));
    p.rect_stroke(top, 1.0, stroke);
    p.rect_stroke(bl, 1.0, stroke);
    p.rect_stroke(br, 1.0, stroke);
    let mid_y = rect.center().y;
    p.line_segment([egui::pos2(cx, top.max.y), egui::pos2(cx, mid_y)], stroke);
    p.line_segment([egui::pos2(bl.center().x, mid_y), egui::pos2(br.center().x, mid_y)], stroke);
    p.line_segment([egui::pos2(bl.center().x, mid_y), egui::pos2(bl.center().x, bl.min.y)], stroke);
    p.line_segment([egui::pos2(br.center().x, mid_y), egui::pos2(br.center().x, br.min.y)], stroke);
}

struct RemoteDesktopApp {
    rt: tokio::runtime::Runtime,
    endpoint: Arc<P2pEndpoint>,
    device_id: String,
    host_name: String,
    host_state: Arc<HostRuntimeState>,
    local_pin: String,

    lan_addr_str: Arc<Mutex<String>>,
    public_addr_str: Arc<Mutex<String>>,
    upnp_ok: Arc<Mutex<Option<bool>>>,
    upnp_note: Arc<Mutex<String>>,
    lan_peers: Option<Arc<RwLock<HashMap<String, DiscoveredPeer>>>>,

    // Client connect form
    target_addr_input: String,
    target_pin_input: String,
    connect_error: Arc<Mutex<Option<String>>>,
    connect_progress: Arc<Mutex<Option<String>>>,
    connecting: Arc<Mutex<bool>>,
    connect_task: Option<tokio::task::JoinHandle<()>>,
    client_session: Arc<Mutex<Option<ClientSessionHandle>>>,

    // Copy feedback & modals
    copied_hint: Option<(&'static str, Instant)>,
    session_notice: Option<String>,
    show_info_modal: bool,

    // Viewer texture & input state
    remote_texture: Option<egui::TextureHandle>,
    fit_to_window: bool,
    send_mouse: bool,
    send_keyboard: bool,
    last_mouse_norm: Option<(f32, f32)>,
}

impl RemoteDesktopApp {
    fn new(cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Runtime) -> Self {
        let identity = DeviceIdentity::generate().expect("device identity");
        let device_id = identity.device_id.clone();
        let host_name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows-PC".into());
        let initial_pin = generate_session_pin();

        let endpoint = Arc::new(
            rt.block_on(async { P2pEndpoint::bind(&identity) })
                .expect("bind QUIC endpoint"),
        );
        let quic_port = endpoint.local_port();

        let lan_ip = local_lan_ipv4()
            .map(|ip| format!("{ip}:{quic_port}"))
            .unwrap_or_else(|| format!("127.0.0.1:{quic_port}"));

        let lan_addr_str = Arc::new(Mutex::new(lan_ip));
        let public_addr_str = Arc::new(Mutex::new("Rilevamento...".into()));
        let upnp_ok = Arc::new(Mutex::new(None));
        let upnp_note = Arc::new(Mutex::new("Verifica UPnP sul router...".into()));

        let host_cert_sha256 = cert_fingerprint_sha256(identity.cert_der.as_ref());
        let host_state = Arc::new(HostRuntimeState::new(initial_pin.clone()));

        rt.spawn(run_host_listener(
            endpoint.clone(),
            device_id.clone(),
            host_name.clone(),
            host_cert_sha256,
            host_state.clone(),
            cc.egui_ctx.clone(),
        ));

        let lan_peers = rt
            .block_on(start_lan_discovery(
                device_id.clone(),
                host_name.clone(),
                quic_port,
            ))
            .ok();

        let pub_clone = public_addr_str.clone();
        let upnp_ok_clone = upnp_ok.clone();
        let upnp_note_clone = upnp_note.clone();
        let ctx_net = cc.egui_ctx.clone();
        rt.spawn(async move {
            match upnp::try_map_udp_port(quic_port).await {
                Ok(m) => {
                    *upnp_ok_clone.lock().unwrap() = Some(true);
                    *upnp_note_clone.lock().unwrap() =
                        format!("UPnP attivo (porta {}/UDP aperta)", m.external_port);
                    *pub_clone.lock().unwrap() = format!("{}:{}", m.external_ip, m.external_port);
                }
                Err(_) => {
                    *upnp_ok_clone.lock().unwrap() = Some(false);
                    *upnp_note_clone.lock().unwrap() =
                        format!("UPnP disattivo: apri porta {quic_port}/UDP sul router per Internet");
                    match stun::discover_public_addr().await {
                        Ok(addr) => {
                            *pub_clone.lock().unwrap() = format!("{}:{quic_port}", addr.ip());
                        }
                        Err(_) => {
                            *pub_clone.lock().unwrap() = "Non rilevato".into();
                        }
                    }
                }
            }
            ctx_net.request_repaint();
        });

        Self {
            rt,
            endpoint,
            device_id,
            host_name,
            host_state,
            local_pin: initial_pin,
            lan_addr_str,
            public_addr_str,
            upnp_ok,
            upnp_note,
            lan_peers,
            target_addr_input: String::new(),
            target_pin_input: String::new(),
            connect_error: Arc::new(Mutex::new(None)),
            connect_progress: Arc::new(Mutex::new(None)),
            connecting: Arc::new(Mutex::new(false)),
            connect_task: None,
            client_session: Arc::new(Mutex::new(None)),
            copied_hint: None,
            session_notice: None,
            show_info_modal: false,
            remote_texture: None,
            fit_to_window: true,
            send_mouse: true,
            send_keyboard: true,
            last_mouse_norm: None,
        }
    }

    fn copy_with_feedback(&mut self, ctx: &egui::Context, key: &'static str, value: String) {
        ctx.copy_text(value);
        self.copied_hint = Some((key, Instant::now()));
    }

    fn is_copied(&self, key: &str) -> bool {
        self.copied_hint
            .map(|(k, t)| k == key && t.elapsed() < Duration::from_secs(2))
            .unwrap_or(false)
    }

    fn start_connect(&mut self, ctx: &egui::Context) {
        let addr_str = self.target_addr_input.trim().to_string();
        let pin = self.target_pin_input.trim().to_string();
        let Ok(addr) = addr_str.parse::<SocketAddr>() else {
            *self.connect_error.lock().unwrap() =
                Some("Formato non valido. Usa IP:Porta (es. 192.168.1.50:21116)".into());
            return;
        };
        if pin.is_empty() {
            *self.connect_error.lock().unwrap() = Some("Inserisci il PIN di sessione".into());
            return;
        }

        *self.connect_error.lock().unwrap() = None;
        *self.connect_progress.lock().unwrap() = Some("Avvio connessione...".into());
        *self.connecting.lock().unwrap() = true;

        let ep = self.endpoint.clone();
        let local_name = self.host_name.clone();
        let local_id = self.device_id.clone();
        let session_slot = self.client_session.clone();
        let err_slot = self.connect_error.clone();
        let prog_slot = self.connect_progress.clone();
        let conn_flag = self.connecting.clone();
        let ctx_c = ctx.clone();

        let handle = self.rt.spawn(async move {
            match connect_client_session(
                ep,
                addr,
                pin,
                local_name,
                local_id,
                ctx_c.clone(),
                prog_slot.clone(),
            )
            .await
            {
                Ok(handle) => {
                    *session_slot.lock().unwrap() = Some(handle);
                }
                Err(e) => {
                    *err_slot.lock().unwrap() = Some(format!("Connessione fallita: {e}"));
                }
            }
            *prog_slot.lock().unwrap() = None;
            *conn_flag.lock().unwrap() = false;
            ctx_c.request_repaint();
        });
        self.connect_task = Some(handle);
    }
}

impl eframe::App for RemoteDesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. Incoming connection consent modal
        let mut consent_decision: Option<bool> = None;
        let consent_info = {
            let guard = self.host_state.pending_consent.lock().unwrap();
            guard
                .as_ref()
                .map(|p| (p.peer_name.clone(), p.peer_id.clone(), p.peer_addr))
        };

        if let Some((peer_name, peer_id, peer_addr)) = consent_info {
            egui::Window::new("Richiesta di Controllo Remoto")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(360.0);
                    ui.label(
                        egui::RichText::new(format!("{peer_name}"))
                            .size(18.0)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!("ID: {peer_id}  •  {peer_addr}"))
                            .color(TEXT_MUTED),
                    );
                    ui.add_space(6.0);
                    ui.label("Chiede di visualizzare e controllare questo computer.");
                    ui.small("Per sicurezza, mouse e tastiera partiranno disabilitati.");
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Accetta").color(egui::Color32::BLACK),
                                )
                                .fill(OK_GREEN)
                                .min_size(egui::vec2(110.0, 32.0)),
                            )
                            .clicked()
                        {
                            consent_decision = Some(true);
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Rifiuta").color(egui::Color32::BLACK),
                                )
                                .fill(ERR_RED)
                                .min_size(egui::vec2(110.0, 32.0)),
                            )
                            .clicked()
                        {
                            consent_decision = Some(false);
                        }
                    });
                });
        }

        if let Some(decision) = consent_decision {
            if let Some(pending) = self.host_state.pending_consent.lock().unwrap().take() {
                let _ = pending.reply.send(decision);
            }
        }

        // 1b. Pick up any host disconnect / connection-loss notice
        if let Some(notice) = self.host_state.disconnect_notice.lock().unwrap().take() {
            self.session_notice = Some(notice);
        }

        // 1c. Show session-ended modal if present
        let mut clear_notice = false;
        if let Some(notice) = &self.session_notice {
            egui::Window::new("Avviso Sessione")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(340.0);
                    ui.colored_label(WARN_AMBER, egui::RichText::new(notice).size(15.0));
                    ui.add_space(12.0);
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("OK").color(egui::Color32::WHITE),
                            )
                            .fill(ACCENT_BLUE)
                            .min_size(egui::vec2(90.0, 30.0)),
                        )
                        .clicked()
                    {
                        clear_notice = true;
                    }
                });
        }
        if clear_notice {
            self.session_notice = None;
        }

        // 1d. Info / Credits modal
        if self.show_info_modal {
            let mut close_info = false;
            let mut copy_req: Option<(&'static str, &'static str)> = None;
            let email_copied = self.is_copied("info_email");
            let discord_copied = self.is_copied("info_discord");

            egui::Window::new("Info — TheBoysDesk")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(350.0);
                    ui.label(egui::RichText::new("TheBoysDesk").size(20.0).strong());
                    ui.label(
                        egui::RichText::new("Versione 0.1.0  •  Controllo remoto P2P")
                            .color(TEXT_MUTED)
                            .size(12.5),
                    );
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(8.0);

                    ui.label(egui::RichText::new("Fatta da Marco Venditti").size(15.0).strong());
                    ui.add_space(6.0);

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Email:").color(TEXT_MUTED));
                        ui.monospace("mark09venditti@gmail.com");
                        let lbl = if email_copied { "Copiato!" } else { "Copia" };
                        if ui.small_button(lbl).clicked() {
                            copy_req = Some(("info_email", "mark09venditti@gmail.com"));
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Discord:").color(TEXT_MUTED));
                        ui.monospace("cicciobaudo");
                        let lbl = if discord_copied { "Copiato!" } else { "Copia" };
                        if ui.small_button(lbl).clicked() {
                            copy_req = Some(("info_discord", "cicciobaudo"));
                        }
                    });

                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Con l'aiuto di LatiosJet")
                            .color(ACCENT_BLUE)
                            .strong(),
                    );
                    ui.add_space(14.0);

                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Chiudi").color(egui::Color32::WHITE),
                            )
                            .fill(ACCENT_BLUE)
                            .min_size(egui::vec2(100.0, 32.0)),
                        )
                        .clicked()
                    {
                        close_info = true;
                    }
                });

            if let Some((key, val)) = copy_req {
                self.copy_with_feedback(ctx, key, val.to_string());
            }
            if close_info {
                self.show_info_modal = false;
            }
        }

        // 2. Always-visible Host Active Session Banner
        let active_peer = self.host_state.active_peer.lock().unwrap().clone();
        if let Some(peer) = active_peer {
            ctx.input(|inp| {
                if inp.modifiers.ctrl && inp.modifiers.alt {
                    if inp.key_pressed(egui::Key::M) {
                        let prev = self.host_state.allow_mouse.load(Ordering::Relaxed);
                        self.host_state.allow_mouse.store(!prev, Ordering::Relaxed);
                    }
                    if inp.key_pressed(egui::Key::K) {
                        let prev = self.host_state.allow_keyboard.load(Ordering::Relaxed);
                        self.host_state.allow_keyboard.store(!prev, Ordering::Relaxed);
                    }
                    if inp.key_pressed(egui::Key::B) {
                        let both_on = self.host_state.allow_mouse.load(Ordering::Relaxed)
                            && self.host_state.allow_keyboard.load(Ordering::Relaxed);
                        self.host_state.allow_mouse.store(!both_on, Ordering::Relaxed);
                        self.host_state.allow_keyboard.store(!both_on, Ordering::Relaxed);
                    }
                    if inp.key_pressed(egui::Key::D) {
                        self.host_state.disconnect_active();
                    }
                }
            });

            let mouse_on = self.host_state.allow_mouse.load(Ordering::Relaxed);
            let kbd_on = self.host_state.allow_keyboard.load(Ordering::Relaxed);
            let both_blocked = !mouse_on && !kbd_on;

            egui::TopBottomPanel::top("host_active_banner")
                .frame(
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgb(28, 33, 46))
                        .stroke(egui::Stroke::new(1.0_f32, CARD_BORDER))
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            WARN_AMBER,
                            egui::RichText::new(format!("● SESSIONE ATTIVA — {peer}")).strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Disconnetti (Ctrl+Alt+D)")
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(185, 28, 28)),
                                )
                                .clicked()
                            {
                                self.host_state.disconnect_active();
                            }
                        });
                    });

                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        let mouse_lbl = if mouse_on {
                            "Mouse: ON (Ctrl+Alt+M)"
                        } else {
                            "Mouse: OFF (Ctrl+Alt+M)"
                        };
                        if ui
                            .button(egui::RichText::new(mouse_lbl).color(if mouse_on {
                                OK_GREEN
                            } else {
                                ERR_RED
                            }))
                            .clicked()
                        {
                            self.host_state.allow_mouse.store(!mouse_on, Ordering::Relaxed);
                        }

                        let kbd_lbl = if kbd_on {
                            "Tastiera: ON (Ctrl+Alt+K)"
                        } else {
                            "Tastiera: OFF (Ctrl+Alt+K)"
                        };
                        if ui
                            .button(egui::RichText::new(kbd_lbl).color(if kbd_on {
                                OK_GREEN
                            } else {
                                ERR_RED
                            }))
                            .clicked()
                        {
                            self.host_state.allow_keyboard.store(!kbd_on, Ordering::Relaxed);
                        }

                        let both_lbl = if both_blocked {
                            "Abilita Entrambi (Ctrl+Alt+B)"
                        } else {
                            "Blocca Entrambi (Ctrl+Alt+B)"
                        };
                        if ui.button(both_lbl).clicked() {
                            let target = both_blocked;
                            self.host_state.allow_mouse.store(target, Ordering::Relaxed);
                            self.host_state.allow_keyboard.store(target, Ordering::Relaxed);
                        }
                    });

                    if both_blocked {
                        ui.add_space(4.0);
                        ui.colored_label(
                            ERR_RED,
                            "⚠ Mouse e tastiera sono disabilitati. Abilitali per consentire il controllo.",
                        );
                    } else if !mouse_on {
                        ui.add_space(4.0);
                        ui.colored_label(WARN_AMBER, "⚠ Mouse disabilitato (solo tastiera attiva).");
                    } else if !kbd_on {
                        ui.add_space(4.0);
                        ui.colored_label(WARN_AMBER, "⚠ Tastiera disabilitata (solo mouse attivo).");
                    }
                });
        }

        // 3. Check if Client Viewer is active (or just ended remotely)
        let remote_end_reason = {
            let guard = self.client_session.lock().unwrap();
            guard
                .as_ref()
                .and_then(|s| s.ended_reason.lock().unwrap().clone())
        };
        if let Some(reason) = remote_end_reason {
            self.client_session.lock().unwrap().take();
            self.remote_texture = None;
            self.session_notice = Some(reason);
        }

        let mut disconnect_client = false;
        let has_client_session = self.client_session.lock().unwrap().is_some();

        if has_client_session {
            self.render_client_viewer(ctx, &mut disconnect_client);
            if disconnect_client {
                if let Some(sess) = self.client_session.lock().unwrap().take() {
                    let _ = sess.disconnect_tx.send(true);
                }
                self.remote_texture = None;
            }
            return;
        }

        // 4. Main Dashboard (Home)
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG_DARK).inner_margin(egui::Margin::same(20.0)))
            .show(ctx, |ui| {
            // Header
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("TheBoysDesk").size(27.0).strong());
                    ui.label(
                        egui::RichText::new("Controllo remoto P2P crittografato (QUIC / TLS 1.3)")
                            .color(TEXT_MUTED)
                            .size(14.0),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Info").clicked() {
                        self.show_info_modal = true;
                    }
                    ui.add_space(6.0);
                    let upnp_state = *self.upnp_ok.lock().unwrap();
                    let (badge_txt, badge_col) = match upnp_state {
                        Some(true) => ("● P2P Internet Pronto (UPnP)", OK_GREEN),
                        Some(false) => ("● Pronto in LAN (UPnP disattivo)", WARN_AMBER),
                        None => ("● Verifica rete...", TEXT_MUTED),
                    };
                    ui.label(egui::RichText::new(badge_txt).color(badge_col).size(12.5));
                });
            });

            ui.add_space(18.0);

            // Two main cards
            ui.columns(2, |cols| {
                // Left Card: Receive Support
                card_frame().show(&mut cols[0], |ui| {
                    ui.set_min_height(330.0);
                    ui.horizontal(|ui| {
                        draw_monitor_icon(ui);
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("Ricevi Assistenza").size(20.0).strong());
                            ui.label(
                                egui::RichText::new("Condividi indirizzo e PIN con chi ti aiuta")
                                    .color(TEXT_MUTED)
                                    .size(12.0),
                            );
                        });
                    });
                    ui.add_space(10.0);

                    ui.label(egui::RichText::new("IL TUO ID DISPOSITIVO").size(11.0).color(TEXT_MUTED));
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&self.device_id)
                                .size(29.0)
                                .strong()
                                .monospace(),
                        );
                        let lbl = if self.is_copied("id") { "Copiato!" } else { "Copia" };
                        if ui.small_button(lbl).clicked() {
                            self.copy_with_feedback(ctx, "id", self.device_id.clone());
                        }
                    });

                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("PIN DI SESSIONE").size(11.0).color(TEXT_MUTED));
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&self.local_pin)
                                .size(27.0)
                                .strong()
                                .monospace()
                                .color(ACCENT_BLUE),
                        );
                        let pin_lbl = if self.is_copied("pin") { "Copiato!" } else { "Copia" };
                        if ui.small_button(pin_lbl).clicked() {
                            self.copy_with_feedback(ctx, "pin", self.local_pin.clone());
                        }
                        if ui.small_button("Rigenera").clicked() {
                            let new_pin = generate_session_pin();
                            self.local_pin = new_pin.clone();
                            let hs = self.host_state.clone();
                            self.rt.spawn(async move {
                                *hs.pin.write().await = new_pin;
                            });
                        }
                    });

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(6.0);

                    let lan_str = self.lan_addr_str.lock().unwrap().clone();
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("LAN:").color(TEXT_MUTED));
                        ui.monospace(&lan_str);
                        let lbl = if self.is_copied("lan") { "Copiato!" } else { "Copia" };
                        if ui.small_button(lbl).clicked() {
                            self.copy_with_feedback(ctx, "lan", lan_str);
                        }
                    });

                    let pub_str = self.public_addr_str.lock().unwrap().clone();
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Internet:").color(TEXT_MUTED));
                        ui.monospace(&pub_str);
                        let lbl = if self.is_copied("pub") { "Copiato!" } else { "Copia" };
                        if ui.small_button(lbl).clicked() {
                            self.copy_with_feedback(ctx, "pub", pub_str);
                        }
                    });

                    let upnp_note = self.upnp_note.lock().unwrap().clone();
                    ui.label(egui::RichText::new(upnp_note).size(11.5).color(TEXT_MUTED));
                });

                // Right Card: Connect to Remote PC
                card_frame().show(&mut cols[1], |ui| {
                    ui.set_min_height(330.0);
                    ui.horizontal(|ui| {
                        draw_arrows_icon(ui);
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("Controlla un altro PC").size(20.0).strong());
                            ui.label(
                                egui::RichText::new("Inserisci l'indirizzo IP:Porta e il PIN del PC remoto")
                                    .color(TEXT_MUTED)
                                    .size(12.0),
                            );
                        });
                    });
                    ui.add_space(12.0);

                    ui.label(egui::RichText::new("Indirizzo IP:Porta").size(12.0).color(TEXT_MUTED));
                    let addr_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.target_addr_input)
                            .hint_text("es. 192.168.1.50:21116")
                            .desired_width(f32::INFINITY),
                    );

                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("PIN di sessione").size(12.0).color(TEXT_MUTED));
                    let pin_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.target_pin_input)
                            .hint_text("es. 123456")
                            .desired_width(f32::INFINITY),
                    );

                    let enter_pressed = (addr_resp.lost_focus() || pin_resp.lost_focus())
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    ui.add_space(12.0);
                    let is_connecting = *self.connecting.lock().unwrap();
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!is_connecting, |ui| {
                            let btn = egui::Button::new(
                                egui::RichText::new("Connetti").color(egui::Color32::WHITE).strong(),
                            )
                            .fill(ACCENT_BLUE)
                            .min_size(egui::vec2(140.0, 38.0));
                            if ui.add(btn).clicked() || (!is_connecting && enter_pressed) {
                                self.start_connect(ctx);
                            }
                        });

                        if is_connecting {
                            ui.spinner();
                            if ui.button("Annulla").clicked() {
                                if let Some(h) = self.connect_task.take() {
                                    h.abort();
                                }
                                *self.connecting.lock().unwrap() = false;
                                *self.connect_progress.lock().unwrap() = None;
                                *self.connect_error.lock().unwrap() =
                                    Some("Connessione annullata".into());
                            }
                        }
                    });

                    if let Some(prog) = self.connect_progress.lock().unwrap().as_ref() {
                        ui.add_space(8.0);
                        ui.colored_label(WARN_AMBER, prog);
                    }
                    if let Some(err) = self.connect_error.lock().unwrap().as_ref() {
                        ui.add_space(8.0);
                        ui.colored_label(ERR_RED, err);
                    }
                });
            });

            ui.add_space(12.0);

            // Bottom Card: LAN Discovery
            card_frame().show(ctx_or_ui_hack(ui), |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(92.0);
                ui.horizontal(|ui| {
                    draw_lan_icon(ui);
                    ui.label(egui::RichText::new("Dispositivi rilevati in LAN").size(15.0).strong());
                });
                ui.add_space(4.0);

                if let Some(peers_arc) = &self.lan_peers {
                    if let Ok(peers) = peers_arc.try_read() {
                        if peers.is_empty() {
                            ui.label(
                                egui::RichText::new("Nessun altro dispositivo rilevato sulla rete locale...")
                                    .color(TEXT_MUTED),
                            );
                        } else {
                            for peer in peers.values() {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&peer.device_name).strong());
                                    ui.label(
                                        egui::RichText::new(format!("({})", peer.device_id))
                                            .color(TEXT_MUTED),
                                    );
                                    ui.monospace(peer.addr.to_string());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.button("Usa indirizzo").clicked() {
                                                self.target_addr_input = peer.addr.to_string();
                                            }
                                        },
                                    );
                                });
                            }
                        }
                    }
                }
            });
        });
    }
}

#[inline]
fn ctx_or_ui_hack(ui: &mut egui::Ui) -> &mut egui::Ui {
    ui
}

impl RemoteDesktopApp {
    fn render_client_viewer(&mut self, ctx: &egui::Context, disconnect: &mut bool) {
        let mut guard = self.client_session.lock().unwrap();
        let Some(sess) = guard.as_mut() else {
            return;
        };

        let fps = *sess.fps.lock().unwrap();
        let rtt_val = *sess.rtt_ms.lock().unwrap();
        let (rtt_str, rtt_col) = match rtt_val {
            Some(ms) if ms < 40 => (format!("{ms} ms"), OK_GREEN),
            Some(ms) if ms <= 100 => (format!("{ms} ms"), WARN_AMBER),
            Some(ms) => (format!("{ms} ms"), ERR_RED),
            None => ("— ms".into(), TEXT_MUTED),
        };
        let status = sess.status_text.lock().unwrap().clone();
        let mon_info = sess
            .monitors
            .iter()
            .find(|m| m.id == sess.selected_monitor)
            .or_else(|| sess.monitors.first())
            .map(|m| format!("{}x{}", m.width, m.height))
            .unwrap_or_else(|| "?".into());

        egui::TopBottomPanel::top("viewer_toolbar")
            .frame(
                egui::Frame::none()
                    .fill(CARD_BG)
                    .stroke(egui::Stroke::new(1.0_f32, CARD_BORDER))
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} ({})  •  {}  •  {:.1} FPS  •",
                            status, sess.remote_addr, mon_info, fps
                        ))
                        .size(12.5),
                    );
                    ui.colored_label(rtt_col, egui::RichText::new(rtt_str).size(12.5).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Disconnetti").color(egui::Color32::WHITE),
                                )
                                .fill(egui::Color32::from_rgb(185, 28, 28)),
                            )
                            .clicked()
                        {
                            *disconnect = true;
                        }
                    });
                });

                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.fit_to_window, "Adatta");
                    ui.separator();

                    if sess.monitors.len() > 1 {
                        for (idx, mon) in sess.monitors.iter().enumerate() {
                            let is_active = sess.selected_monitor == mon.id;
                            let btn_txt = egui::RichText::new(format!("M{}", idx + 1))
                                .color(if is_active {
                                    egui::Color32::WHITE
                                } else {
                                    TEXT_MUTED
                                });
                            let mut btn = egui::Button::new(btn_txt);
                            if is_active {
                                btn = btn.fill(ACCENT_BLUE);
                            }
                            if ui
                                .add(btn)
                                .on_hover_text(format!("{} ({}x{})", mon.name, mon.width, mon.height))
                                .clicked()
                                && !is_active
                            {
                                sess.selected_monitor = mon.id;
                                let _ = sess
                                    .control_tx
                                    .send(ControlMessage::SelectMonitor { monitor_id: mon.id });
                            }
                        }
                    } else {
                        ui.label(egui::RichText::new("Monitor 1").color(TEXT_MUTED).size(12.0));
                    }
                    ui.separator();

                    if ui
                        .button(
                            egui::RichText::new(if self.send_mouse {
                                "Mouse: ON"
                            } else {
                                "Mouse: OFF"
                            })
                            .color(if self.send_mouse { OK_GREEN } else { ERR_RED }),
                        )
                        .clicked()
                    {
                        self.send_mouse = !self.send_mouse;
                    }

                    if ui
                        .button(
                            egui::RichText::new(if self.send_keyboard {
                                "Tastiera: ON"
                            } else {
                                "Tastiera: OFF"
                            })
                            .color(if self.send_keyboard { OK_GREEN } else { ERR_RED }),
                        )
                        .clicked()
                    {
                        self.send_keyboard = !self.send_keyboard;
                        if !self.send_keyboard {
                            let _ = sess.input_tx.send(InputEvent::ReleaseAllKeys);
                        }
                    }

                    let both_off = !self.send_mouse && !self.send_keyboard;
                    if ui
                        .button(if both_off { "Ripristina Input" } else { "Solo Vista" })
                        .clicked()
                    {
                        self.send_mouse = both_off;
                        self.send_keyboard = both_off;
                        if !self.send_keyboard {
                            let _ = sess.input_tx.send(InputEvent::ReleaseAllKeys);
                        }
                    }
                });
            });

        if let Some(frame) = sess.latest_frame.lock().unwrap().take() {
            let color_img = egui::ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                &frame.rgba,
            );
            match &mut self.remote_texture {
                Some(tex) => tex.set(color_img, egui::TextureOptions::LINEAR),
                None => {
                    self.remote_texture = Some(ctx.load_texture(
                        "remote_screen",
                        color_img,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                let Some(tex) = &self.remote_texture else {
                    ui.centered_and_justified(|ui| {
                        ui.label("In attesa del primo frame video...");
                    });
                    return;
                };

                let tex_size = tex.size_vec2();
                let avail = ui.available_size();
                let draw_size = if self.fit_to_window {
                    let scale = (avail.x / tex_size.x).min(avail.y / tex_size.y);
                    tex_size * scale
                } else {
                    tex_size
                };

                let (rect, response) = ui.allocate_exact_size(avail, egui::Sense::click_and_drag());
                let img_rect = egui::Rect::from_center_size(rect.center(), draw_size);
                ui.painter().image(
                    tex.id(),
                    img_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                ctx.input(|inp| {
                    if self.send_mouse {
                        if let Some(pos) = inp.pointer.hover_pos() {
                            if img_rect.contains(pos) {
                                let xn = ((pos.x - img_rect.min.x) / img_rect.width()).clamp(0.0, 1.0);
                                let yn = ((pos.y - img_rect.min.y) / img_rect.height()).clamp(0.0, 1.0);
                                let moved = self
                                    .last_mouse_norm
                                    .map(|(lx, ly)| (lx - xn).abs() > 0.0005 || (ly - yn).abs() > 0.0005)
                                    .unwrap_or(true);
                                if moved {
                                    self.last_mouse_norm = Some((xn, yn));
                                    let _ = sess.input_tx.send(InputEvent::MouseMove {
                                        x_norm: xn,
                                        y_norm: yn,
                                        monitor_id: sess.selected_monitor,
                                    });
                                }
                            }
                        }

                        if response.hovered() {
                            let scroll = inp.raw_scroll_delta;
                            if scroll.x.abs() > 0.1 || scroll.y.abs() > 0.1 {
                                let _ = sess.input_tx.send(InputEvent::MouseWheel {
                                    delta_x: scroll.x / 120.0,
                                    delta_y: scroll.y / 120.0,
                                });
                            }
                        }
                    }

                    for ev in &inp.events {
                        match ev {
                            egui::Event::PointerButton {
                                pos,
                                button,
                                pressed,
                                ..
                            } if self.send_mouse && img_rect.contains(*pos) => {
                                let mb = match button {
                                    egui::PointerButton::Primary => Some(MouseButton::Left),
                                    egui::PointerButton::Secondary => Some(MouseButton::Right),
                                    egui::PointerButton::Middle => Some(MouseButton::Middle),
                                    egui::PointerButton::Extra1 => Some(MouseButton::Back),
                                    egui::PointerButton::Extra2 => Some(MouseButton::Forward),
                                };
                                if let Some(b) = mb {
                                    let _ = sess.input_tx.send(InputEvent::MouseButton {
                                        button: b,
                                        pressed: *pressed,
                                    });
                                }
                            }
                            egui::Event::Key {
                                key,
                                pressed,
                                repeat: false,
                                ..
                            } if self.send_keyboard => {
                                if let Some((sc, ext)) = egui_key_to_scancode(*key) {
                                    let _ = sess.input_tx.send(InputEvent::Key {
                                        scan_code: sc,
                                        extended: ext,
                                        pressed: *pressed,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                });
            });
    }
}
