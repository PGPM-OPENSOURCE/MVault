// reference/check-js-restores-rust-pdf.cjs — the Electron app restores a paper backup the Rust app
// printed (the other direction is app/src/qrscan.rs `restores_the_reference_pdf`).
//   env -u ELECTRON_RUN_AS_NODE node reference/check-js-restores-rust-pdf.cjs <rust-printed.pdf>
'use strict';
const { REPO, launchApp, goto } = require('./lib.cjs');

(async () => {
  const pdf = process.argv[2];
  const ctx = await launchApp(REPO, { theme: 'pgpm' });
  try {
    await goto(ctx, 'dashboard.html');
    await ctx.page.click('button[data-action="openPaperDecrypt"]');
    await ctx.page.setInputFiles('#paperFiles', pdf);
    await ctx.page.waitForFunction(() => /Rebuild complete|Rebuild failed/.test(document.getElementById('paperStatus')?.textContent || ''), null, { timeout: 180000 });
    await ctx.page.waitForTimeout(1500);
    const status = await ctx.page.evaluate(() => document.getElementById('paperStatus').innerText);
    const lines = status.split('\n');
    console.log(lines.slice(0, 3).join('\n'));
    console.log('...');
    console.log(lines.slice(-3).join('\n'));
    const vaults = await ctx.page.evaluate(() => Array.from(document.querySelectorAll('.vault-item')).map(e => e.innerText.split('\n')[0]));
    console.log('vault list:', JSON.stringify(vaults));
    if (!/Rebuild complete/.test(status)) process.exit(2);
  } finally {
    await ctx.app.close();
  }
})().catch((e) => { console.error(e); process.exit(1); });
