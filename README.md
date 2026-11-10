# QR from Screen

Scan the QR codes shown on your screen and act on them, like the QR reader of a phone.

Press a keyboard shortcut. QR from Screen takes a screenshot, finds the QR codes in it and asks what to do:

| QR code content | Action |
|---|---|
| Web link (`http`, `https`) | Open it in your browser |
| Wi-Fi network (`WIFI:`) | Connect to it through NetworkManager |
| Email (`mailto:`) | Write it in your mail app |
| Anything else | Copy the text |

Nothing happens without your confirmation. If the screen shows more than one code, you choose which one to use.

## Features

- **No window to manage.** It scans the screen, shows a single dialog and quits.
- **Works on any Wayland desktop** with an [XDG desktop portal](https://flatpak.github.io/xdg-desktop-portal/) that can take screenshots: GNOME, KDE Plasma, niri, Hyprland, Sway and others. No extra configuration is needed.
- **Leaves nothing behind.** The screenshot is read into memory and deleted at once.
- **Safe by design:**
  - links show the real site they open, with a warning about common tricks such as `user@site` addresses or look-alike international characters;
  - email links keep only the address, subject and body, never hidden recipients or attachments;
  - Wi-Fi networks you have already saved are used as they are, and are never changed by a QR code;
  - any content with invisible or direction-changing characters can only be copied, never opened.

## Requirements

- GTK 4.10 or later and libadwaita 1.6 or later
- `xdg-desktop-portal` with a backend that supports the Screenshot portal (for example `xdg-desktop-portal-gnome`, `-kde`, `-hyprland` or `-wlr`)
- NetworkManager, only to connect to Wi-Fi networks

## Install the Flatpak

Download the package for your computer from the [latest release](https://github.com/etidona/QRFromScreen/releases/latest): `x86_64` for most PCs, `aarch64` for ARM computers such as Apple Silicon Macs running Asahi Linux. Then install it:

```sh
flatpak install --user qr-from-screen-0.1.0-x86_64.flatpak
```

Flatpak downloads the GNOME runtime from [Flathub](https://flathub.org/setup) if it is not installed yet. The app is not on Flathub itself, so updates are not automatic: install the package of each new release in the same way.

To run it from a shortcut, use the command `flatpak run io.github.etidona.QRFromScreen`.

The Flatpak can talk to NetworkManager, to connect to Wi-Fi networks, and can access the `Pictures/Screenshots` folder, to delete the screenshot it takes. If your desktop saves screenshots somewhere else, they are not deleted: allow that folder with `flatpak override --user --filesystem=<folder> io.github.etidona.QRFromScreen`.

## Build and install

You need Rust 1.85 or later and the GTK 4 and libadwaita development packages (`gtk4-devel` and `libadwaita-devel` on Fedora; `libgtk-4-dev` and `libadwaita-1-dev` on Debian and Ubuntu).

```sh
cargo build --release
```

To install it for your user:

```sh
install -Dm755 target/release/qr-from-screen ~/.local/bin/qr-from-screen
install -Dm644 data/io.github.etidona.QRFromScreen.desktop -t ~/.local/share/applications/
install -Dm644 data/icons/hicolor/scalable/apps/io.github.etidona.QRFromScreen.svg -t ~/.local/share/icons/hicolor/scalable/apps/
install -Dm644 data/icons/hicolor/symbolic/apps/io.github.etidona.QRFromScreen-symbolic.svg -t ~/.local/share/icons/hicolor/symbolic/apps/
```

## Add a keyboard shortcut

QR from Screen is meant to be run from a shortcut. Assign one to the `qr-from-screen` command (or `flatpak run io.github.etidona.QRFromScreen` for the Flatpak) in your desktop's keyboard settings. For niri, add this to the `binds` section of your configuration:

```kdl
Ctrl+Shift+Q hotkey-overlay-title="Scan QR Code from Screen" { spawn "qr-from-screen"; }
```

If `~/.local/bin` is not in your compositor's `PATH`, use the full path to the command.

## Notes

- On Wayland, copied text is served by the app that copied it. Without a clipboard manager, QR from Screen stays in the background after copying, until something else is copied.
- Codes for WEP and enterprise (802.1X) Wi-Fi networks are shown as text.

## License

QR from Screen is free software, released under the [GNU General Public License v3.0 or later](LICENSE).
