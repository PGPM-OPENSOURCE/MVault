// core/tests/js_zxcvbn.cjs — log2(guesses) from the zxcvbn.js the Electron app ships, one per line.
//   node js_zxcvbn.cjs < answers.txt
'use strict';
const zxcvbn = require(require('path').join(process.env.MVAULT_REPO, 'src/libs/zxcvbn.js'));
const lines = require('fs').readFileSync(0, 'utf8').split('\n');
const out = lines.filter((l, i) => i < lines.length - 1 || l !== '').map(a => { a = a.replace(/\\t/g, '\t'); try { return Math.log2(zxcvbn(a).guesses || 1); } catch (e) { return null; } });
console.log(JSON.stringify(out));
