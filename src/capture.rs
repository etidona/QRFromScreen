// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 etidona

//! Screen capture through the XDG Screenshot portal and QR decoding.

use ashpd::desktop::{ResponseError, screenshot::Screenshot};
use gtk::{gio::{self, prelude::*}, glib};
use std::collections::HashMap;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
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

    // In a Flatpak sandbox the screenshot comes through the document portal,
    // read-only: find the real file to delete it.
    let document = host_document(&path).await;
    let document_id = document.as_ref().map(|(id, _)| id.clone());

    let decode = gio::spawn_blocking(move || {
        // Read the file and delete it at once, so the screenshot never stays
        // on disk, even if decoding takes forever or crashes.
        let bytes = std::fs::read(&path);
        match &document {
            Some((_, host)) => {
                let _ = std::fs::remove_file(host);
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
        let bytes = bytes.map_err(|e| format!("The screenshot could not be read.\n\n{e}"))?;
        decode(&bytes)
    });

    // Some screens (many shapes that look like QR corners) make the
    // decoder very slow: give up rather than spin silently.
    let result = match glib::future_with_timeout(DECODE_TIMEOUT, decode).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("The decoder crashed.".into()),
        Err(_) => Err("Scanning took too long. The screen may contain too many \
                       shapes that look like QR codes."
            .into()),
    };

    // The file has been read by now: the portal entry is no longer needed.
    if let Some(id) = document_id {
        forget_document(&id).await;
    }
    result
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

const DOCUMENTS: &str = "org.freedesktop.portal.Documents";
const DOCUMENTS_PATH: &str = "/org/freedesktop/portal/documents";

/// For a file exported by the document portal (`/run/…/doc/<id>/<name>`),
/// its document ID and its real path on the host.
async fn host_document(path: &Path) -> Option<(String, PathBuf)> {
    let parts: Vec<_> = path.iter().collect();
    if parts.get(1).is_none_or(|p| *p != "run") {
        return None;
    }
    let doc = parts.iter().position(|p| *p == "doc")?;
    let id = parts.get(doc + 1)?.to_str()?.to_string();

    let bus = gio::bus_get_future(gio::BusType::Session).await.ok()?;
    let args = (vec![id.as_str()],).to_variant();
    let reply = documents_call(&bus, "GetHostPaths", &args).await?;
    let mut host = reply.child_value(0).get::<HashMap<String, Vec<u8>>>()?.remove(&id)?;
    // The path comes as a NUL-terminated byte string.
    if host.last() == Some(&0) {
        host.pop();
    }
    Some((id, std::ffi::OsString::from_vec(host).into()))
}

/// Remove a document from the portal's store, so entries don't pile up.
async fn forget_document(id: &str) {
    if let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await {
        let _ = documents_call(&bus, "Delete", &(id,).to_variant()).await;
    }
}

async fn documents_call(bus: &gio::DBusConnection, method: &str, args: &glib::Variant) -> Option<glib::Variant> {
    bus.call_future(Some(DOCUMENTS), DOCUMENTS_PATH, DOCUMENTS, method, Some(args), None, gio::DBusCallFlags::NONE, 5000)
        .await
        .ok()
}
