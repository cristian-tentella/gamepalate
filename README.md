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

- Windows 10 or 11 (Linux and macOS are planned)
- A [Steam Web API key](https://steamcommunity.com/dev/apikey)

## Data

Data lives in `%APPDATA%\gamepalate` (config, votes, corrections, `catalog.sqlite`). Override with `GAMEPALATE_DATA_DIR`.

The API key is stored in Windows Credential Manager, or read from `STEAM_API_KEY`. Logs go to `logs\gamepalate.log` (API keys and Steam IDs are redacted).

## Build

```
cargo build --release -p gamepalate-gui
```

Output: `target/release/gamepalate.exe`. The first run asks for the API key and your Steam profile, then downloads the catalog (this takes a while).

`gamepalate-cli` covers the same features from the terminal. Build it with `cargo build --release -p gamepalate-cli` and run `gamepalate-cli --help` for the commands.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

The bundled Atkinson Hyperlegible Next font is under the SIL Open Font License (`crates/gamepalate-gui/fonts`).
