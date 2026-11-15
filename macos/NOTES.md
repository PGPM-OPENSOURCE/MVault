# MVault in Rust — porting notes

Goal: 100% Rust, native UI (egui/eframe), no web engine, no JavaScript. The screens must look
identical to the Electron MVault (PGPM theme), and vaults/paper backups must stay byte-compatible
with Memoro Vault / MVault (Electron) in both directions.

Reference implementation: the Electron MVault repository (JS in src/js, CSS in src/css, HTML in src/*.html).
The Electron build there runs on this Mac with Electron 43.8.10 (Electron 44 needs macOS 13).
Run it with ELECTRON_RUN_AS_NODE unset (VS Code's extension host sets it).

## Decisions
- UI: egui/eframe 0.37, custom-painted to match the PGPM CSS. Default (and only reachable) theme
  is PGPM: index.js hides the theme picker, nothing else switches themes. Cypherpunk/Clean only
  apply if an older install saved them in prefs.json.
- Window: macOS Electron build opens 1000x700 (frame) => 1000x672 content.
- Storage: everything in memory; only prefs.json (theme) and eula_accepted_v1.json on disk, in
  ~/Library/Application Support/<app>/.

## Format facts (must match exactly)
- Argon2id v0x13, t=3, m=16384 KiB, p=1, len=32. SALT QUIRK: the bytes given to Argon2 are the
  ASCII of base64-STANDARD-NO-PAD(raw salt), not the raw salt (memoro-crypto lib.rs).
- deriveKey(pw, saltHex): argon2(utf8(pw), b64nopad(hex->bytes)).
- deriveFileKey(concat, fileId): pw = utf8(concat + fileId); salt = SHA-256(pw) -> b64nopad.
- AES-256-GCM, 12-byte IV, tag appended (ciphertext||tag).
- Layer 1: key = deriveKey(unlockConcat = a[g0]+a[g1] (trim+lower), layer1Salt);
  plaintext = JSON {questionList, selectedL2Indices, remainingL2Indices, gateIndices}.
- Full key: permutedConcat = gates + remaining + permutation(selected) joined ''; fullSalt.
  vaultHash = sha256hex(utf8(permutedConcat)).
- Build: layer2 = shuffle(all idx minus gates minus trap); selected = first 6;
  remaining = shuffle(rest + trap). Trap (red herring) answer normalized to "".
- Files: uploads, then seeds (text/plain), then final-message.txt (trimmed, if non-empty).
  meta {id, filename: id+'.vaultdoc', originalName, mimeType, iv:[..], sha256}.
- vault.meta = AES(fullKey, metaIv, JSON {files, seedFiles:[{name,content}], vaultHash, ...extras}).
- vault.json {version:'1.1.0', created ISO(ms,Z), instructions, fullSalt, layer1Salt,
  questionPrompts[2], vaultMetaIv[], argonSettings{time,mem,parallelism,type:'Argon2id',hashLen}}.
- vault.enc JSON {ciphertext:[..], iv:[..]}. JSON files pretty-printed with 2 spaces.
- ZIP order: vault.json, vault.enc (deflate), vault.meta + *.vaultdoc (stored), README.txt,
  Vault_Builder_Instructions.txt (deflate; full vault only).
- questionList item: {question, expectedLength (UTF-16 len of trimmed, null if MCQ/!showLength),
  hintLetters, hintValues (chars of trimmed at 1-based pos), spaceIndexes, custom, showLength,
  mcqEnabled, mcqOptions (shuffled, 6-12, real answer included)}.
- Recovery: answers trim+lower; try all 720 orders of the 6 selected answers against vault.meta.
- Paper backup (mv-lite-v1): lite ZIP (no docs, same salts+permutation) AES-GCM'd with random
  32-byte key; QR "MVKEY|<64hex>" (EC Q), "MVHDR|<b64(json {v,iv,size,sha256})>" (EC Q),
  "MVCT|i/n|<324 b64 chars>" (EC M). Letter PDF, 3 cols, tile 180pt, margin 24, gap 12,
  label pad 14, banner 72; Courier text.

## Tooling
- reference/capture.cjs + capture-editor.cjs: Electron screenshots + layout dumps (reference/pgpm/*.json|png)
- app: `mvault --snap <scenario> --out <dir>` (app/src/snap.rs) -> png + json
- reference/compare.py <ref> <rust> <scenario>: text positions + pixel diff (ref PNG has display ICC; converted to sRGB)
- compare.py needs Python with pillow and numpy (python3 -m venv venv; venv/bin/pip install pillow numpy)
- perl -e 'alarm shift; exec @ARGV' 60 cmd  (no `timeout` on macOS)
- Text metrics: Cascadia 16px, 19px lines, baseline 15; CH=9.375. Fallback for ⋮ = Apple Symbols.
- Outside list markers in Blink sit ~3px right/1px lower than "• " right-aligned (fix later).

## Progress
- [x] Reference screenshots (Electron 43, PGPM) of every screen and dialog
- [x] Core: crypto, vault seal/open/verify, reseal, lite + paper PDF, paper restore (parse/rebuild)
- [x] Cross-compat tests against the JS core (node + shipped WASM), zxcvbn port exact
- [x] Every screen: licence gate, index, dashboard (+ unlock/rename/confirm/offline/message,
      Restore Physical Backup), how, create, setup, answers, build, recover, vault editor, viewer
- [x] Paper backups both ways: app/src/qrscan.rs reads the Electron-printed PDF
      (cargo test -p mvault --release qrscan); the Electron app restores a Rust-printed one
      (reference/check-js-restores-rust-pdf.cjs, PDF from the ignored test write_paper_pdf_for_js)
- [x] macOS bundle + DMG: scripts/package-macos.sh -> dist/ (ad-hoc signed, x86_64)

## Fidelity (reference/compare.py, all 63 scenarios)
- Pixel difference above 24/255: 0.02% (index) to 3.2% (how-scroll1); 59 of 63 under 2.6%.
  What is left is glyph rasterisation (Chromium's text is a little heavier and sits ~1px apart),
  the clock time in the restore log, and Blink's outside list markers (~3px right, 1px lower).
- Text runs line up exactly; "extra" list markers in build-post* are ::marker boxes the
  reference dump does not record as text.

## Electron bugs fixed here (screens differ from the reference on purpose)
- Editor/dialog primary buttons are bright (were #0c0c0c on the page colour, invisible).
- The chosen multiple-choice answer is shown inverted (was dark text with no fill).
- Messages raised under the paper-restore dialog show over it (were hidden under it).
- Paper restore rebuilds again for a second set of files (`__mvAutoRebuildDone` never reset).
- The editor reopens on the latest saved revision (recover.js kept the first session/meta).

## macOS features (app/src/macos.rs)
- App/Edit/Window menu like main.js `macAppMenu`; Edit items become egui events.
- Camera: AVCaptureSession at 1280x720, BGRA frames, scanned every 300ms (paper dialog).
- Audio/video: AVPlayer fed from memory by an AVAssetResourceLoader delegate (custom URL
  scheme), frames from AVPlayerItemVideoOutput. No WebM/Ogg/Opus (AVFoundation limits).
- Startup scrub of web storage left by releases up to 1.0.8 (main.js `scrubLegacyWebStorage`).
