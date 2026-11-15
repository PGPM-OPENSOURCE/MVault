// core/tests/js_open.cjs — opens a vault with the Electron app's own vault-core.js + shipped WASM,
// exactly the way recover.js does (layer 1, then every order of the six hidden answers), and
// compares every file with what the Rust side says it put in.
//
//   node js_open.cjs <vault.zip> <expected.json>
// expected.json: { answers: [normalized...], gates: [g0, g1], uploads: [{name, b64}], seeds: [{name, content}], finalMessage }
'use strict';
const fs = require('fs');
const path = require('path');
// the Electron MVault checkout (core/tests/compat.rs passes it)
const { load } = require(path.join(process.env.MVAULT_REPO, 'test/helpers/env.cjs'));

function generatePermutations(array) {
  if (array.length <= 1) return [array.slice()];
  const result = [];
  for (let i = 0; i < array.length; i++) {
    const current = array[i];
    const remaining = array.slice(0, i).concat(array.slice(i + 1));
    for (const p of generatePermutations(remaining)) result.push([current, ...p]);
  }
  return result;
}

(async () => {
  const [zipPath, expPath] = process.argv.slice(2);
  const exp = JSON.parse(fs.readFileSync(expPath, 'utf8'));
  const { MV } = await load();
  const { entries, vaultJson, vaultEnc } = await MV.readVaultZip(new Uint8Array(fs.readFileSync(zipPath)));
  const A = exp.answers;
  const unlockConcat = A[exp.gates[0]] + A[exp.gates[1]];
  const l1 = await MV.decryptLayer1(unlockConcat, vaultJson, vaultEnc);
  if (JSON.stringify(l1.gateIndices) !== JSON.stringify(exp.gates)) throw new Error('gate indices differ');
  const perms = generatePermutations(l1.selectedL2Indices.map(i => A[i]));
  let meta = null, concat = '', attempt = 0;
  for (const p of perms) {
    attempt++;
    const c = [...l1.gateIndices.map(i => A[i]), ...l1.remainingL2Indices.map(i => A[i]), ...p].join('');
    try { meta = await MV.decryptMeta(c, vaultJson, entries.get('vault.meta')); concat = c; break; } catch (_) {}
  }
  if (!meta) throw new Error('no answer order opened vault.meta');
  const decrypted = [];
  for (const m of meta.files) decrypted.push({ meta: m, bytes: await MV.decryptFile(concat, m, entries.get(String(m.filename).toLowerCase())) });
  const c = MV.splitContents(meta, decrypted);
  const eq = (a, b, what) => { if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`${what} differ:\n  js:   ${JSON.stringify(a)}\n  rust: ${JSON.stringify(b)}`); };
  eq(c.uploads.map(u => u.name), exp.uploads.map(u => u.name), 'upload names');
  eq(c.uploads.map(u => Buffer.from(u.bytes).toString('base64')), exp.uploads.map(u => u.b64), 'upload bytes');
  eq(c.seeds, exp.seeds, 'seeds');
  eq(c.finalMessage, exp.finalMessage, 'final message');
  if (meta.vaultHash !== await MV.sha256Hex(new TextEncoder().encode(concat))) throw new Error('vault hash');
  if (exp.metaExtras) for (const [k, v] of Object.entries(exp.metaExtras)) eq(meta[k], v, `meta.${k}`);
  // the builder re-opens what it made with verifyVaultZip; do the same with the JS verifier
  await MV.verifyVaultZip(new Uint8Array(fs.readFileSync(zipPath)), { unlockConcat, permutedConcat: concat });
  // the question layer must render in recover.js: every item has a question string
  for (const q of l1.questionList) if (typeof q.question !== 'string') throw new Error('question item without text');
  console.log(JSON.stringify({ ok: true, attempt, files: meta.files.length, vaultJsonKeys: Object.keys(vaultJson), questionList: l1.questionList }));
})().catch((e) => { console.error(String(e && e.stack || e)); process.exit(1); });
