// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 etidona

//! QR from Screen: capture the screen, find QR codes and ask what to do with them.
//!
//! The app has no main window: it captures the screen, then shows a single
//! confirmation dialog and exits when that dialog is answered.

mod capture;
mod content;
mod nm;

use adw::prelude::*;
use content::{Content, Security};
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

const APP_ID: &str = "io.github.etidona.QRFromScreen";

/// Keeps the application alive until released, since there is no main window.
type Hold = Rc<RefCell<Option<gio::ApplicationHoldGuard>>>;

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_activate(|app| {
        let hold: Hold = Rc::new(RefCell::new(Some(app.hold())));
        // The capture happens before any window is shown, so the app
        // never appears in its own screenshot.
        glib::spawn_future_local(async move {
            match capture::scan_screen().await {
                Ok(codes) => match codes.len() {
                    0 => message("No QR code found", "Make sure the code is fully visible on screen.", hold),
                    1 => confirm(content::parse(&codes[0]), hold),
                    _ => choose(codes, hold),
                },
                Err(e) => message("Scan failed", &e, hold),
            }
        });
    });

    app.run()
}

/// Release the hold: the app exits once nothing else keeps it alive.
fn release(hold: &Hold) {
    hold.borrow_mut().take();
}

/// Build a dialog with a Cancel button and an optional suggested action.
fn dialog(heading: &str, body: &str, action: Option<(&str, &str)>) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("cancel", if action.is_some() { "_Cancel" } else { "_Close" });
    dialog.set_close_response("cancel");
    if let Some((id, label)) = action {
        dialog.add_response(id, label);
        dialog.set_response_appearance(id, adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some(id));
    }
    dialog
}

/// An informational dialog that ends the app when closed.
fn message(heading: &str, body: &str, hold: Hold) {
    let d = dialog(heading, body, None);
    d.connect_response(None, move |_, _| release(&hold));
    d.present(None::<&gtk::Widget>);
}

/// Several codes on screen: let the user pick one.
fn choose(codes: Vec<String>, hold: Hold) {
    let d = dialog(&format!("{} QR codes found", codes.len()), "Which one do you want to use?", None);
    for (i, code) in codes.iter().enumerate() {
        d.add_response(&format!("code{i}"), &button_label(i, code));
    }
    d.set_prefer_wide_layout(true);
    d.connect_response(None, move |_, response| {
        match response.strip_prefix("code").and_then(|i| i.parse::<usize>().ok()) {
            Some(i) => confirm(content::parse(&codes[i]), hold.clone()),
            None => release(&hold),
        }
    });
    d.present(None::<&gtk::Widget>);
}

/// A short, single-line button label, numbered so that codes starting
/// the same way stay distinguishable.
fn button_label(i: usize, code: &str) -> String {
    const MAX: usize = 48;
    let visible = code.chars().map(|c| if content::is_hidden_char(c) { ' ' } else { c });
    let mut short: String = visible.collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    if short.chars().nth(MAX).is_some() {
        short = short.chars().take(MAX).collect();
        short.push('…');
    }
    // Underscores would be read as mnemonics: escape them.
    format!("{}. {}", i + 1, short.replace('_', "__"))
}

/// Ask for confirmation, then act on the content.
fn confirm(content: Content, hold: Hold) {
    let d = match &content {
        Content::Url { uri, host, warnings } => {
            let mut body = format!("Site: {host}\n\n{uri}");
            for w in warnings {
                body.push_str(&format!("\n\n⚠ {w}"));
            }
            dialog("Open this link?", &body, Some(("ok", "_Open")))
        }
        Content::Wifi { ssid, hidden, security, .. } => {
            let mut body = format!("Network: “{ssid}”\nSecurity: {}", match security {
                Security::Open => "none (open network)",
                Security::Wpa => "WPA/WPA2 password",
                Security::Wpa3 => "WPA3 password",
            });
            if *hidden {
                body.push_str("\nHidden network");
            }
            body.push_str("\n\nIf this network is already saved, its saved settings are used.");
            let d = dialog("Connect to this Wi-Fi network?", &body, Some(("ok", "_Connect")));
            d.set_default_response(Some("cancel"));
            d
        }
        Content::Email { address, subject, .. } => {
            let mut body = format!("To: {address}");
            if let Some(subject) = subject {
                body.push_str(&format!("\nSubject: {subject}"));
            }
            let d = dialog("Write an email?", &body, Some(("ok", "_Write")));
            d.set_default_response(Some("cancel"));
            d
        }
        Content::Text(text) => {
            let mut body = text.clone();
            if content::has_hidden_chars(text) {
                body.push_str(
                    "\n\n⚠ This text contains line breaks or invisible characters. \
                     Be careful when pasting it into a terminal.",
                );
            }
            dialog("Text found in the QR code", &body, Some(("ok", "_Copy")))
        }
    };

    if matches!(content, Content::Text(_)) {
        // Wayland compositors ignore clipboard requests from windows without
        // keyboard focus, and libadwaita closes the dialog before emitting
        // "response". Keep it open until the response handler has copied
        // the text, then close it.
        d.set_can_close(false);
        d.connect_close_attempt(|d| {
            let d = d.clone();
            glib::idle_add_local_once(move || {
                d.force_close();
            });
        });
        // Should a close come without a response, still let the app exit.
        let answered = Rc::new(std::cell::Cell::new(false));
        let (flag, h) = (answered.clone(), hold.clone());
        d.connect_closed(move |_| {
            if !flag.get() {
                release(&h);
            }
        });
        d.connect_response(None, move |_, _| answered.set(true));
    }

    d.connect_response(None, move |_, response| {
        if response != "ok" {
            release(&hold);
            return;
        }
        if let Content::Text(text) = &content {
            return copy(text, hold.clone());
        }
        let (content, hold) = (content.clone(), hold.clone());
        glib::spawn_future_local(act(content, hold));
    });
    d.present(None::<&gtk::Widget>);
}

async fn act(content: Content, hold: Hold) {
    match content {
        Content::Url { uri, .. } | Content::Email { uri, .. } => {
            match gtk::UriLauncher::new(&uri).launch_future(None::<&gtk::Window>).await {
                Ok(()) => release(&hold),
                Err(e) => message("Could not open it", &e.to_string(), hold),
            }
        }
        Content::Wifi { ssid, password, hidden, security } => {
            let network = nm::Network { ssid: &ssid, password: password.as_deref(), hidden, security };
            let body = match nm::connect(&network).await {
                Ok(nm::Started::New) => format!("NetworkManager is connecting to “{ssid}”."),
                Ok(nm::Started::Saved) => format!(
                    "“{ssid}” is already saved: NetworkManager is connecting with its saved settings."
                ),
                Err(e) => return message("Connection failed", &e, hold),
            };
            message("Connecting", &body, hold);
        }
        // Text is copied in the dialog's response handler, never here.
        Content::Text(_) => release(&hold),
    }
}

/// Copy to the clipboard. On Wayland the text is served by this process, so
/// the app stays in the background until another app (or a clipboard
/// manager) takes ownership of the clipboard.
fn copy(text: &str, hold: Hold) {
    let Some(display) = gtk::gdk::Display::default() else {
        return release(&hold);
    };
    let clipboard = display.clipboard();
    clipboard.set_text(text);
    clipboard.connect_local_notify(move |cb| {
        if !cb.is_local() {
            release(&hold);
        }
    });
}
