//! Finding the paper backup's QR codes (dashboard.js `scanPdfFile`, `scanImageFile`,
//! `findAllQRCodesOnCanvas`). PDF pages are drawn by hayro at 4.2x, as the original's pdf.js pass
//! did, and cut along the backup's tile grid; photos and scans are searched whole, in overlapping
//! pieces and once enlarged. rqrr decodes, trying both polarities (jsQR's `attemptBoth`).

/// An 8-bit grey picture (0 = black).
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Gray {
    /// A photo or scan, at most 3000px on its longer side (nearest neighbour, like a canvas
    /// drawn with `imageSmoothingEnabled = false`).
    pub fn from_image(img: &image::DynamicImage) -> Gray {
        let l = img.to_luma_alpha8();
        let (iw, ih) = (l.width() as usize, l.height() as usize);
        let ratio = (3000.0 / iw as f32).min(3000.0 / ih as f32).min(1.0);
        let (w, h) = (((iw as f32 * ratio).floor() as usize).max(1), ((ih as f32 * ratio).floor() as usize).max(1));
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            let sy = (y * ih / h).min(ih - 1);
            for x in 0..w {
                let sx = (x * iw / w).min(iw - 1);
                let p = l.get_pixel(sx as u32, sy as u32);
                // transparent pixels show the white canvas behind them
                let (v, a) = (p[0] as u32, p[1] as u32);
                px.push(((v * a + 255 * (255 - a)) / 255) as u8);
            }
        }
        Gray { w, h, px }
    }

    fn scaled(&self, f: f32) -> Gray {
        let (w, h) = ((self.w as f32 * f).floor() as usize, (self.h as f32 * f).floor() as usize);
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            let sy = ((y as f32 / f) as usize).min(self.h - 1);
            for x in 0..w {
                px.push(self.px[sy * self.w + ((x as f32 / f) as usize).min(self.w - 1)]);
            }
        }
        Gray { w, h, px }
    }
}

/// Every QR code rqrr finds in a rectangle of `g` (dark on light first, then inverted).
pub fn decode(g: &Gray, x: usize, y: usize, w: usize, h: usize) -> Vec<String> {
    if w == 0 || h == 0 || x + w > g.w || y + h > g.h {
        return vec![];
    }
    let run = |invert: bool| -> Vec<String> {
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |xx, yy| {
            let v = g.px[(y + yy) * g.w + x + xx];
            if invert { 255 - v } else { v }
        });
        img.detect_grids().into_iter()
            .filter_map(|grid| grid.decode().ok())
            .map(|(_, s)| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let found = run(false);
    if found.is_empty() { run(true) } else { found }
}

/// The number of pages in a PDF, or why it cannot be read.
pub fn pdf_open(bytes: &[u8]) -> Result<hayro::hayro_syntax::Pdf, String> {
    hayro::hayro_syntax::Pdf::new(bytes.to_vec()).map_err(|e| format!("{e:?}"))
}

/// Page `index` (0-based) drawn at 4.2x.
pub fn pdf_page(pdf: &hayro::hayro_syntax::Pdf, index: usize) -> Option<Gray> {
    let pages = hayro::render_pdf(pdf, 4.2, hayro::hayro_interpret::InterpreterSettings::default(), &hayro::RenderSettings::default(), Some(index..=index))?;
    let p = pages.first()?;
    // premultiplied RGBA on a transparent page, laid on white
    let (w, h) = (p.width() as usize, p.height() as usize);
    let px = p.data().iter().map(|c| {
        let over = 255 - c.a as u32;
        let (r, g, b) = (c.r as u32 + over, c.g as u32 + over, c.b as u32 + over);
        ((r * 299 + g * 587 + b * 114) / 1000).min(255) as u8
    }).collect();
    Some(Gray { w, h, px })
}

/// One PDF page cut along the backup's grid (Letter, 3 columns, 24pt margins, 12pt gaps, a 72pt
/// banner, 14pt labels). Each tile is tried pulled in 2%, then pushed out 8%. `found` gets every
/// code; scanning stops once `done()` says every piece is in. Returns how many codes were read.
pub fn scan_pdf_page(g: &Gray, found: &mut dyn FnMut(String), done: &dyn Fn() -> bool) -> usize {
    let (w, h) = (g.w as f64, g.h as f64);
    let px_per_pt = w / 612.0;
    let margin = (24.0 * px_per_pt).round();
    let gap = (12.0 * px_per_pt).round();
    let label = (14.0 * px_per_pt).round();
    let banner = (72.0 * px_per_pt).round();
    let inner_w = w - 2.0 * margin;
    let tile = ((inner_w - gap * 2.0) / 3.0).floor();
    let used_w = 3.0 * tile + gap * 2.0;
    let start_x = margin + ((inner_w - used_w) / 2.0).floor();
    let row_h = label + tile + gap;
    let mut n = 0;
    let mut try_rect = |x: f64, y: f64, sz: f64| -> bool {
        let inset = (sz * 0.02).round();
        let pad = (sz * 0.08).round();
        let variants = [
            {
                let (x1, y1) = ((x + inset).max(0.0), (y + inset).max(0.0));
                (x1, y1, (w - x1).min(sz - inset * 2.0).max(0.0), (h - y1).min(sz - inset * 2.0).max(0.0))
            },
            {
                let (x0, y0) = ((x - pad).max(0.0), (y - pad).max(0.0));
                (x0, y0, (w - x0).min(sz + pad * 2.0), (h - y0).min(sz + pad * 2.0))
            },
        ];
        for (rx, ry, rw, rh) in variants {
            if rw <= 0.0 || rh <= 0.0 {
                continue;
            }
            let vals = decode(g, rx as usize, ry as usize, rw as usize, rh as usize);
            if !vals.is_empty() {
                for v in vals {
                    found(v);
                    n += 1;
                }
                return true;
            }
        }
        false
    };
    let mut y = margin + banner + label;
    while y + tile <= h - margin {
        for c in 0..3 {
            try_rect(start_x + c as f64 * (tile + gap), y, tile);
        }
        y += row_h;
        if done() {
            break;
        }
    }
    drop(try_rect);
    // not laid out like a printed backup (a rescanned or photographed page): search all of it
    if n == 0 && !done() {
        n += scan_image(g, found);
    }
    n
}

/// A photo or scan: the whole picture, a 4x4 grid, a 6x6 grid with half-step overlaps, and the
/// whole picture enlarged 1.5x for small or soft codes. Returns how many distinct codes it read.
pub fn scan_image(g: &Gray, found: &mut dyn FnMut(String)) -> usize {
    let mut seen = std::collections::HashSet::new();
    let mut add = |v: Vec<String>, found: &mut dyn FnMut(String)| {
        for s in v {
            if seen.insert(s.clone()) {
                found(s);
            }
        }
    };
    add(decode(g, 0, 0, g.w, g.h), found);
    for (grid, overlap) in [(4usize, false), (6usize, true)] {
        let (sx, sy) = (g.w / grid, g.h / grid);
        if sx == 0 || sy == 0 {
            continue;
        }
        let offs_x = if overlap { vec![0, sx / 2] } else { vec![0] };
        let offs_y = if overlap { vec![0, sy / 2] } else { vec![0] };
        for &ox in &offs_x {
            for &oy in &offs_y {
                for gy in 0..grid {
                    for gx in 0..grid {
                        let (x, y) = (gx * sx + ox, gy * sy + oy);
                        if x >= g.w || y >= g.h {
                            continue;
                        }
                        let (w, h) = (sx.min(g.w - x), sy.min(g.h - y));
                        add(decode(g, x, y, w, h), found);
                    }
                }
            }
        }
    }
    let big = g.scaled(1.5);
    add(decode(&big, 0, 0, big.w, big.h), found);
    seen.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The paper backup the Electron app printed for the fixture vault restores here.
    #[test]
    fn restores_the_reference_pdf() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../reference/pgpm/_fixture-lite-QR.pdf");
        let Ok(bytes) = std::fs::read(&p) else { return };
        let pdf = pdf_open(&bytes).unwrap();
        let mut st = mvault_core::paper::PaperState::default();
        for i in 0..pdf.pages().len() {
            let g = pdf_page(&pdf, i).unwrap();
            let mut got = Vec::new();
            scan_pdf_page(&g, &mut |s| got.push(s), &|| false);
            for s in got {
                st.ingest(&s);
            }
        }
        assert!(st.ready(), "{}", st.status_text());
        let zip = st.rebuild().unwrap();
        mvault_core::vault::peek_vault_zip(&zip).unwrap();
    }

    /// Diagnostics: renders MV_PDF_A and MV_PDF_B and compares them page by page.
    #[test]
    #[ignore]
    fn compare_two_pdfs() {
        let a = std::fs::read(std::env::var("MV_PDF_A").unwrap()).unwrap();
        let b = std::fs::read(std::env::var("MV_PDF_B").unwrap()).unwrap();
        let (pa, pb) = (pdf_open(&a).unwrap(), pdf_open(&b).unwrap());
        for i in 0..pa.pages().len() {
            let (ga, gb) = (pdf_page(&pa, i).unwrap(), pdf_page(&pb, i).unwrap());
            let diff = ga.px.iter().zip(&gb.px).filter(|(x, y)| x.abs_diff(**y) > 8).count();
            let (mut na, mut nb) = (0, 0);
            scan_pdf_page(&ga, &mut |_| na += 1, &|| false);
            scan_pdf_page(&gb, &mut |_| nb += 1, &|| false);
            println!("page {i}: {}x{} vs {}x{}, {diff} pixels differ; codes {na} vs {nb}", ga.w, ga.h, gb.w, gb.h);
        }
    }

    /// Writes the fixture vault as a paper backup printed by this app, for the Electron app to
    /// restore (reference/check-js-restores-rust-pdf.cjs):
    ///   MV_PDF_OUT=/tmp/x.pdf cargo test -p mvault --release write_paper_pdf_for_js -- --ignored
    #[test]
    #[ignore]
    fn write_paper_pdf_for_js() {
        let out = std::env::var("MV_PDF_OUT").expect("MV_PDF_OUT");
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../reference/pgpm/_fixture-vault.zip");
        let zip = std::fs::read(p).unwrap();
        std::fs::write(out, mvault_core::paper::build_paper_pdf(&zip).unwrap().bytes).unwrap();
    }

    /// And one this app prints restores too (the build is the same code the editor and the
    /// builder use).
    #[test]
    fn restores_its_own_pdf() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../reference/pgpm/_fixture-vault.zip");
        let Ok(full) = std::fs::read(&p) else { return };
        // any bytes make a fine payload: the paper layer only encrypts and splits them
        let lite = full[..full.len().min(20_000)].to_vec();
        let pdf_bytes = mvault_core::paper::build_paper_pdf(&lite).unwrap().bytes;
        let pdf = pdf_open(&pdf_bytes).unwrap();
        let mut st = mvault_core::paper::PaperState::default();
        for i in 0..pdf.pages().len() {
            let g = pdf_page(&pdf, i).unwrap();
            let mut got = Vec::new();
            scan_pdf_page(&g, &mut |s| got.push(s), &|| false);
            for s in got {
                st.ingest(&s);
            }
        }
        assert!(st.ready(), "{}", st.status_text());
        assert_eq!(st.rebuild().unwrap(), lite);
    }
}
