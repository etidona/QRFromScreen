// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 etidona

//! Screen capture through the XDG Screenshot portal and QR decoding.

use ashpd::desktop::{ResponseError, screenshot::Screenshot};
use gtk::{gio::{self, prelude::*}, glib};
use std::time::Duration;

/// How long decoding may take before giving up.
const DECODE_TIMEOUT: Duration = Duration::from_secs(10);

/// Capture the whole screen and return the content of every QR code found.
pub async fn scan_screen() -> Result<Vec<String>, String> {
    // A non-interactive screenshot is what every portal backend supports.
    let response = Screenshot::request()
        .interactive(false)
        .modal(false)
        .send()
        .await
        .and_then(|request| request.response())
        .map_err(|e| match e {
            ashpd::Error::Response(ResponseError::Cancelled) => {
                "Taking the screenshot was not allowed.".to_string()
            }
            e => format!(
                "The screenshot portal is not available. Check that an \
                 xdg-desktop-portal backend supporting Screenshot is installed \
                 and enabled for this desktop.\n\n{e}"
            ),
        })?;

    let path = gio::File::for_uri(response.uri().as_str())
        .path()
        .ok_or("The screenshot portal returned a non-local file.")?;

    let decode = gio::spawn_blocking(move || {
        // Read the file and delete it at once, so the screenshot never stays
        // on disk, even if decoding takes forever or crashes.
        let bytes = std::fs::read(&path);
        let _ = std::fs::remove_file(&path);
        let bytes = bytes.map_err(|e| format!("The screenshot could not be read.\n\n{e}"))?;
        decode(&bytes)
    });

    // Some screens (many shapes that look like QR corners) make the
    // decoder very slow: give up rather than spin silently.
    match glib::future_with_timeout(DECODE_TIMEOUT, decode).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("The decoder crashed.".into()),
        Err(_) => Err("Scanning took too long. The screen may contain too many \
                       shapes that look like QR codes."
            .into()),
    }
}

/// Find and decode every QR code in a screenshot, each one listed once.
fn decode(bytes: &[u8]) -> Result<Vec<String>, String> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|e| format!("The screenshot could not be read.\n\n{e}"))?
        .into_luma8();

    let mut codes = Vec::new();
    for grid in rqrr::PreparedImage::prepare(img).detect_grids() {
        let mut bytes = Vec::new();
        if grid.decode_to(&mut bytes).is_ok() {
            let content = to_text(bytes);
            // The same code can appear twice on screen: list it once.
            if !codes.contains(&content) {
                codes.push(content);
            }
        }
    }
    Ok(codes)
}

/// QR codes are normally UTF-8, but the standard's default for binary
/// content is ISO-8859-1: fall back to it rather than dropping the code.
fn to_text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| char::from(b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_encodings() {
        assert_eq!(to_text("Café".as_bytes().to_vec()), "Café");
        assert_eq!(to_text(vec![b'C', b'a', b'f', 0xE9]), "Café");
    }
}
