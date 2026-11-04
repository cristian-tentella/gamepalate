<p align="center"><img src=".github/logo.png" width="128" height="128" alt="gamepalate logo"></p>

<h1 align="center">gamepalate</h1>

gamepalate is a Steam game recommender. It builds a tag profile from your library and playtime, then ranks the Steam catalog against it to produce a list of suggestions. You vote on them, and your votes feed back into the ranking.

The recommendation system runs locally. The only network calls go to Steam.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset=".github/screenshots/discover-dark.png">
  <img src=".github/screenshots/discover-light.png" alt="Discover">
</picture>

## Screens

- **Discover**: recommendations with filtering options
- **Votes**: votes on past recommendations
- **Library**: games list with covers, where you can adjust how each game counts
- **Profile**: top tags and families

<picture>
  <source media="(prefers-color-scheme: dark)" srcset=".github/screenshots/votes-dark.png">
  <img src=".github/screenshots/votes-light.png" alt="Votes">
</picture>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset=".github/screenshots/library-dark.png">
  <img src=".github/screenshots/library-light.png" alt="Library">
</picture>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset=".github/screenshots/profile-dark.png">
  <img src=".github/screenshots/profile-light.png" alt="Profile">
</picture>

## Requirements

- Windows 10 or 11, or Linux x86_64 with glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+, Fedora 36+, Arch, SteamOS 3 in desktop mode). macOS is planned.
- A [Steam Web API key](https://steamcommunity.com/dev/apikey)

## Download

Download `gamepalate-setup.exe` from the [latest release](https://github.com/cristian-tentella/gamepalate/releases/latest) and run it. It installs gamepalate for your user only, with no administrator rights, and adds it to the Start menu. Uninstalling keeps your data.

To skip the installer, download `gamepalate.exe` and run it from any folder. `SHA256SUMS.txt` lists the checksums of the files.

The files are not signed yet, so on the first run Windows SmartScreen shows "Windows protected your PC". Click **More info**, then **Run anyway**.

### Linux

Download `gamepalate-x86_64-linux.tar.gz`, extract it and run `gamepalate` from the extracted folder:

```
tar -xzf gamepalate-x86_64-linux.tar.gz
./gamepalate/gamepalate
```

To add it to the applications menu, copy `gamepalate` and `gamepalate-cli` to `~/.local/bin`, `gamepalate.png` to `~/.local/share/icons/hicolor/256x256/apps`, and `gamepalate.desktop` to `~/.local/share/applications`.

On WSL2 (WSLg), the window shows only through X11 with the software renderer: run it with `WAYLAND_DISPLAY= ICED_BACKEND=tiny-skia ./gamepalate/gamepalate`. Desktop Linux needs nothing extra.

## Data

Data lives in `%APPDATA%\gamepalate` on Windows and `~/.local/share/gamepalate` on Linux (config, votes, corrections, `catalog.sqlite`). Override with `GAMEPALATE_DATA_DIR`.

The API key is stored in Windows Credential Manager, or on Linux in the desktop keyring (GNOME Keyring, KWallet) through Secret Service. It can also be read from `STEAM_API_KEY`, which is the way to go on desktops without a keyring, such as a bare i3 or Sway. Logs go to `logs/gamepalate.log` in the data folder (API keys and Steam IDs are redacted).

Steam is found in its usual folders, both the native install and the Flatpak one.

## Build

Building needs [NASM](https://nasm.us) on the `PATH`, for the AVIF decoder. On Linux it also needs `pkg-config` and the development files of D-Bus, xkbcommon, Wayland and X11. On Ubuntu and Debian:

```
sudo apt install nasm pkg-config libdbus-1-dev libxkbcommon-dev libwayland-dev libx11-dev
```

```
cargo build --release -p gamepalate-gui
```

Output: `target/release/gamepalate.exe` (`target/release/gamepalate` on Linux). The first run asks for the API key and your Steam profile, then downloads the catalog (this takes a while).

`gamepalate-cli` covers the same features from the terminal. Build it with `cargo build --release -p gamepalate-cli` and run `gamepalate-cli --help` for the commands.

To build the installer, build both executables, then compile `crates/gamepalate-gui/gamepalate.iss` with [Inno Setup](https://jrsoftware.org/isinfo.php) 6. Output: `target/installer/gamepalate-setup.exe`.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

The bundled Atkinson Hyperlegible Next font is under the SIL Open Font License, and the bundled Material Symbols icons are under Apache-2.0 (`crates/gamepalate-gui/fonts`).
