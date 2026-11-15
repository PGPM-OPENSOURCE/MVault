// reference/capture-editor.cjs — the recovered-vault viewer and the vault editor (second pass).
//   env -u ELECTRON_RUN_AS_NODE node reference/capture-editor.cjs <outDir>
'use strict';
const fs = require('fs'), os = require('os'), path = require('path');
const L = require('./lib.cjs');
const { H, REPO, launchApp, goto, ANSWERS, GATES, TRAP, log, snap, step, scrollEl } = L;
const OUT = path.resolve(process.argv[2] || 'reference/pgpm');
L.setOut(OUT);

(async () => {
  const ctx = await launchApp(REPO, { theme: 'pgpm' });
  try {
    const workDir = fs.mkdtempSync(path.join(os.tmpdir(), 'mv-refs-'));
    const named = path.join(workDir, 'Family Vault.zip');
    fs.copyFileSync(path.join(OUT, '_fixture-vault.zip'), named);
    await H.importVault(ctx, named);
    await H.unlock(ctx, ANSWERS[GATES[0]].toLowerCase(), ANSWERS[GATES[1]].toLowerCase());
    const answers = Object.fromEntries(ANSWERS.map((a, i) => [i, i === TRAP ? '' : a.toLowerCase()]));
    await H.recover(ctx, answers);
    log('recovered');

    await step('viewer-seed', async () => {
      await ctx.page.locator('.mv-recovered-row', { hasText: 'btc_cold_storage.txt' }).locator('button:has-text("Open")').click();
      await ctx.page.waitForTimeout(800);
      await snap(ctx, 'viewer-seed');
      await ctx.page.keyboard.press('Escape');
    });

    await step('editor', async () => {
      await ctx.page.click('.mv-edit-vault');
      await ctx.page.locator('.mv-editor').waitFor();
      await snap(ctx, 'editor');
      await scrollEl(ctx, '.mv-editor', 100000);
      await snap(ctx, 'editor-bottom');
      await scrollEl(ctx, '.mv-editor', 0);
      await ctx.page.locator('.mv-row', { hasText: 'accounts.txt' }).locator('button:has-text("Open")').click();
      await ctx.page.waitForTimeout(600);
      await snap(ctx, 'editor-viewer-text');
      await ctx.page.click('.mv-viewer button:has-text("Edit text")');
      await snap(ctx, 'editor-viewer-text-edit');
      await ctx.page.click('.mv-viewer button:has-text("Cancel")');
      await ctx.page.keyboard.press('Escape');
      await ctx.page.locator('.mv-row', { hasText: 'accounts.txt' }).locator('button:has-text("Rename")').click();
      await snap(ctx, 'editor-rename');
      await ctx.page.fill('.mv-dialog input.mv-input', 'accounts-and-contacts.txt');
      await H.dialogButton(ctx, 'Save');
      await ctx.page.locator('.mv-seed-row button:has-text("Show")').first().click();
      await ctx.page.fill('.mv-final-message', 'A changed final message.');
      await ctx.page.evaluate(() => document.activeElement?.blur());
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
      await snap(ctx, 'editor-save-checked');
      await H.dialogButton(ctx, 'Save new vault');
      await ctx.page.waitForTimeout(300);
      await snap(ctx, 'editor-sealing');
      await ctx.page.locator('.mv-dialog-title:has-text("Paper backup contents")').waitFor({ timeout: 240000 });
      await snap(ctx, 'editor-paper');
      await H.dialogButton(ctx, 'Make paper backup');
      await ctx.page.locator('.mv-dialog-title:has-text("Vault saved")').waitFor({ timeout: 240000 });
      await snap(ctx, 'editor-saved');
      await H.dialogButton(ctx, 'OK');
      await ctx.page.click('.mv-editor button:has-text("Close editor")');
      await ctx.page.waitForTimeout(300);
      await ctx.page.click('button:has-text("Donate with preferred crypto")');
      await ctx.page.waitForTimeout(300);
      await snap(ctx, 'recover-toast');
    });
  } finally {
    await ctx.app.close();
  }
  log('done');
})().catch((e) => { console.error(e); process.exit(1); });
