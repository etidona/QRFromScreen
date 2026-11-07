// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 etidona

//! Connect to Wi-Fi networks through NetworkManager's D-Bus API.
//!
//! This talks to NetworkManager on the system bus directly, without nmcli,
//! so it also works from a Flatpak sandbox granted
//! `--system-talk-name=org.freedesktop.NetworkManager`.
//!
//! The app only asks NetworkManager to connect: the outcome, and any
//! password prompt, are left to NetworkManager and the desktop.

use crate::content::Security;
use gtk::gio;
use gtk::glib::{self, Variant, prelude::*, variant::ObjectPath};
use std::collections::HashMap;

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS_IFACE: &str = "org.freedesktop.NetworkManager.Settings";
const CONNECTION_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";

const DEVICE_TYPE_WIFI: u32 = 2;
/// NM_DEVICE_STATE_DISCONNECTED: from here on, a device can be activated.
const DEVICE_STATE_USABLE: u32 = 30;
const DBUS_TIMEOUT_MS: i32 = 30_000;

/// One settings section (`a{sv}`), e.g. "802-11-wireless".
type Section = HashMap<String, Variant>;
/// A whole connection profile (`a{sa{sv}}`).
type Settings = HashMap<String, Section>;

/// A Wi-Fi network read from a QR code.
pub struct Network<'a> {
    pub ssid: &'a str,
    pub password: Option<&'a str>,
    pub hidden: bool,
    pub security: Security,
}

/// What was asked of NetworkManager.
#[derive(Debug, PartialEq)]
pub enum Started {
    /// The network was already saved: its profile was activated unchanged.
    Saved,
    /// A new profile was created from the QR code and activated.
    New,
}

/// Ask NetworkManager to connect to the network, without waiting for the result.
pub async fn connect(net: &Network<'_>) -> Result<Started, String> {
    let unavailable = |e| format!("NetworkManager is not available.\n\n{}", message_of(e));
    let bus = gio::bus_get_future(gio::BusType::System).await.map_err(unavailable)?;

    let wifi_on = property(&bus, NM_PATH, NM, "WirelessEnabled").await.map_err(unavailable)?;
    if wifi_on.get::<bool>() == Some(false) {
        return Err("Wi-Fi is turned off. Turn it on and try again.".into());
    }

    let device = wifi_device(&bus).await?.ok_or("No usable Wi-Fi adapter was found.")?;
    let root = ObjectPath::try_from("/").expect("\"/\" is a valid object path");
    let failed = |e| format!("The connection could not be started.\n\n{}", message_of(e));

    // A saved profile is used as it is: the QR code never changes it.
    // NetworkManager picks the device for it.
    if let Some(profile) = saved_profile(&bus, net.ssid).await {
        let args = (profile, root.clone(), root).to_variant();
        call(&bus, NM_PATH, NM, "ActivateConnection", Some(&args)).await.map_err(failed)?;
        return Ok(Started::Saved);
    }

    let args = (new_settings(net), device, root).to_variant();
    call(&bus, NM_PATH, NM, "AddAndActivateConnection", Some(&args)).await.map_err(failed)?;
    Ok(Started::New)
}

/// Build the connection profile for a network that is not saved yet.
fn new_settings(net: &Network<'_>) -> Settings {
    let mut connection = Section::new();
    connection.insert("type".into(), "802-11-wireless".to_variant());
    connection.insert("id".into(), net.ssid.to_variant());

    let mut wireless = Section::new();
    wireless.insert("ssid".into(), net.ssid.as_bytes().to_variant());
    wireless.insert("mode".into(), "infrastructure".to_variant());
    if net.hidden {
        wireless.insert("hidden".into(), true.to_variant());
    }

    let mut settings = Settings::new();
    settings.insert("connection".into(), connection);
    settings.insert("802-11-wireless".into(), wireless);
    if let Some(security) = security_section(net) {
        settings.insert("802-11-wireless-security".into(), security);
    }
    settings
}

/// The "802-11-wireless-security" section, or `None` for an open network.
fn security_section(net: &Network<'_>) -> Option<Section> {
    let mut s = Section::new();
    match net.security {
        Security::Open => return None,
        Security::Wpa => s.insert("key-mgmt".into(), "wpa-psk".to_variant()),
        Security::Wpa3 => s.insert("key-mgmt".into(), "sae".to_variant()),
    };
    if let Some(p) = net.password {
        s.insert("psk".into(), p.to_variant());
    }
    Some(s)
}

/// The first Wi-Fi device that NetworkManager manages and can activate.
async fn wifi_device(bus: &gio::DBusConnection) -> Result<Option<ObjectPath>, String> {
    let reply = call(bus, NM_PATH, NM, "GetDevices", None)
        .await
        .map_err(|e| format!("NetworkManager is not available.\n\n{}", message_of(e)))?;
    for path in paths(&reply.child_value(0)) {
        let get_u32 = async |name| property(bus, &path, DEVICE_IFACE, name).await.ok()?.get::<u32>();
        if get_u32("DeviceType").await == Some(DEVICE_TYPE_WIFI)
            && get_u32("State").await.is_some_and(|s| s >= DEVICE_STATE_USABLE)
        {
            return Ok(ObjectPath::try_from(path).ok());
        }
    }
    Ok(None)
}

/// A saved profile that connects to `ssid` as a client, if any.
async fn saved_profile(bus: &gio::DBusConnection, ssid: &str) -> Option<ObjectPath> {
    let reply = call(bus, SETTINGS_PATH, SETTINGS_IFACE, "ListConnections", None).await.ok()?;
    for path in paths(&reply.child_value(0)) {
        let Ok(reply) = call(bus, &path, CONNECTION_IFACE, "GetSettings", None).await else {
            continue;
        };
        if reply.child_value(0).get::<Settings>().is_some_and(|s| is_client_profile_for(&s, ssid)) {
            return ObjectPath::try_from(path).ok();
        }
    }
    None
}

/// Whether a profile connects to `ssid` as a client (not as a hotspot).
fn is_client_profile_for(settings: &Settings, ssid: &str) -> bool {
    let Some(wireless) = settings.get("802-11-wireless") else {
        return false;
    };
    let same_ssid = wireless.get("ssid").and_then(|v| v.get::<Vec<u8>>()).as_deref() == Some(ssid.as_bytes());
    let mode = wireless.get("mode").and_then(|v| v.str());
    same_ssid && !matches!(mode, Some("ap" | "adhoc" | "mesh"))
}

async fn call(
    bus: &gio::DBusConnection,
    path: &str,
    iface: &str,
    method: &str,
    args: Option<&Variant>,
) -> Result<Variant, glib::Error> {
    // Let polkit ask for a password where creating a system-wide profile needs one.
    let flags = gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION;
    bus.call_future(Some(NM), path, iface, method, args, None, flags, DBUS_TIMEOUT_MS)
        .await
}

/// Read one D-Bus property and unwrap its `v` container.
async fn property(bus: &gio::DBusConnection, path: &str, iface: &str, name: &str) -> Result<Variant, glib::Error> {
    let args = (iface, name).to_variant();
    let reply = call(bus, path, "org.freedesktop.DBus.Properties", "Get", Some(&args)).await?;
    Ok(reply.child_value(0).as_variant().unwrap_or(reply))
}

/// The object paths in an `ao` value.
fn paths(array: &Variant) -> Vec<String> {
    array.iter().filter_map(|v| v.str().map(str::to_owned)).collect()
}

/// A D-Bus error message without the "GDBus.Error:…" prefix.
fn message_of(mut e: glib::Error) -> String {
    gio::DBusError::strip_remote_error(&mut e);
    e.message().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(password: Option<&'static str>, security: Security) -> Network<'static> {
        Network { ssid: "Café", password, hidden: true, security }
    }

    #[test]
    fn new_profile_wpa() {
        let s = new_settings(&net(Some("secret123"), Security::Wpa));
        let v = s.to_variant();
        assert_eq!(v.type_().as_str(), "a{sa{sv}}");
        let wireless = &s["802-11-wireless"];
        assert_eq!(wireless["ssid"].get::<Vec<u8>>().unwrap(), "Café".as_bytes());
        assert_eq!(wireless["ssid"].type_().as_str(), "ay");
        assert_eq!(wireless["hidden"].get::<bool>(), Some(true));
        let sec = &s["802-11-wireless-security"];
        assert_eq!(sec["key-mgmt"].str(), Some("wpa-psk"));
        assert_eq!(sec["psk"].str(), Some("secret123"));
    }

    #[test]
    fn new_profile_other_security() {
        let s = new_settings(&net(None, Security::Open));
        assert!(!s.contains_key("802-11-wireless-security"));

        let s = new_settings(&net(Some("secret123"), Security::Wpa3));
        assert_eq!(s["802-11-wireless-security"]["key-mgmt"].str(), Some("sae"));
    }

    #[test]
    fn matching_saved_profiles() {
        let mut settings = new_settings(&net(Some("x"), Security::Wpa));
        assert!(is_client_profile_for(&settings, "Café"));
        assert!(!is_client_profile_for(&settings, "Cafe"));
        settings.get_mut("802-11-wireless").unwrap().insert("mode".into(), "ap".to_variant());
        assert!(!is_client_profile_for(&settings, "Café"));
    }

    /// Read-only check against the real NetworkManager: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_read_only_probe() {
        glib::MainContext::default().block_on(async {
            let bus = gio::bus_get_future(gio::BusType::System).await.unwrap();
            let device = wifi_device(&bus).await.unwrap();
            println!("Wi-Fi device: {device:?}");
            assert!(device.is_some());
            assert!(saved_profile(&bus, "no-such-network-qr-from-screen").await.is_none());
        });
    }
}
