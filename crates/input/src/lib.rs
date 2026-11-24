//! Windows remote input injection using `SendInput`.

use protocol::{InputEvent, MouseButton};
use thiserror::Error;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, WHEEL_DELTA,
};

#[derive(Debug, Error)]
pub enum InputError {
    #[error("SendInput failed to inject event")]
    SendInputFailed,
}

/// Enforces host-side input permissions before any OS injection occurs.
///
/// `ReleaseAllKeys` is always permitted because it only emits key-up events.
pub fn is_input_event_permitted(
    event: &InputEvent,
    allow_mouse: bool,
    allow_keyboard: bool,
) -> bool {
    match event {
        InputEvent::MouseMove { .. }
        | InputEvent::MouseButton { .. }
        | InputEvent::MouseWheel { .. } => allow_mouse,
        InputEvent::Key { .. } => allow_keyboard,
        InputEvent::ReleaseAllKeys => true,
    }
}

/// Injects mouse and keyboard events into the active Windows desktop session.
#[derive(Debug, Default)]
pub struct WindowsInputInjector;

impl WindowsInputInjector {
    /// Releases all standard mouse buttons so nothing stays stuck if mouse input is disabled.
    pub fn release_mouse_buttons(&self) {
        for btn in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
        ] {
            let _ = self.mouse_button(btn, false);
        }
    }

    /// Releases both mouse buttons and modifier keys.
    pub fn release_all_inputs(&self) {
        self.release_mouse_buttons();
        let _ = self.inject(&InputEvent::ReleaseAllKeys);
    }

    pub fn inject(&self, event: &InputEvent) -> Result<(), InputError> {
        match *event {
            InputEvent::MouseMove {
                x_norm,
                y_norm,
                monitor_id: _,
            } => self.move_mouse_normalized(x_norm, y_norm),
            InputEvent::MouseButton { button, pressed } => self.mouse_button(button, pressed),
            InputEvent::MouseWheel { delta_x, delta_y } => self.mouse_wheel(delta_x, delta_y),
            InputEvent::Key {
                scan_code,
                extended,
                pressed,
            } => self.key_scancode(scan_code, extended, pressed),
            InputEvent::ReleaseAllKeys => {
                // Release common modifier scan codes so nothing stays stuck on disconnect.
                for &(sc, ext) in &[
                    (0x2A, false), // LShift
                    (0x36, false), // RShift
                    (0x1D, false), // LCtrl
                    (0x1D, true),  // RCtrl
                    (0x38, false), // LAlt
                    (0x38, true),  // RAlt
                    (0x5B, true),  // LWin
                ] {
                    let _ = self.key_scancode(sc, ext, false);
                }
                Ok(())
            }
        }
    }

    fn move_mouse_normalized(&self, x_norm: f32, y_norm: f32) -> Result<(), InputError> {
        if !x_norm.is_finite() || !y_norm.is_finite() {
            return Ok(());
        }
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);

            // Map primary-screen normalized [0..1] to virtual desktop [0..65535].
            let primary_w = GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN).max(1);
            let primary_h = GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN).max(1);

            let px = (x_norm.clamp(0.0, 1.0) * primary_w as f32) as i32;
            let py = (y_norm.clamp(0.0, 1.0) * primary_h as f32) as i32;

            let abs_x = (((px - vx) as f64 / vw as f64) * 65535.0).round() as i32;
            let abs_y = (((py - vy) as f64 / vh as f64) * 65535.0).round() as i32;

            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: abs_x,
                        dy: abs_y,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            send_one(input)
        }
    }

    fn mouse_button(&self, button: MouseButton, pressed: bool) -> Result<(), InputError> {
        let (flags, data) = match (button, pressed) {
            (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
            (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (MouseButton::Back, true) => (MOUSEEVENTF_XDOWN, 1),
            (MouseButton::Back, false) => (MOUSEEVENTF_XUP, 1),
            (MouseButton::Forward, true) => (MOUSEEVENTF_XDOWN, 2),
            (MouseButton::Forward, false) => (MOUSEEVENTF_XUP, 2),
        };

        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        send_one(input)
    }

    fn mouse_wheel(&self, delta_x: f32, delta_y: f32) -> Result<(), InputError> {
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return Ok(());
        }
        if delta_y.abs() > 0.01 {
            let amount = (delta_y * WHEEL_DELTA as f32).round() as i32 as u32;
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: amount,
                        dwFlags: MOUSEEVENTF_WHEEL,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            send_one(input)?;
        }
        if delta_x.abs() > 0.01 {
            let amount = (delta_x * WHEEL_DELTA as f32).round() as i32 as u32;
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: amount,
                        dwFlags: MOUSEEVENTF_HWHEEL,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            send_one(input)?;
        }
        Ok(())
    }

    fn key_scancode(
        &self,
        scan_code: u16,
        extended: bool,
        pressed: bool,
    ) -> Result<(), InputError> {
        let mut flags = KEYEVENTF_SCANCODE;
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !pressed {
            flags |= KEYEVENTF_KEYUP;
        }

        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: scan_code,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        send_one(input)
    }
}

fn send_one(input: INPUT) -> Result<(), InputError> {
    unsafe {
        let sent = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        if sent == 0 {
            Err(InputError::SendInputFailed)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_gate_blocks_mouse_and_keyboard_when_disabled() {
        let mv = InputEvent::MouseMove {
            x_norm: 0.5,
            y_norm: 0.5,
            monitor_id: 0,
        };
        let click = InputEvent::MouseButton {
            button: MouseButton::Left,
            pressed: true,
        };
        let wheel = InputEvent::MouseWheel {
            delta_x: 0.0,
            delta_y: 1.0,
        };
        let key = InputEvent::Key {
            scan_code: 0x1E,
            extended: false,
            pressed: true,
        };

        // Both disabled (default)
        assert!(!is_input_event_permitted(&mv, false, false));
        assert!(!is_input_event_permitted(&click, false, false));
        assert!(!is_input_event_permitted(&wheel, false, false));
        assert!(!is_input_event_permitted(&key, false, false));
        assert!(is_input_event_permitted(&InputEvent::ReleaseAllKeys, false, false));

        // Only mouse enabled
        assert!(is_input_event_permitted(&mv, true, false));
        assert!(is_input_event_permitted(&click, true, false));
        assert!(!is_input_event_permitted(&key, true, false));

        // Only keyboard enabled
        assert!(!is_input_event_permitted(&mv, false, true));
        assert!(!is_input_event_permitted(&click, false, true));
        assert!(is_input_event_permitted(&key, false, true));
    }
}
