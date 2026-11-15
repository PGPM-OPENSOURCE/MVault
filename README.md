# MVault

MVault is a modded version of [Memoro Vault](https://github.com/Kasmaristo-Delvakto/memoro-vault) by
Kasmaristo Delvakto, modified in November 2026 and released under the same license, the GNU General
Public License v3 (see [LICENSE](LICENSE)). The vault format, the encryption and the recovery process
are unchanged, so vaults made with either app open in the other.

## Coding language

MVault is a desktop app built with Electron. It is written mostly in JavaScript, with HTML and CSS for
its screens. Its encryption core (Argon2id key derivation and AES-GCM encryption) is written in Rust
and compiled to WebAssembly.

## macOS app (native, Rust)

[`macos/`](macos) holds MVault for macOS as a native app written entirely in Rust: the same
screens and the same vault, re-save and paper-backup formats as the Electron app, without a
web engine. Its README covers download, building and testing; the DMG is attached to the
release.
