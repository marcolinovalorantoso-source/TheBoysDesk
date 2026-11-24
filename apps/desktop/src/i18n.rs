//! Internationalization (`EN` default with instant `IT` toggle).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    It,
}

impl Lang {
    pub fn toggle(self) -> Self {
        match self {
            Self::En => Self::It,
            Self::It => Self::En,
        }
    }

    #[cfg(test)]
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "EN",
            Self::It => "IT",
        }
    }

    pub fn switch_button_label(self) -> &'static str {
        match self {
            Self::En => "Lang: EN (Switch to IT)",
            Self::It => "Lingua: IT (Passa a EN)",
        }
    }

    pub fn pick(self, en: &'static str, it: &'static str) -> &'static str {
        match self {
            Self::En => en,
            Self::It => it,
        }
    }

    pub fn window_title(self) -> &'static str {
        self.pick(
            "TheBoysDesk — P2P Remote Desktop",
            "TheBoysDesk — Controllo Remoto P2P",
        )
    }

    pub fn subtitle(self) -> &'static str {
        self.pick(
            "Encrypted P2P remote desktop (QUIC / TLS 1.3)",
            "Controllo remoto P2P crittografato (QUIC / TLS 1.3)",
        )
    }

    pub fn net_badge(self, upnp_state: Option<bool>) -> &'static str {
        match upnp_state {
            Some(true) => self.pick(
                "● Internet P2P Ready (UPnP)",
                "● P2P Internet Pronto (UPnP)",
            ),
            Some(false) => self.pick(
                "● LAN Ready (UPnP disabled)",
                "● Pronto in LAN (UPnP disattivo)",
            ),
            None => self.pick("● Checking network...", "● Verifica rete..."),
        }
    }

    pub fn upnp_note_text(self, upnp_state: Option<bool>, quic_port: u16) -> String {
        match (self, upnp_state) {
            (Self::En, Some(true)) => {
                format!("UPnP active (port {quic_port}/UDP open)")
            }
            (Self::It, Some(true)) => {
                format!("UPnP attivo (porta {quic_port}/UDP aperta)")
            }
            (Self::En, Some(false)) => {
                format!("UPnP inactive: forward port {quic_port}/UDP on your router for Internet")
            }
            (Self::It, Some(false)) => {
                format!("UPnP disattivo: apri porta {quic_port}/UDP sul router per Internet")
            }
            (Self::En, None) => "Checking router UPnP...".into(),
            (Self::It, None) => "Verifica UPnP sul router...".into(),
        }
    }

    pub fn copy_label(self, copied: bool) -> &'static str {
        if copied {
            self.pick("Copied!", "Copiato!")
        } else {
            self.pick("Copy", "Copia")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_defaults_to_english_and_toggles() {
        let l = Lang::default();
        assert_eq!(l, Lang::En);
        assert_eq!(l.code(), "EN");
        let toggled = l.toggle();
        assert_eq!(toggled, Lang::It);
        assert_eq!(toggled.code(), "IT");
        assert_eq!(toggled.toggle(), Lang::En);
    }
}
