# MVault

MVault is a modded version of [Memoro Vault](https://github.com/Kasmaristo-Delvakto/memoro-vault) by
Kasmaristo Delvakto, modified in November 2026 and released under the same license, the GNU General
Public License v3 (see [LICENSE](LICENSE)). The vault format, the encryption and the recovery process
are unchanged, so vaults made with either app open in the other.

## Coding language

MVault comes in two builds, and both read and write the same vaults and paper backups:

- **macOS: written entirely in Rust** ([`macos/`](macos)). A native app with no web engine and no
  JavaScript: its screens, the vault format and encryption, PDF and QR-code handling, and audio and
  video playback are all Rust.
- **Windows and Linux: Electron** (the rest of this repository). Written mostly in JavaScript, with
  HTML and CSS for its screens. Its encryption core (Argon2id key derivation and AES-GCM encryption)
  is written in Rust and compiled to WebAssembly.

## macOS app

Download the DMG from the [latest release](https://github.com/PGPM-OPENSOURCE/MVault/releases/latest).
Building and testing are covered in [`macos/README.md`](macos/README.md).
