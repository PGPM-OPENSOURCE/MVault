'use strict';
const fs = require('fs');
const os = require('os');
const path = require('path');
const L = require('./lib.cjs');
const { H, REPO, launchApp, goto, waitForDownload, downloadCount, QUESTIONS, ANSWERS, GATES, TRAP, SEED, FINAL, log, snap, snapHover, step, scrollTo, scrollEl } = L;
const OUT = path.resolve(process.argv[2] || 'reference/pgpm');
const THEME = process.argv[3] || 'pgpm';
L.setOut(OUT);

(async () => {
  // ---- 1) License window (first run) ----
  await step('license', async () => {
    const lic = await launchApp(REPO, { acceptEula: false, theme: null });
    const page = await lic.app.firstWindow();
    await page.waitForLoadState('domcontentloaded');
    await page.waitForTimeout(800);
    const size = await lic.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].getContentSize());
    log('license content size', size);
    await snap({ page }, 'license');
    await snapHover({ page }, 'license-hover-accept', '#acceptBtn');
    await lic.app.close();
  });

  // ---- 2) Main app ----
  const ctx = await launchApp(REPO, { theme: THEME });
  const size = await ctx.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].getContentSize());
  log('main content size', size);
  fs.writeFileSync(path.join(OUT, '_window.json'), JSON.stringify({ contentSize: size, theme: THEME }));

  try {
    await step('index', async () => {
      await goto(ctx, 'index.html');
      await snap(ctx, 'index');
      await snapHover(ctx, 'index-hover-begin', '#startBtn');
      await ctx.page.click('#startBtn');
      await ctx.page.locator('#safetyModal').waitFor({ state: 'visible' });
      await snap(ctx, 'index-safety');
      await scrollEl(ctx, '#safetyModal', 100000);
      await snap(ctx, 'index-safety-bottom');
    });

    await step('dashboard', async () => {
      await goto(ctx, 'dashboard.html');
      await snap(ctx, 'dashboard');
      await snapHover(ctx, 'dashboard-hover-create', 'button[data-arg="create"]');
      await ctx.page.click('button[data-arg="create"]');
      await snap(ctx, 'dashboard-offline');
      await ctx.page.click('button[data-action="cancelOfflineWarning"]');
      await ctx.page.click('button[data-action="openPaperDecrypt"]');
      await snap(ctx, 'dashboard-paper');
      await ctx.page.click('button[data-action="closePaperDecrypt"]');
    });

    await step('how-it-works', async () => {
      await goto(ctx, 'how-it-works.html');
      await snap(ctx, 'how');
      const h = await ctx.page.evaluate(() => document.scrollingElement.scrollHeight);
      for (let y = 600, i = 1; y < h; y += 600, i++) { await scrollTo(ctx, y); await snap(ctx, `how-scroll${i}`); }
    });

    await step('create-vault', async () => {
      await goto(ctx, 'create-vault.html');
      await snap(ctx, 'create');
      await ctx.page.locator('#questionSlider').fill('20');
      await ctx.page.evaluate(() => window.updateQuestionCount());
      await snap(ctx, 'create-20');
    });

    await step('setup-questions', async () => {
      await ctx.page.evaluate(() => { localStorage.setItem('customQuestionCount', '12'); localStorage.removeItem('questions'); });
      await goto(ctx, 'setup-questions.html');
      await snap(ctx, 'setup');
      await ctx.page.click('#unlock-0'); await ctx.page.click('#unlock-3');
      await snap(ctx, 'setup-checked');
      await scrollTo(ctx, 100000); await snap(ctx, 'setup-bottom'); await scrollTo(ctx, 0);
      await ctx.page.click('button[data-action="openModal"][data-arg="1"]');
      await snap(ctx, 'setup-edit');
      await ctx.page.selectOption('#categoryDropdown', 'Pets');
      await snap(ctx, 'setup-edit-suggest');
      await ctx.page.click('button[data-action="closeModal"]');
      await ctx.page.click('#helpBtn');
      await snap(ctx, 'setup-help');
      await ctx.page.click('#helpCloseBtn');
      await ctx.page.click('#unlock-3');
      await ctx.page.click('button[data-action="showDisclaimer"]');
      await snap(ctx, 'setup-msg');
      await ctx.page.click('#messageModal button');
      await ctx.page.click('#unlock-3');
      await ctx.page.click('button[data-action="showDisclaimer"]');
      await snap(ctx, 'setup-disclaimer');
    });

    await step('answer-questions', async () => {
      await ctx.page.evaluate(({ q, gates }) => {
        localStorage.setItem('questions', JSON.stringify(q));
        localStorage.setItem('questionCount', String(q.length));
        localStorage.setItem('unlockQuestions', JSON.stringify(gates));
        localStorage.setItem('selectedVaultType', 'docs-only');
        localStorage.removeItem('namedSeeds'); localStorage.removeItem('uploadedFileMeta');
      }, { q: QUESTIONS, gates: GATES });
      await goto(ctx, 'answer-questions.html');
      await ctx.page.waitForTimeout(500);
      await snap(ctx, 'answers');
      for (let i = 0; i < ANSWERS.length; i++) await ctx.page.fill(`#answer-${i}`, ANSWERS[i].toLowerCase());
      await ctx.page.locator(`.trap-checkbox[data-index="${TRAP}"]`).check();
      await scrollTo(ctx, 0);
      await snap(ctx, 'answers-filled');
      const h = await ctx.page.evaluate(() => document.scrollingElement.scrollHeight);
      for (let y = 600, i = 1; y < h; y += 600, i++) { await scrollTo(ctx, y); await snap(ctx, `answers-scroll${i}`); }
      await ctx.page.click('summary.entropy-summary-toggle');
      await ctx.page.locator('summary.entropy-summary-toggle').scrollIntoViewIfNeeded();
      await snap(ctx, 'answers-entropy-open');
      await ctx.page.evaluate(() => { const d = document.querySelector('.entropy-body'); d && d.scrollIntoView(); });
      await snap(ctx, 'answers-entropy-body');
      await ctx.page.click('summary.entropy-summary-toggle');
      await ctx.page.fill('#newSeedName', 'BTC Cold Storage');
      await ctx.page.fill('#newSeedContent', SEED);
      await ctx.page.click('button[data-action="saveNamedSeed"]');
      const workDir = fs.mkdtempSync(path.join(os.tmpdir(), 'mv-refs-'));
      const note = path.join(workDir, 'accounts.txt');
      fs.writeFileSync(note, 'Accounts\n========\nCredit union checking (ends 2231)\n');
      await ctx.page.setInputFiles('#fileUpload', note);
      await ctx.page.waitForTimeout(500);
      await ctx.page.fill('#finalMessage', FINAL);
      await ctx.page.locator('#multiSeedSection').scrollIntoViewIfNeeded();
      await snap(ctx, 'answers-seed-upload');
      await scrollTo(ctx, 100000);
      await snap(ctx, 'answers-bottom');
      await scrollTo(ctx, 0);
      await ctx.page.click('#hint-options-4');
      await snap(ctx, 'answers-hint');
      await ctx.page.selectOption('#numLetters', '1');
      await ctx.page.evaluate(() => window.buildLetterSelectors());
      await ctx.page.fill('#customHintText', 'a very common town name');
      await snap(ctx, 'answers-hint-filled');
      await ctx.page.click('#mcqEnabled');
      await snap(ctx, 'answers-hint-mcq');
      await ctx.page.click('#mcqEnabled');
      await ctx.page.click('#hintSaveBtn');
      await snap(ctx, 'answers-hint-saved');
      await ctx.page.locator('.info-icon').first().click();
      await snap(ctx, 'answers-trap');
      await ctx.page.click('#trapModal button');
      await ctx.page.locator(`.trap-checkbox[data-index="0"]`).click();
      await snap(ctx, 'answers-msg');
      await ctx.page.click('#messageModal button');
    });

    // ---- build ----
    let zipFile = null, qrPdf = null;
    const workDir = fs.mkdtempSync(path.join(os.tmpdir(), 'mv-refs-'));
    await step('build', async () => {
      const txt = Buffer.from('Accounts\n========\nCredit union checking (ends 2231)\nBrokerage IRA (ends 9180)\nHardware wallet: in the safe, top shelf\n');
      const png = H.makePng(640, 400, 3);
      const pdf = H.makePdf ? H.makePdf(3) : null;
      const uploads = [
        ...(pdf ? [{ id: 's1', name: 'letter-of-instruction.pdf', type: 'application/pdf', bytes: pdf }] : []),
        { id: 's2', name: 'cabin-sunset.png', type: 'image/png', bytes: png },
        { id: 's3', name: 'accounts.txt', type: 'text/plain', bytes: txt },
      ];
      const data = {
        userData: {
          id: 'userData', answers: ANSWERS, normalizedAnswers: ANSWERS.map(a => a.toLowerCase()),
          spaceIndexes: ANSWERS.map(a => [...a].flatMap((ch, i) => (ch === ' ' ? [i] : []))), seeds: [],
          hints: { 4: { letters: [1], custom: 'a very common town name', showLength: true }, 6: { mcqEnabled: true, mcqOptions: ['Subaru', 'Toyota', 'Ford', 'Honda', 'Jeep', 'Volvo'], letters: [], custom: '', showLength: false } },
          questions: QUESTIONS, questionCount: QUESTIONS.length, unlockIndexes: GATES, finalMessage: FINAL, trapIndex: TRAP,
        },
        uploads,
        namedSeedFiles: [{ name: 'btc_cold_storage.txt', content: SEED }],
      };
      fs.writeFileSync(path.join(OUT, '_fixture-inputs.json'), JSON.stringify({ ...data, uploads: uploads.map(u => ({ ...u, bytes: Buffer.from(u.bytes).toString('base64') })) }, null, 1));
      await H.seedBuildInputs(ctx, data);
      const before = await downloadCount(ctx);
      await goto(ctx, 'build-vault.html');
      await ctx.page.waitForTimeout(300);
      await snap(ctx, 'build-progress');
      zipFile = await waitForDownload(ctx, 'memoro-vault.zip', { after: before });
      fs.copyFileSync(zipFile, path.join(OUT, '_fixture-vault.zip'));
      await ctx.page.locator('#litePickerModal').waitFor({ state: 'visible' });
      await snap(ctx, 'build-picker');
      await ctx.page.locator('#liteSeedsList input[type=checkbox]').first().check();
      await snap(ctx, 'build-picker-checked');
      await ctx.page.click('#litePickerConfirm');
      qrPdf = await waitForDownload(ctx, 'memoro-lite-QR.pdf', { after: before });
      fs.copyFileSync(qrPdf, path.join(OUT, '_fixture-lite-QR.pdf'));
      await ctx.page.locator('#postBuildModal').waitFor({ state: 'visible' });
      await snap(ctx, 'build-post');
      await scrollEl(ctx, '#postBuildModal', 100000);
      await snap(ctx, 'build-post-bottom');
    });

    // ---- dashboard with a vault, unlock ----
    await step('dashboard-vault', async () => {
      const named = path.join(workDir, 'Family Vault.zip');
      fs.copyFileSync(zipFile, named);
      await goto(ctx, 'dashboard.html');
      await ctx.page.setInputFiles('#vaultZipInput', named);
      await ctx.page.locator('#messageModal').waitFor({ state: 'visible' });
      await snap(ctx, 'dashboard-imported');
      await ctx.page.click('#messageModal button');
      await snap(ctx, 'dashboard-vault');
      await ctx.page.click('.three-dots');
      await snap(ctx, 'dashboard-menu');
      await ctx.page.click('.vault-menu button[data-action="renameVault"]');
      await snap(ctx, 'dashboard-rename');
      await ctx.page.click('button[data-action="cancelRename"]');
      await ctx.page.click('.three-dots');
      await ctx.page.click('.vault-menu button[data-action="deleteVault"]');
      await snap(ctx, 'dashboard-confirm');
      await ctx.page.click('button[data-action="confirmNo"]');
      await ctx.page.locator('.unlock-button').last().click();
      await snap(ctx, 'dashboard-unlock');
      await ctx.page.fill('#answer1Input', 'wrong');
      await ctx.page.fill('#answer2Input', 'answers');
      await ctx.page.click('#unlockButton');
      await ctx.page.locator('#messageModal').waitFor({ state: 'visible', timeout: 60000 });
      await snap(ctx, 'dashboard-unlock-failed');
      await ctx.page.click('#messageModal button');
      await ctx.page.waitForTimeout(1500);
      await snap(ctx, 'dashboard-lockout');
      await ctx.page.waitForTimeout(15000);
      await ctx.page.fill('#answer1Input', ANSWERS[GATES[0]].toLowerCase());
      await ctx.page.fill('#answer2Input', ANSWERS[GATES[1]].toLowerCase());
      await snap(ctx, 'dashboard-unlock-filled');
      await Promise.all([ctx.page.waitForURL(/recover\.html/, { timeout: 120000 }), ctx.page.click('#unlockButton')]);
      await ctx.page.waitForLoadState('domcontentloaded');
    });

    // ---- recover ----
    await step('recover', async () => {
      await ctx.page.locator('#redHerringModal').waitFor({ state: 'visible' });
      await snap(ctx, 'recover-rh');
      await ctx.page.locator('#redHerringModal button').click();
      await ctx.page.locator('input.answer-input').first().waitFor();
      await snap(ctx, 'recover');
      const h = await ctx.page.evaluate(() => document.scrollingElement.scrollHeight);
      for (let y = 600, i = 1; y < h; y += 600, i++) { await scrollTo(ctx, y); await snap(ctx, `recover-scroll${i}`); }
      await scrollTo(ctx, 0);
      for (let i = 0; i < ANSWERS.length; i++) {
        await ctx.page.fill(`input.answer-input[data-index="${i}"]`, i === TRAP ? '' : ANSWERS[i].toLowerCase());
      }
      await snap(ctx, 'recover-filled');
      await ctx.page.click('button:has-text("Recover Vault")');
      await ctx.page.waitForTimeout(2500);
      await snap(ctx, 'recover-pow');
      await ctx.page.locator('#seedModal.show').waitFor({ timeout: 300000 });
      await snap(ctx, 'recover-done');
      await scrollEl(ctx, '#seedModal', 100000);
      await snap(ctx, 'recover-done-bottom');
    });

    // ---- viewer ----
    await step('viewer', async () => {
      for (const [file, name] of [['cabin-sunset.png', 'viewer-image'], ['letter-of-instruction.pdf', 'viewer-pdf'], ['accounts.txt', 'viewer-text'], ['btc_cold_storage.txt', 'viewer-seed']]) {
        const row = ctx.page.locator('.mv-recovered-row', { hasText: file });
        if (!(await row.count())) continue;
        await row.locator('button:has-text("Open")').click();
        await ctx.page.waitForTimeout(1200);
        await snap(ctx, name);
        if (name === 'viewer-text') {
          await ctx.page.click('.mv-viewer button:has-text("Edit text")');
          await snap(ctx, 'viewer-text-edit');
          await ctx.page.click('.mv-viewer button:has-text("Cancel")');
        }
        await ctx.page.keyboard.press('Escape');
      }
    });

    // ---- editor ----
    await step('editor', async () => {
      await ctx.page.click('.mv-edit-vault');
      await ctx.page.locator('.mv-editor').waitFor();
      await snap(ctx, 'editor');
      await scrollEl(ctx, '.mv-editor', 100000);
      await snap(ctx, 'editor-bottom');
      await scrollEl(ctx, '.mv-editor', 0);
      await ctx.page.locator('.mv-row', { hasText: 'accounts.txt' }).locator('button:has-text("Rename")').click();
      await snap(ctx, 'editor-rename');
      await ctx.page.fill('.mv-dialog input.mv-input', 'accounts-and-contacts.txt');
      await H.dialogButton(ctx, 'Save');
      await ctx.page.locator('.mv-seed-row button:has-text("Show")').first().click();
      await snap(ctx, 'editor-changed');
      await ctx.page.click('button:has-text("+ Add seed phrase")');
      await snap(ctx, 'editor-add-seed');
      await H.dialogButton(ctx, 'Cancel');
      await ctx.page.locator('.mv-row', { hasText: 'cabin-sunset.png' }).locator('button:has-text("Remove")').click();
      await snap(ctx, 'editor-remove');
      await H.dialogButton(ctx, 'Cancel');
      await ctx.page.click('.mv-save');
      await snap(ctx, 'editor-save');
      await ctx.page.locator('.mv-dialog .mv-check-row input').check();
      await H.dialogButton(ctx, 'Save new vault');
      await ctx.page.waitForTimeout(400);
      await snap(ctx, 'editor-sealing');
      await ctx.page.locator('.mv-dialog-title:has-text("Paper backup contents")').waitFor({ timeout: 240000 });
      await snap(ctx, 'editor-paper');
      await H.dialogButton(ctx, 'Make paper backup');
      await ctx.page.locator('.mv-dialog-title:has-text("Vault saved")').waitFor({ timeout: 240000 });
      await snap(ctx, 'editor-saved');
      await H.dialogButton(ctx, 'OK');
      await ctx.page.click('.mv-editor button:has-text("Close editor")');
      await ctx.page.click('button:has-text("Donate with preferred crypto")');
      await ctx.page.waitForTimeout(300);
      await snap(ctx, 'recover-toast');
    });

    // ---- restore physical backup from the PDF ----
    await step('paper-restore', async () => {
      await goto(ctx, 'dashboard.html');
      await ctx.page.click('button[data-action="openPaperDecrypt"]');
      await ctx.page.setInputFiles('#paperFiles', qrPdf);
      await ctx.page.waitForTimeout(6000);
      await snap(ctx, 'dashboard-paper-restored');
    });
  } finally {
    await ctx.app.close();
  }
  log('done');
})().catch((e) => { console.error(e); process.exit(1); });
