//! index.html: [Begin], then "Before You Begin" (PGPM goes straight on to the dashboard).

use crate::app::{Shared, layer};
use crate::pages::common::{code, li, tip};
use crate::pages::dashboard::Dashboard;
use crate::ui::{Btn, LINE, Span, col, s};

#[derive(Default)]
pub struct Index {
    pub safety: bool,
}

pub fn show(st: &mut Index, sh: &mut Shared, ui: &mut egui::Ui) {
    if st.safety {
        let mut go = false;
        layer(ui, sh, "safety", true, |pg, w| {
            let h = safety(pg, w, &mut go);
            h
        });
        if go {
            sh.go(crate::app::Page::Dashboard(Dashboard::default()));
        }
        return;
    }
    let mut begin = false;
    layer(ui, sh, "index", true, |pg, _w| {
        // .content: flex column, 22px gap after the (empty) welcome line
        if pg.button("begin", 8.0, 30.0, Btn::new("Begin")).clicked() {
            begin = true;
        }
        49.0
    });
    if begin {
        st.safety = true;
    }
}

fn tips() -> Vec<Vec<Span>> {
    let t2 = col::T2;
    let st = |x: &str| s(x, col::T1);
    vec![
        vec![s("Memoro Vault is ", t2), st("not a wallet"), s(". It is a ", t2), st("last-resort backup"),
             s(" for long-term storage of sensitive data. If you are protecting seed phrases or private keys, only decrypt them in ", t2),
             st("Tails OS"), s(" for maximum security.", t2)],
        vec![s("Avoid using your real name in the questions or when naming your vault. Initials only.", t2)],
        vec![s("The vault’s strength comes from cryptography, but blending in as a plain ", t2), code(".zip"),
             s(" file increases your safety. Don’t draw attention to it.", t2)],
        vec![s("Choose answers that outsiders cannot easily guess. Avoid obvious or common choices.", t2)],
        vec![s("Always start from a clean session. Restart your computer or boot fresh into Tails OS before use.", t2)],
        vec![s("Do not paste decrypted text (like seed phrases) into other apps. Clipboard history can leak sensitive data.", t2)],
    ]
}

/// The safety notice, laid out as index_pgpm.css + the console layer put it. Returns the height.
fn safety(pg: &mut crate::ui::Pg<'_>, width: f32, go: &mut bool) -> f32 {
    let x = 9.0; // overlay padding 8 + the dialog's 1px border
    let w = width - 18.0;
    let mut y = 9.0;
    pg.text(x, y, "Before You Begin", col::WHITE);
    y += LINE * 2.0 + 1.0; // h2 margin, then the h3's 1px top border
    pg.text(x, y, "BEST PRACTICES", col::WHITE);
    y += LINE * 2.0;
    for t in tips() {
        y += tip(pg, x, y, w, &t, col::T3) + 8.0;
    }
    // the "nuke" tip: text, a list, more text
    pg.text(x + 2.0, y, "•", col::T3);
    let tx = x + 16.0;
    let tw = w - 16.0;
    y += pg.para(tx, y, tw, &[s("Memoro Vault includes a “nuke” function", col::T1), s(" that clears traces when you exit:", col::T2)],
                 crate::ui::Ws::Normal, Default::default(), LINE);
    y += 6.0; // ul margin-top (the li's 3px collapses into it)
    let items = [
        "Clipboard contents",
        "Browser storage (IndexedDB, localStorage, sessionStorage), which Memoro keeps in memory only and never writes to disk",
        "Temporary decrypted files",
    ];
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            y += 3.0;
        }
        y += li(pg, tx + 18.0, y, tw - 18.0, &[s(*it, col::T2)], col::T3);
    }
    y += 8.0; // ul margin-bottom
    y += pg.para(tx, y, tw, &[s("This reduces leftovers, but remember: malware or keyloggers can still capture data live. Make sure your internet is turned off before decryption.", col::T2)],
                 crate::ui::Ws::Normal, Default::default(), LINE);
    y += 8.0;
    y += tip(pg, x, y, w, &[s("Only use the official Memoro Vault program. Avoid modified or unofficial versions. Always verify hashes before installation.", col::T2)], col::T3);
    y += 8.0; // the last tip's margin (collapses with .modal-tips' 6px)
    // sticky button bar: 1px top border, 19px above the button, 8px below
    let bar_h = 1.0 + LINE + LINE + 8.0;
    let view_h = pg.ui.clip_rect().height();
    let scroll_top = pg.ui.clip_rect().min.y - pg.o.y;
    let bar_y = y.min(scroll_top + view_h - bar_h).max(y.min(scroll_top + view_h - bar_h));
    let bar_y = if y + bar_h > scroll_top + view_h { (scroll_top + view_h - bar_h).max(0.0) } else { bar_y.max(y) };
    pg.fill(x, bar_y, w, bar_h, col::BG);
    if pg.button("understand", x, bar_y + 1.0 + LINE, Btn::new("I Understand")).clicked() {
        *go = true;
    }
    y + bar_h + 1.0
}
