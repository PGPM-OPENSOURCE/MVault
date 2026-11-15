//! QR paper backup "mv-lite-v1" (paper-backup.js, and the restore half of dashboard.js).

use std::collections::{BTreeMap, HashSet};

use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str};
use qrcode::{EcLevel, QrCode};
use serde_json::{Value, json};

use crate::crypto;
use crate::vault::APP_VERSION;
use crate::{Error, Result};

pub const CT_CHARS_PER_TILE: usize = 324;

pub struct PaperPdf {
    pub bytes: Vec<u8>,
    pub tiles: usize,
    pub pages: usize,
    pub sha256: String,
}

/// The three kinds of QR payload for a lite ZIP, freshly encrypted under a random key.
pub struct Payloads {
    pub key: String,
    pub header: String,
    pub tiles: Vec<String>,
    pub sha256: String,
}

pub fn payloads(lite_zip: &[u8]) -> Result<Payloads> {
    let key = zeroize::Zeroizing::new(crypto::random_bytes(32));
    let iv = crypto::random_bytes(12);
    let ct = crypto::aes_gcm_encrypt(&key, &iv, lite_zip)?;
    let sha = crypto::sha256_hex(lite_zip);
    let header = json!({ "v": "mv-lite-v1", "iv": iv, "size": lite_zip.len(), "sha256": sha });
    let header_b64 = crypto::b64_std(serde_json::to_string(&header)?.as_bytes());
    let ct_b64 = crypto::b64_std(&ct);
    let chunks: Vec<&str> = ct_b64.as_bytes().chunks(CT_CHARS_PER_TILE).map(|c| std::str::from_utf8(c).unwrap()).collect();
    let total = chunks.len();
    Ok(Payloads {
        key: format!("MVKEY|{}", crypto::hex(&key)),
        header: format!("MVHDR|{header_b64}"),
        tiles: chunks.iter().enumerate().map(|(i, c)| format!("MVCT|{}/{}|{}", i + 1, total, c)).collect(),
        sha256: sha,
    })
}

/// QR modules as an 8-bit grey image with a 4-module quiet zone, sized like paper-backup.js
/// (about 750 px across, at least 4 px per module).
fn qr_image(text: &str, ec: EcLevel) -> Result<(Vec<u8>, u32)> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), ec).map_err(|e| Error::Format(format!("QR: {e:?}")))?;
    let count = code.width() as u32;
    let desired_px = 256u32.max((180.0f64 * (300.0 / 72.0)).ceil() as u32);
    let ppm = 4u32.max(desired_px / count);
    let margin = 4 * ppm;
    let size = count * ppm + margin * 2;
    let mut px = vec![255u8; (size * size) as usize];
    let colors = code.to_colors();
    for r in 0..count {
        for c in 0..count {
            if colors[(r * count + c) as usize] == qrcode::Color::Dark {
                for y in 0..ppm {
                    let row = ((margin + r * ppm + y) * size + margin + c * ppm) as usize;
                    px[row..row + ppm as usize].fill(0);
                }
            }
        }
    }
    Ok((px, size))
}

/// Encrypts the lite ZIP under a fresh random key and lays the QR tiles out on Letter pages:
/// KEY | HEADER | CT 1 on the first row, three CT tiles per row after that.
pub fn build_paper_pdf(lite_zip: &[u8]) -> Result<PaperPdf> {
    let p = payloads(lite_zip)?;
    let (page_w, page_h) = (612.0f32, 792.0f32);
    const MARGIN: f32 = 24.0;
    const GAP: f32 = 12.0;
    const LABEL_PAD: f32 = 14.0;
    const BANNER_H: f32 = 72.0;
    let inner_w = page_w - 2.0 * MARGIN;
    let cols = 3.0f32;
    let tile = ((inner_w - GAP * (cols - 1.0)) / cols).floor();
    let start_x = MARGIN + ((inner_w - (cols * tile + (cols - 1.0) * GAP)) / 2.0).floor();
    let now = chrono::Local::now();
    let stamp = now.format("%Y-%m-%d %H:%M").to_string();

    // tiles in order with their page positions (y from the top, like jsPDF)
    struct Placed { page: usize, x: f32, y: f32, payload: String, label: String, ec: EcLevel }
    let mut placed = Vec::new();
    let total = p.tiles.len();
    let mut page = 0usize;
    let mut x = start_x;
    let mut y = MARGIN + BANNER_H + LABEL_PAD;
    let row_h = LABEL_PAD + tile;
    placed.push(Placed { page, x, y, payload: p.key.clone(), label: "KEY".into(), ec: EcLevel::Q });
    x += tile + GAP;
    placed.push(Placed { page, x, y, payload: p.header.clone(), label: "HEADER".into(), ec: EcLevel::Q });
    x += tile + GAP;
    let mut ct = 0usize;
    if ct < total {
        placed.push(Placed { page, x, y, payload: p.tiles[ct].clone(), label: format!("CT {}/{}", ct + 1, total), ec: EcLevel::M });
        ct += 1;
    }
    x = start_x;
    y += row_h + GAP;
    while ct < total {
        if y + tile + MARGIN > page_h {
            page += 1;
            x = start_x;
            y = MARGIN + BANNER_H + LABEL_PAD;
        }
        let mut c = 0;
        while c < 3 && ct < total {
            placed.push(Placed { page, x, y, payload: p.tiles[ct].clone(), label: format!("CT {}/{}", ct + 1, total), ec: EcLevel::M });
            ct += 1;
            x += tile + GAP;
            c += 1;
        }
        x = start_x;
        y += row_h + GAP;
    }
    let pages = page + 1;

    let mut pdf = Pdf::new();
    let mut next = 1;
    let mut new_ref = || { let r = Ref::new(next); next += 1; r };
    let catalog = new_ref();
    let tree = new_ref();
    let font = new_ref();
    let font_bold = new_ref();
    let page_ids: Vec<Ref> = (0..pages).map(|_| new_ref()).collect();
    let content_ids: Vec<Ref> = (0..pages).map(|_| new_ref()).collect();
    let image_ids: Vec<Ref> = placed.iter().map(|_| new_ref()).collect();

    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids(page_ids.iter().copied()).count(pages as i32);
    pdf.type1_font(font).base_font(Name(b"Courier")).encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold).base_font(Name(b"Courier-Bold")).encoding_predefined(Name(b"WinAnsiEncoding"));

    let text_w = |s: &str, size: f32| s.len() as f32 * 0.6 * size;
    for pg in 0..pages {
        let mut c = Content::new();
        let text = |c: &mut Content, f: &[u8], size: f32, x: f32, y_top: f32, s: &str| {
            c.begin_text();
            c.set_font(Name(f), size);
            c.next_line(x, page_h - y_top);
            c.show(Str(s.as_bytes()));
            c.end_text();
        };
        text(&mut c, b"F1", 11.0, MARGIN, MARGIN + 12.0, "Memoro Vault - Paper Backup");
        text(&mut c, b"F1", 10.0, MARGIN, MARGIN + 28.0,
             &format!("Built: {stamp} | Version: {APP_VERSION} | Format: mv-lite-v1 | ZIP bytes: {}", lite_zip.len()));
        text(&mut c, b"F1", 10.0, MARGIN, MARGIN + 44.0, &format!("SHA-256 (ZIP): {}", p.sha256));
        let page_line = format!("Page {} | Scan KEY and HEADER on page 1, then all CT tiles", pg + 1);
        text(&mut c, b"F1", 10.0, page_w - MARGIN - text_w(&page_line, 10.0), MARGIN + 12.0, &page_line);
        let mut res_images = Vec::new();
        for (i, t) in placed.iter().enumerate().filter(|(_, t)| t.page == pg) {
            text(&mut c, b"F2", 9.0, t.x + 2.0, t.y - 4.0, &t.label);
            let name = format!("Im{i}");
            c.save_state();
            c.transform([tile, 0.0, 0.0, tile, t.x, page_h - t.y - tile]);
            c.x_object(Name(name.as_bytes()));
            c.restore_state();
            res_images.push((name, image_ids[i]));
        }
        pdf.stream(content_ids[pg], &c.finish());
        let mut page_w_ = pdf.page(page_ids[pg]);
        page_w_.media_box(Rect::new(0.0, 0.0, page_w, page_h));
        page_w_.parent(tree);
        page_w_.contents(content_ids[pg]);
        let mut res = page_w_.resources();
        res.fonts().pair(Name(b"F1"), font).pair(Name(b"F2"), font_bold);
        {
            let mut xo = res.x_objects();
            for (name, id) in &res_images {
                xo.pair(Name(name.as_bytes()), *id);
            }
        }
        res.finish();
        page_w_.finish();
    }
    for (i, t) in placed.iter().enumerate() {
        let (px, size) = qr_image(&t.payload, t.ec)?;
        let deflated = miniz_oxide::deflate::compress_to_vec_zlib(&px, 9);
        let mut img = pdf.image_xobject(image_ids[i], &deflated);
        img.filter(Filter::FlateDecode);
        img.width(size as i32);
        img.height(size as i32);
        img.color_space().device_gray();
        img.bits_per_component(8);
        img.interpolate(false);
        img.finish();
    }
    Ok(PaperPdf { bytes: pdf.finish(), tiles: total, pages, sha256: p.sha256 })
}

// ---------------------------------------------------------------------------------------------
// Restore: QR text -> pieces -> lite ZIP
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub iv: Vec<u8>,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Key(String),
    Header(Header),
    Ct { idx: usize, total: usize, b64: String },
}

/// dashboard.js `parseQRText` (strict shapes: a forged header gets nothing through).
pub fn parse_qr_text(txt: &str) -> Option<Piece> {
    let txt = crate::jsstr::js_trim(txt);
    if let Some(rest) = txt.strip_prefix("MVKEY|") {
        let hex = crate::jsstr::js_trim(rest).to_lowercase();
        if hex.len() == 64 && hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Some(Piece::Key(hex));
        }
        return None;
    }
    if let Some(rest) = txt.strip_prefix("MVHDR|") {
        let raw = crypto::b64_decode_loose(crate::jsstr::js_trim(rest)).ok()?;
        let v: Value = serde_json::from_slice(&raw).ok()?;
        let iv = v.get("iv")?.as_array()?;
        if iv.len() != 12 || !iv.iter().all(|b| b.as_u64().is_some_and(|n| n <= 255) && b.as_f64().is_some_and(|f| f.fract() == 0.0)) {
            return None;
        }
        let size = v.get("size")?.as_f64()?;
        if size.fract() != 0.0 || !(0.0..=10.0 * 1024.0 * 1024.0).contains(&size) {
            return None;
        }
        let sha = v.get("sha256")?.as_str()?;
        if sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
            return None;
        }
        return Some(Piece::Header(Header { iv: iv.iter().map(|b| b.as_u64().unwrap() as u8).collect(), size: size as u64, sha256: sha.to_string() }));
    }
    if let Some(rest) = txt.strip_prefix("MVCT|") {
        let (idx, rest) = rest.split_once('/')?;
        let (total, b64) = rest.split_once('|')?;
        if idx.is_empty() || total.is_empty() || !idx.bytes().all(|c| c.is_ascii_digit()) || !total.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        if b64.is_empty() || !b64.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c.is_whitespace()) {
            return None;
        }
        let idx: usize = idx.parse().ok()?;
        let total: usize = total.parse().ok()?;
        if idx < 1 || total < 1 || idx > total || total > 10_000 {
            return None;
        }
        return Some(Piece::Ct { idx, total, b64: crate::jsstr::js_trim(b64).to_string() });
    }
    None
}

#[derive(Default)]
pub struct PaperState {
    pub header: Option<Header>,
    pub key_hex: Option<String>,
    pub tiles: BTreeMap<usize, String>,
    pub total_tiles: Option<usize>,
    seen: HashSet<String>,
}

impl PaperState {
    /// dashboard.js `ingestQR`; returns the status line to log, if any.
    pub fn ingest(&mut self, raw: &str) -> Option<String> {
        if raw.is_empty() || !self.seen.insert(raw.to_string()) {
            return None;
        }
        match parse_qr_text(raw)? {
            Piece::Key(hex) => {
                if self.key_hex.is_none() {
                    self.key_hex = Some(hex);
                    return Some("Captured MVKEY (key).".into());
                }
            }
            Piece::Header(h) => {
                if self.header.is_none() {
                    self.header = Some(h);
                    return Some("Captured MVHDR (header).".into());
                }
            }
            Piece::Ct { idx, total, b64 } => {
                let mut msg = None;
                match self.total_tiles {
                    None => self.total_tiles = Some(total),
                    Some(t) if t != total => msg = Some(format!("Conflicting CT totals seen (was {t}, now {total}). Keeping first.")),
                    _ => {}
                }
                if let std::collections::btree_map::Entry::Vacant(e) = self.tiles.entry(idx) {
                    e.insert(b64);
                    return Some(format!("Captured MVCT tile {idx}/{}.", self.total_tiles.unwrap_or(total)));
                }
                return msg;
            }
        }
        None
    }

    pub fn ready(&self) -> bool {
        self.header.is_some() && self.key_hex.is_some() && self.total_tiles.is_some_and(|t| self.tiles.len() == t)
    }

    pub fn percent(&self) -> u32 {
        let mut p = 0u32;
        if self.header.is_some() { p += 25; }
        if self.key_hex.is_some() { p += 25; }
        if let Some(t) = self.total_tiles {
            p += 50u32.min((50.0 * (self.tiles.len() as f64 / t as f64)).round() as u32);
        }
        p.min(100)
    }

    pub fn status_text(&self) -> String {
        format!(
            "Status — Header: {} | Tiles: {}/{} | KEY: {}",
            if self.header.is_some() { "yes" } else { "no" },
            self.tiles.len(),
            self.total_tiles.map(|t| t.to_string()).unwrap_or_else(|| "?".into()),
            if self.key_hex.is_some() { "yes" } else { "no" },
        )
    }

    /// dashboard.js `rebuildLiteFromPaper` (without the download): the verified lite ZIP bytes.
    pub fn rebuild(&self) -> Result<Vec<u8>> {
        let header = self.header.as_ref().ok_or_else(|| Error::Format("Missing header (MVHDR).".into()))?;
        let key_hex = self.key_hex.as_ref().ok_or_else(|| Error::Format("Missing key (MVKEY).".into()))?;
        let total = self.total_tiles.ok_or_else(|| Error::Format("Tiles not detected yet.".into()))?;
        let mut ct = Vec::new();
        for i in 1..=total {
            let part = self.tiles.get(&i).ok_or_else(|| Error::Format(format!("Missing ciphertext tile {i}/{total}.")))?;
            ct.extend(crypto::b64_decode_loose(part)?);
        }
        let key = zeroize::Zeroizing::new(crypto::unhex(key_hex)?);
        let zip = crypto::aes_gcm_decrypt(&key, &header.iv, &ct)?;
        if zip.len() as u64 != header.size {
            return Err(Error::Format(format!("Size mismatch (expected {}, got {}).", header.size, zip.len())));
        }
        let got = crypto::sha256_hex(&zip);
        if got != header.sha256 {
            return Err(Error::Format(format!("SHA-256 mismatch. Expected {}, got {got}.", header.sha256)));
        }
        Ok(zip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn payload_roundtrip() {
        let lite = crypto::random_bytes(5000);
        let p = payloads(&lite).unwrap();
        let mut st = PaperState::default();
        for t in p.tiles.iter().rev() {
            st.ingest(t);
        }
        st.ingest(&p.header);
        assert!(!st.ready());
        st.ingest(&p.key);
        assert!(st.ready());
        assert_eq!(st.percent(), 100);
        assert_eq!(st.rebuild().unwrap(), lite);
        assert!(parse_qr_text("MVKEY|xyz").is_none());
        assert!(parse_qr_text("MVCT|0/3|abc").is_none());
    }
    #[test]
    fn pdf_builds() {
        let lite = crypto::random_bytes(3000);
        let pdf = build_paper_pdf(&lite).unwrap();
        assert!(pdf.bytes.starts_with(b"%PDF-"));
        assert!(pdf.tiles >= 1);
    }
}
