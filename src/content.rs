// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 etidona

//! Recognise what a QR code contains.
//!
//! Anything that could disguise where an action leads (invisible characters,
//! malformed links, unexpected parameters) is either removed or downgraded to
//! plain text, which can only be copied.

use gtk::glib;

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// A web link (http or https).
    Url {
        uri: String,
        /// The site, in ASCII form (international names as `xn--…`).
        host: String,
        /// Tricks found in the link, to show as warnings.
        warnings: Vec<&'static str>,
    },
    /// Wi-Fi credentials: `WIFI:T:WPA;S:name;P:password;H:false;;`
    Wifi {
        ssid: String,
        password: Option<String>,
        hidden: bool,
        security: Security,
    },
    /// `mailto:` link, rebuilt with the address, subject and body only.
    Email {
        uri: String,
        /// Decoded address, to show.
        address: String,
        /// Decoded subject, to show.
        subject: Option<String>,
    },
    /// Anything else.
    Text(String),
}

/// Wi-Fi security type, from the `T:` field of a Wi-Fi code.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Security {
    /// No password (`T:nopass`).
    Open,
    /// WPA/WPA2 personal (`T:WPA`, the most common).
    Wpa,
    /// WPA3 personal (`T:SAE` or `T:WPA3`).
    Wpa3,
}

pub const WARN_USERINFO: &str =
    "The link has a user name before the site (…@), a trick often used to disguise the real site.";
pub const WARN_IDN: &str =
    "The site name uses international characters, which can imitate a different site.";

/// Whether the text has control characters (line breaks included), or
/// invisible or direction-changing characters that can hide its real content.
pub fn has_hidden_chars(s: &str) -> bool {
    s.chars().any(is_hidden_char)
}

/// See [`has_hidden_chars`].
pub fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(c,
            // Unicode's default-ignorable code points, plus line and
            // paragraph separators.
            '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}' | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}' | '\u{3164}' | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF0}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}')
}

/// Case-insensitive prefix strip.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// Work out what a decoded QR code contains.
pub fn parse(raw: &str) -> Content {
    let s = raw.trim();
    let text = || Content::Text(s.to_string());

    if strip_prefix_ci(s, "http://").is_some() || strip_prefix_ci(s, "https://").is_some() {
        return parse_url(s).unwrap_or_else(text);
    }
    if let Some(rest) = strip_prefix_ci(s, "WIFI:") {
        return parse_wifi(rest).unwrap_or_else(text);
    }
    if let Some(rest) = strip_prefix_ci(s, "mailto:") {
        return parse_mailto(rest).unwrap_or_else(text);
    }
    text()
}

/// A well-formed web link without hidden characters.
fn parse_url(s: &str) -> Option<Content> {
    // Browsers read `\` as `/` in web links, unlike GLib: a link with one
    // could open a different site from the one shown. Spaces don't belong
    // in a link either.
    if has_hidden_chars(s) || s.contains(|c: char| c == '\\' || c.is_whitespace()) {
        return None;
    }
    let uri = glib::Uri::parse(s, glib::UriFlags::NONE).ok()?;
    let host = uri.host().filter(|h| !h.is_empty())?;
    let ascii = glib::hostname_to_ascii(&host)?;

    let mut warnings = Vec::new();
    if uri.userinfo().is_some() {
        warnings.push(WARN_USERINFO);
    }
    // GLib may already return the name in ASCII form: look for "xn--" labels.
    if ascii.split('.').any(|label| label.to_ascii_lowercase().starts_with("xn--")) {
        warnings.push(WARN_IDN);
    }
    Some(Content::Url { uri: s.to_string(), host: ascii.to_string(), warnings })
}

/// A `mailto:` link keeping only the address, subject and body: parameters
/// such as `bcc` or `attach` could add hidden recipients or files.
fn parse_mailto(rest: &str) -> Option<Content> {
    if has_hidden_chars(rest) {
        return None;
    }
    let (raw_address, query) = rest.split_once('?').unwrap_or((rest, ""));
    let address = unescape(raw_address).filter(|a| !a.is_empty() && !has_hidden_chars(a))?;

    let (mut kept, mut subject) = (Vec::new(), None);
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key.eq_ignore_ascii_case("subject") {
            // The subject is shown in the dialog: it must be plain, visible text.
            let decoded = unescape(value).filter(|v| !has_hidden_chars(v))?;
            subject = Some(decoded).filter(|v| !v.is_empty());
            kept.push(pair);
        } else if key.eq_ignore_ascii_case("body") {
            // The body is shown by the mail app before sending; line breaks are fine.
            unescape(value)?;
            kept.push(pair);
        }
    }
    let mut uri = format!("mailto:{raw_address}");
    if !kept.is_empty() {
        uri.push('?');
        uri.push_str(&kept.join("&"));
    }
    Some(Content::Email { uri, address, subject })
}

/// Decode `%XX` escapes; `None` if they are malformed or not UTF-8.
fn unescape(s: &str) -> Option<String> {
    glib::Uri::unescape_string(s, None::<&str>).map(String::from)
}

/// Parse the fields of a Wi-Fi code. Values may contain `\;` `\,` `\:` `\\` escapes.
/// Codes without a usable network name, or for enterprise or obsolete WEP
/// networks, give `None`.
fn parse_wifi(fields: &str) -> Option<Content> {
    let (mut ssid, mut password, mut hidden, mut kind) = (None, None, false, None);
    let mut enterprise = false;
    let mut field = String::new();
    let mut chars = fields.chars();

    let mut take = |field: &str| {
        if let Some((key, value)) = field.split_once(':') {
            match key {
                "S" => ssid = Some(value.to_string()),
                "P" if !value.is_empty() => password = Some(value.to_string()),
                "H" => hidden = value.eq_ignore_ascii_case("true"),
                "T" => kind = Some(value.to_ascii_uppercase()),
                // Enterprise (802.1X) networks: EAP method, identity.
                "E" | "I" => enterprise = true,
                _ => {}
            }
        }
    };

    while let Some(c) = chars.next() {
        match c {
            '\\' => field.extend(chars.next()),
            ';' => {
                take(&field);
                field.clear();
            }
            _ => field.push(c),
        }
    }
    take(&field);

    // A name with invisible characters, or with spaces around it, could pass
    // for a known network. Wi-Fi names are at most 32 bytes long.
    let ssid = ssid.filter(|s| {
        !s.is_empty() && s.len() <= 32 && !has_hidden_chars(s) && s.trim() == s
    })?;
    if enterprise || kind.as_deref().is_some_and(|k| k.contains("EAP")) {
        return None;
    }
    let security = match kind.as_deref() {
        Some("NOPASS") => Security::Open,
        Some("WEP") => return None,
        Some("SAE" | "WPA3") => Security::Wpa3,
        Some("") | None if password.is_none() => Security::Open,
        // WPA, WPA2, WPA/WPA2 and anything unknown that comes with a password.
        _ => Security::Wpa,
    };
    Some(Content::Wifi { ssid, password, hidden, security })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Content {
        Content::Text(s.into())
    }

    #[test]
    fn url() {
        assert_eq!(
            parse("https://example.com/a?b=c"),
            Content::Url { uri: "https://example.com/a?b=c".into(), host: "example.com".into(), warnings: vec![] }
        );
        assert!(matches!(parse("HTTP://X.ORG"), Content::Url { .. }));
    }

    #[test]
    fn deceptive_urls() {
        let Content::Url { host, warnings, .. } = parse("https://www.paypal.com:login@evil.example/") else {
            panic!("not a link");
        };
        assert_eq!(host, "evil.example");
        assert_eq!(warnings, vec![WARN_USERINFO]);

        let Content::Url { host, warnings, .. } = parse("https://аpple.com") else {
            panic!("not a link");
        };
        assert!(host.starts_with("xn--"));
        assert_eq!(warnings, vec![WARN_IDN]);

        // Direction overrides and line breaks: only copyable as text.
        assert_eq!(parse("https://evil.com/\u{202E}moc.lapyap"), text("https://evil.com/\u{202E}moc.lapyap"));
        assert_eq!(parse("https://a.com/\nb"), text("https://a.com/\nb"));
        assert_eq!(parse("https://"), text("https://"));
        // Browsers read `\` as `/`: the site shown would not be the one opened.
        assert_eq!(parse("https://evil.com\\@good.com/"), text("https://evil.com\\@good.com/"));
        assert_eq!(parse("https://a.com/b c"), text("https://a.com/b c"));
    }

    #[test]
    fn wifi_with_escapes() {
        assert_eq!(
            parse(r"WIFI:T:WPA;S:My\;Net;P:pa\:ss\\w;H:true;;"),
            Content::Wifi {
                ssid: "My;Net".into(),
                password: Some(r"pa:ss\w".into()),
                hidden: true,
                security: Security::Wpa,
            }
        );
    }

    #[test]
    fn wifi_open_network() {
        assert_eq!(
            parse("WIFI:T:nopass;S:Guest;P:;;"),
            Content::Wifi { ssid: "Guest".into(), password: None, hidden: false, security: Security::Open }
        );
    }

    fn security_of(raw: &str) -> Security {
        match parse(raw) {
            Content::Wifi { security, .. } => security,
            other => panic!("not a Wi-Fi code: {other:?}"),
        }
    }

    #[test]
    fn wifi_security_types() {
        assert_eq!(security_of("WIFI:T:WPA;S:a;P:12345678;;"), Security::Wpa);
        assert_eq!(security_of("WIFI:T:wpa2;S:a;P:12345678;;"), Security::Wpa);
        assert_eq!(security_of("WIFI:T:SAE;S:a;P:12345678;;"), Security::Wpa3);
        assert_eq!(security_of("WIFI:T:WPA3;S:a;P:12345678;;"), Security::Wpa3);
        // No type: guess from the password.
        assert_eq!(security_of("WIFI:S:a;P:12345678;;"), Security::Wpa);
        assert_eq!(security_of("WIFI:S:a;;"), Security::Open);
    }

    #[test]
    fn unusable_wifi_is_text() {
        for raw in [
            "WIFI:T:WPA;P:x;;",
            "WIFI:T:WPA;S:;P:x;;",
            "WIFI:T:WEP;S:a;P:abcde;;",
            "WIFI:T:nopass;S:Casa\u{200B}Mia;;",
            "WIFI:T:nopass;S:CasaMia\u{E0041};;",
            "WIFI:T:nopass;S:CasaMia ;;",
            "WIFI:T:nopass;S:CasaMia\u{00A0};;",
            "WIFI:T:nopass;S:123456789012345678901234567890123;;",
            "WIFI:T:WPA2-EAP;S:Corp;E:PEAP;I:me;P:x;;",
            "WIFI:T:WPA;S:Corp;E:PEAP;P:x;;",
        ] {
            assert_eq!(parse(raw), text(raw));
        }
    }

    #[test]
    fn mail() {
        assert_eq!(
            parse("mailto:a@b.c?subject=hi%20there"),
            Content::Email {
                uri: "mailto:a@b.c?subject=hi%20there".into(),
                address: "a@b.c".into(),
                subject: Some("hi there".into()),
            }
        );
        assert_eq!(
            parse("mailto:a@b.c?bcc=spy@x.y&Body=hello%0Aworld&attach=/home/u/.ssh/id&cc=z@x.y"),
            Content::Email { uri: "mailto:a@b.c?Body=hello%0Aworld".into(), address: "a@b.c".into(), subject: None }
        );
        // The address is shown decoded, so a hidden second recipient is visible.
        let Content::Email { address, .. } = parse("mailto:a@b.c%2Cspy@x.y") else { panic!() };
        assert_eq!(address, "a@b.c,spy@x.y");
        for raw in ["mailto:?to=spy@x.y", "mailto:a@b.c?subject=%0Abcc", "mailto:a@b.c?body=\u{202E}x", "mailto:a@b.c?subject=%ZZ"] {
            assert_eq!(parse(raw), text(raw));
        }
    }

    #[test]
    fn other_content_is_text() {
        for raw in ["hello", "tel:+39123", "SMSTO:+39123:hello", "javascript:alert(1)"] {
            assert_eq!(parse(raw), text(raw));
        }
    }

    #[test]
    fn hidden_chars() {
        assert!(!has_hidden_chars("Café Wi-Fi 5G"));
        assert!(has_hidden_chars("a\nb"));
        assert!(has_hidden_chars("Casa\u{200B}Mia"));
        assert!(has_hidden_chars("x\u{202E}y"));
    }
}
