// core/data/extract-zxcvbn.cjs — pulls the dictionaries and keyboard graphs out of the zxcvbn.js
// the Electron app ships (zxcvbn 4.4.2), so the Rust port scores against exactly the same data.
//   node core/data/extract-zxcvbn.cjs <MVault>/src/libs/zxcvbn.js core/data/zxcvbn-data.json
'use strict';
const fs = require('fs');
const [src, out] = process.argv.slice(2);
const s = fs.readFileSync(src, 'utf8');

// the object literal that starts at `start` ("{"), with strings skipped while matching braces
function literalAt(start) {
  let depth = 0, q = null;
  for (let i = start; i < s.length; i++) {
    const c = s[i];
    if (q) { if (c === '\\') i++; else if (c === q) q = null; continue; }
    if (c === '"' || c === "'") { q = c; continue; }
    if (c === '{') depth++;
    else if (c === '}') { depth--; if (depth === 0) return s.slice(start, i + 1); }
  }
  throw new Error('unbalanced literal');
}
const evalLiteral = (lit) => new Function(`"use strict"; return (${lit});`)();

const graphsStart = s.lastIndexOf('{', s.indexOf('qwerty:{'));
const graphs = evalLiteral(literalAt(graphsStart));
const listsStart = s.lastIndexOf('{', s.indexOf('passwords:"'));
const lists = evalLiteral(literalAt(listsStart));

const data = {
  source: 'zxcvbn.js 4.4.2 as shipped in MVault src/libs (dictionaries + adjacency graphs)',
  frequency_lists: Object.entries(lists).map(([name, words]) => [name, words]),
  adjacency_graphs: Object.entries(graphs).map(([name, g]) => [name, Object.entries(g)]),
};
fs.writeFileSync(out, JSON.stringify(data));
console.log('lists:', data.frequency_lists.map(([n, w]) => `${n}=${w.length}`).join(' '));
console.log('graphs:', data.adjacency_graphs.map(([n, g]) => `${n}=${g.length}`).join(' '));
