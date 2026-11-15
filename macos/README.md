# MVault for macOS (native, Rust)

Memoro Vault / MVault as a native macOS app written entirely in Rust: no web engine, no
JavaScript. It looks and works like the Electron MVault (PGPM theme), and the files it makes
are interchangeable with it in both directions: vaults, re-saved vaults and printed QR paper
backups.

- UI: [egui](https://github.com/emilk/egui) / eframe, drawn to match the PGPM theme
- Crypto: Argon2id + AES-256-GCM, byte-compatible with the Electron app's Rust/WASM core
- PDF viewing and paper-backup reading: [hayro](https://github.com/LaurenzV/hayro) +
  [rqrr](https://github.com/WanzenBug/rqrr); paper backups printed with
  [pdf-writer](https://github.com/typst/pdf-writer) + [qrcode](https://github.com/kennytm/qrcode-rust)
- macOS: AppKit menu, AVFoundation for the camera (QR scanning) and audio/video playback

Everything stays in memory. Decrypted files are only written when you save them, and audio
and video play straight from memory (an AVFoundation resource loader hands the bytes over),
so nothing decrypted touches the disk to play.

## Download

Get `Memoro-Vault-<version>.dmg` from the GitHub release, open it and drag
**Memoro Vault** to Applications. The app is signed ad hoc (no Apple Developer ID), so the
first time macOS asks: right-click the app, choose **Open**, then **Open**.

Requirements: macOS 10.15 or later. The release build is for Intel Macs; Apple Silicon
Macs run it through Rosetta (build from source for a native arm64 binary).

## Build

```sh
cargo build -p mvault --release      # target/release/mvault
scripts/package-macos.sh             # dist/Memoro Vault.app and dist/Memoro-Vault-<version>.dmg
```

## Layout

| Path | What |
| --- | --- |
| `core/` | vault format: seal, open, verify, re-seal, recovery (720 orders), lite vaults, paper-backup PDF and its restore, zxcvbn strength estimate, the JS string rules the format depends on |
| `app/` | the app: pages (`app/src/pages`), UI toolkit (`ui.rs`), QR reading (`qrscan.rs`), macOS integration (`macos.rs`) |
| `reference/` | screenshots and layout dumps of every screen of the Electron app, the scripts that captured them, and `compare.py`, which checks the Rust app against them |
| `scripts/` | packaging |

## Tests

```sh
cargo test --workspace --release
```

- `core` unit tests: crypto, the JS string and number rules, recovery order, paper payloads
- `core/tests/compat.rs`: a vault the Electron app built opens here; vaults sealed and
  re-sealed here open with the Electron app's own `vault-core.js` + WASM under Node; the
  strength estimate matches the shipped `zxcvbn.js`. Needs Node and an MVault (Electron)
  checkout: `MVAULT_REPO=/path/to/MVault`, or this folder inside it, or a sibling `MVault`.
- `app` tests: the paper backup the Electron app printed restores here, and one printed here
  restores here. `reference/check-js-restores-rust-pdf.cjs` checks that the Electron app
  restores one printed here.

Screen-by-screen comparison with the Electron app:

```sh
target/release/mvault --snap <scenario> --out /tmp/snaps     # e.g. editor, recover-done, license
python3 reference/compare.py reference/pgpm /tmp/snaps <scenario>
```

## Differences from the Electron app

Fixed here (the Electron app has these bugs):

- In the vault editor and its dialogs the main buttons (Save vault, Save, Add, Keep changes,
  Save new vault, OK, Make paper backup, Choose a location) were dark text on a dark
  background, invisible until hovered. They are bright here, like main buttons elsewhere.
- A chosen multiple-choice answer on the recover page lost its highlight and showed dark text
  on the page colour. It is shown highlighted here.
- A message raised while the Restore Physical Backup dialog is open (the import's "Success",
  camera errors) stayed hidden under it. It shows on top here.
- After one restore from paper, choosing more files read them but never rebuilt the vault.
- Closing and reopening the vault editor after saving went back to the first revision, so
  the next save was numbered (and dated) from the old one. It continues from the latest here.

Not yet the same:

- Audio and video play through AVFoundation, which covers MP4/MOV/M4V (H.264, HEVC) and
  MP3/AAC/M4A/WAV/AIFF/FLAC/ALAC. WebM, Ogg and Opus, which Chromium also plays, show
  "could not be played" (Download still works).
- Text is drawn by egui, so glyph edges differ slightly from Chromium's; layouts match.

## License

GPL-3.0, see [LICENSE](LICENSE) (the same agreement the app shows on first run). Cascadia Mono
is under the SIL Open Font License 1.1 (`app/assets/fonts/CascadiaMono-OFL.txt`).
