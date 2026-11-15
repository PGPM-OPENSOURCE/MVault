//! how-it-works.html: the preface and the explanation, then [Return to Dashboard].

use crate::app::{Shared, layer};
use crate::pages::dashboard::Dashboard;
use crate::ui::{Btn, LINE, Span, Ws, WrapOpts, col, s};

#[derive(Default)]
pub struct How;

const SECTIONS: &[(&str, &str)] = &[
    ("TL;DR", "Memoro Vault is an offline, self-contained encryption tool that protects digital assets such as wallet keys, personal messages, and sensitive files. Instead of passwords or logins, it uses memory-based questions chosen by you. Unlocking requires not only the correct answers but also the correct sequence, with the option of adding misleading questions to confuse attackers. Files are encrypted with industry-standard AES-256 and can be backed up physically using scannable QR codes. Without your knowledge of the answers in the right order, the vault cannot be decrypted by anyone, not even its creators."),
    ("Why Memoro Vault Is Necessary", "In the digital age, personal and financial data are fragile. They are stored on devices that fail, entrusted to companies that can shut down, or kept in cloud services that may be compromised. Even legal systems cannot guarantee long-term accessibility for heirs or collaborators. Memoro Vault addresses this challenge by creating a durable, self-sovereign container for valuable information that survives beyond individual accounts, jurisdictions, or corporate failures."),
    ("How the Encryption Works", "When you build a vault, you create between four and twenty-five personal questions. These may be obscure facts, family memories, or shared knowledge among trusted individuals. The answers themselves are never stored. Instead, they are processed through Argon2id, a memory-hard key derivation function specifically designed to resist brute-force attempts, even from attackers with specialized hardware. This process ensures that only someone with the correct knowledge can reconstruct the encryption key."),
    ("Red Herrings and Required Order", "To further strengthen security, the system allows the inclusion of a red herring question. A red herring looks plausible but is intentionally misleading, ensuring that attackers cannot succeed by guessing likely answers. Additionally, Memoro Vault requires that answers be entered in the exact original sequence. This sequence is enforced by a six-answer permutation model that introduces seven hundred and twenty possible arrangements, dramatically increasing complexity for unauthorized attempts."),
    ("File and Vault Integrity", "Every file inside the vault, along with its name, metadata, and any final messages, is encrypted using AES-256 in Galois/Counter Mode (GCM). This method provides both confidentiality and authenticity, meaning that no one can read or alter the data without detection. The encrypted content is packaged into a portable, tamper-evident ZIP archive. If the archive is modified in any way, recovery becomes impossible, thereby guaranteeing the integrity of what was originally sealed."),
    ("Physical Backups Using QR Codes", "Memoro Vault also supports the generation of physical paper backups. These backups divide the encrypted vault into QR-encoded tiles that can be printed and stored in a secure location or distributed among trusted individuals. Even if all digital copies are lost, the vault can be reconstructed entirely from these printed pages, requiring no USB drives or online storage. This feature ensures resilience against hardware failure or digital decay."),
    ("Viewing and Editing a Vault", "Once a vault is open, its photos, PDFs, text, audio and video can be viewed inside Memoro Vault, so nothing has to be saved to disk just to look at it. The vault editor lets you add, rename and remove files, change seed phrases and rewrite the final message without rebuilding the vault from scratch. Saving seals everything again with the same questions and answers, but with fresh salts, fresh encryption and a new hidden answer order, then re-opens the new file and checks every item before it is handed to you. Older copies still open with the same answers and still hold their old contents, so replace every stored copy (and reprint the paper backup) after you save."),
    ("Practical Applications", "Memoro Vault can safeguard cryptocurrency seed phrases, personal letters, creative works, or legal instructions. It can preserve family history for future generations, enable trusted recovery without sharing accounts, and provide resistance against theft or coercion by incorporating misleading questions and strict permutations. Above all, it empowers individuals to maintain absolute control over their digital legacy."),
];

pub fn show(_st: &mut How, sh: &mut Shared, ui: &mut egui::Ui) {
    let mut back = false;
    layer(ui, sh, "how", true, |pg, width| {
        let x = 8.0;
        let w = width - 16.0;
        let para = |pg: &mut crate::ui::Pg<'_>, x: f32, y: f32, w: f32, spans: &[Span]| pg.para(x, y, w, spans, Ws::Normal, WrapOpts::default(), LINE);
        let mut y = 8.0;
        pg.text(x, y, "PREFACE", col::WHITE);
        y += 2.0 * LINE;
        // the preface box: a 1px (invisible) border, no padding
        let (bx, bw) = (x + 1.0, w - 2.0);
        y += 1.0;
        y += para(pg, bx, y, bw, &[
            s("“Cypherpunks write code. We know that someone has to write software to defend privacy… We cannot expect governments, corporations, or other large, faceless organizations to grant us privacy… We must defend our own privacy if we expect to have any.”", col::T1),
            s(" — Eric Hughes, ", col::T2),
            s("A\u{a0}Cypherpunk’s\u{a0}Manifesto\u{a0}(1993)", col::T2),
        ]) + 16.0;
        y += para(pg, bx, y, bw, &[s("Memoro Vault is written in that same spirit — code designed to empower people. Security rests in the hands of its users: a vault can be trivial to break or virtually impossible, depending on the strength of the answers chosen. The power to encrypt money, messages, and memories is invaluable — the ability to hide in plain sight, to lock away what matters, and to decide who, if anyone, ever sees it again.", col::T2)]) + 16.0;
        y += para(pg, bx, y, bw, &[s("Freedom always comes with risk. Memoro Vault provides the lock, but the strength of that lock is yours to determine.", col::T2)]);
        y += 24.0;
        pg.text(bx, y, "“Libereco ne estas donaco. Ĝi estas devo.”", col::T3);
        y += LINE;
        pg.text(bx, y, "Kaŝmaristo Delvakto", col::T3);
        y += LINE + 1.0;
        pg.text(x, y, "How Memoro Vault Works", col::WHITE);
        y += 2.0 * LINE;
        for (title, body) in SECTIONS {
            y += 1.0; // h2's top border
            pg.text(x, y, &title.to_uppercase(), col::WHITE);
            y += 2.0 * LINE;
            y += para(pg, x, y, w, &[s(*body, col::T2)]) + 16.0;
        }
        y += para(pg, x, y, w, &[
            s("There are no backdoors and no exceptions.", col::T1),
            s(" Decryption is mathematically impossible without the exact answers in the correct order and the original vault file. Memoro Vault does not rely on any external server or authority, making it an enduring safeguard for digital sovereignty.", col::T2),
        ]) + 16.0;
        y += 40.0 - 16.0; // .button-container margin-top 40px (the p's 16px collapses into it)
        if pg.button("back", x, y, Btn::new("Return to Dashboard")).clicked() {
            back = true;
        }
        y + LINE + 8.0
    });
    if back {
        sh.go(crate::app::Page::Dashboard(Dashboard::default()));
    }
}
