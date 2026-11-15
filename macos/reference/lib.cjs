// reference/capture.cjs — screenshots + layout dumps of every MVault (Electron) screen.
//
//   env -u ELECTRON_RUN_AS_NODE node reference/capture.cjs <outDir> [theme]
//
// Drives the real Electron app (the MVault checkout: $MVAULT_REPO, or the folder above this
// project) through every page and dialog,
// saving <name>.png (what the window shows) and <name>.json (every text line, box and control with
// its exact position and colours). The Rust app is built to match these. It also keeps the vault
// ZIP and paper backup PDF it builds, as compatibility fixtures with known answers.
'use strict';
const fs = require('fs');
const os = require('os');
const path = require('path');
const MVAULT_REPO = process.env.MVAULT_REPO || path.resolve(__dirname, '../..');
const H = require(path.join(MVAULT_REPO, 'test/e2e/helpers.cjs'));
const { REPO, launchApp, goto, waitForDownload, downloadCount } = H;

let OUT = path.resolve('reference/pgpm');
function setOut(dir) { OUT = path.resolve(dir); fs.mkdirSync(OUT, { recursive: true }); }

const QUESTIONS = [
  'What was the name of your childhood best friend?', 'What was the street name where you grew up?',
  'What is your oldest sibling’s middle name?', 'What was the name of your first pet?',
  'What city were you born in?', 'What is the name of your favorite childhood teacher?',
  'What was the make of your first car?', 'What is the middle name of one of your grandparents?',
  'What is the title of your favorite childhood book?', 'What was the mascot of your high school?',
  'What was your favorite vacation spot as a kid?', 'What was the name of your first boss?',
];
const ANSWERS = ['Tomasz Wiley', 'Juniper Lane', 'Everett', 'Biscuit', 'Springfield', 'Mrs Halvorsen',
  'Subaru', 'Augustine', 'Hatchet', 'Falcons', 'Grandma Ruth’s lake house', 'Ruth Ann'];
const GATES = [0, 3];
const TRAP = 8;
const SEED = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const FINAL = 'If you are reading this, the plan worked. Start with the letter of instruction, then call Dana at the credit union. Love you all.';

const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);

// ---------------------------------------------------------------------------------------------
// Layout dump: text lines (per rendered line), boxes with a background, controls, list markers,
// scroll containers. Coordinates are CSS px relative to the viewport.
// ---------------------------------------------------------------------------------------------
async function dump(page) {
  return page.evaluate(() => {
    const px = (v) => Math.round(v * 100) / 100;
    const shown = (el) => {
      for (let e = el; e; e = e.parentElement) {
        const cs = getComputedStyle(e);
        if (cs.display === 'none' || cs.visibility === 'hidden' || cs.opacity === '0') return false;
      }
      return true;
    };
    const sel = (el) => el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') +
      (typeof el.className === 'string' && el.className.trim() ? '.' + el.className.trim().split(/\s+/).join('.') : '');
    const texts = [];
    const tw = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    const range = document.createRange();
    for (let node; (node = tw.nextNode());) {
      const s = node.data;
      if (!s.trim()) continue;
      const el = node.parentElement;
      if (!el || ['SCRIPT', 'STYLE', 'OPTION'].includes(el.tagName) || !shown(el)) continue;
      const cs = getComputedStyle(el);
      let cur = null;
      for (let i = 0; i < s.length; i++) {
        range.setStart(node, i); range.setEnd(node, i + 1);
        const rs = range.getClientRects();
        if (!rs.length) continue;
        const rc = rs[rs.length - 1];
        if (!cur || Math.abs(rc.top - cur.y) > 2) {
          if (cur) texts.push(cur);
          cur = { t: '', x: px(rc.left), y: px(rc.top), w: 0, h: px(rc.height), c: cs.color, fs: cs.fontSize, el: sel(el) };
          if (cs.textTransform !== 'none') cur.tt = cs.textTransform;
          if (cs.letterSpacing !== 'normal' && cs.letterSpacing !== '0px') cur.ls = cs.letterSpacing;
          if (cs.whiteSpace !== 'normal') cur.ws = cs.whiteSpace;
          if (cs.fontStyle !== 'normal') cur.fst = cs.fontStyle;
          if (cs.lineHeight !== '19px') cur.lh = cs.lineHeight;
          const deco = cs.textDecorationLine; if (deco && deco !== 'none') cur.deco = deco;
        }
        cur.t += s[i];
        cur.w = px(rc.right - cur.x);
      }
      if (cur) texts.push(cur);
    }
    const boxes = [];
    const pseudo = [];
    const scrollers = [];
    for (const el of document.body.querySelectorAll('*')) {
      if (['SCRIPT', 'STYLE', 'OPTION', 'BR'].includes(el.tagName) || !shown(el)) continue;
      const cs = getComputedStyle(el);
      const rc = el.getBoundingClientRect();
      if (rc.width < 0.5 || rc.height < 0.5) continue;
      const bg = cs.backgroundColor;
      const hasBg = bg && bg !== 'rgba(0, 0, 0, 0)';
      const isBtn = el.tagName === 'BUTTON' || el.classList.contains('button') || el.classList.contains('mv-btn');
      const isCtl = isBtn || ['INPUT', 'TEXTAREA', 'SELECT', 'VIDEO', 'CANVAS', 'IMG', 'SUMMARY', 'A', 'LABEL'].includes(el.tagName);
      const b = getComputedStyle(el, '::before').content, a = getComputedStyle(el, '::after').content;
      const hasPseudo = (b && b !== 'none' && b !== 'normal') || (a && a !== 'none' && a !== 'normal');
      if (hasBg || isCtl) {
        const o = { el: sel(el), x: px(rc.left), y: px(rc.top), w: px(rc.width), h: px(rc.height), bg, c: cs.color };
        if (isCtl) {
          o.tag = el.tagName;
          if (el.type) o.type = el.type;
          if ('value' in el && !isBtn && el.tagName !== 'LI') o.value = el.value;
          if (el.placeholder) o.ph = el.placeholder;
          if (el.type === 'checkbox') o.checked = el.checked;
          if (el.disabled) o.disabled = true;
          if (isBtn) o.text = el.textContent.trim();
          if (el.tagName === 'SELECT') o.text = el.selectedOptions[0]?.textContent || '';
          if (el.tagName === 'IMG') o.natural = [el.naturalWidth, el.naturalHeight];
        }
        if (hasPseudo) { o.before = b; o.after = a; }
        boxes.push(o);
      } else if (hasPseudo) {
        pseudo.push({ el: sel(el), x: px(rc.left), y: px(rc.top), w: px(rc.width), h: px(rc.height), before: b, after: a, c: cs.color,
          bc: getComputedStyle(el, '::before').color });
      }
      if ((el.scrollHeight > el.clientHeight + 1) && /(auto|scroll)/.test(cs.overflowY)) {
        scrollers.push({ el: sel(el), x: px(rc.left), y: px(rc.top), w: px(rc.width), h: px(rc.height), st: el.scrollTop, sh: el.scrollHeight, ch: el.clientHeight });
      }
    }
    const lis = [...document.querySelectorAll('li')].filter(shown).map(li => {
      const rc = li.getBoundingClientRect(); const cs = getComputedStyle(li);
      return { x: px(rc.left), y: px(rc.top), w: px(rc.width), h: px(rc.height), lst: cs.listStyleType, pos: cs.listStylePosition, mc: getComputedStyle(li, '::marker').color };
    });
    const se = document.scrollingElement;
    return {
      url: location.pathname.split('/').pop(), title: document.title,
      vw: innerWidth, vh: innerHeight, dpr: devicePixelRatio,
      sx: scrollX, sy: scrollY, sh: se.scrollHeight, cw: document.documentElement.clientWidth,
      active: document.activeElement ? sel(document.activeElement) : null,
      texts, boxes, pseudo, lis, scrollers,
    };
  });
}

async function snap(ctx, name) {
  try {
    await ctx.page.waitForTimeout(400);
    await ctx.page.mouse.move(1, 1); // no hover state in the reference unless a step asks for it
    await ctx.page.waitForTimeout(100);
    await ctx.page.screenshot({ path: path.join(OUT, `${name}.png`) });
    fs.writeFileSync(path.join(OUT, `${name}.json`), JSON.stringify(await dump(ctx.page)));
    log('saved', name);
  } catch (e) {
    log('FAILED', name, e.message);
  }
}
async function snapHover(ctx, name, selector) {
  try {
    await ctx.page.hover(selector);
    await ctx.page.waitForTimeout(200);
    await ctx.page.screenshot({ path: path.join(OUT, `${name}.png`) });
    fs.writeFileSync(path.join(OUT, `${name}.json`), JSON.stringify(await dump(ctx.page)));
    log('saved', name);
  } catch (e) { log('FAILED', name, e.message); }
}
async function step(label, fn) {
  try { await fn(); } catch (e) { log(`STEP FAILED (${label}):`, e.message.split('\n')[0]); }
}
const scrollTo = (ctx, y) => ctx.page.evaluate((y) => window.scrollTo(0, y), y);
const scrollEl = (ctx, sel, y) => ctx.page.evaluate(([s, y]) => { const e = document.querySelector(s); if (e) e.scrollTop = y; }, [sel, y]);


module.exports = { H, REPO, launchApp, goto, waitForDownload, downloadCount, QUESTIONS, ANSWERS, GATES, TRAP, SEED, FINAL, log, dump, snap, snapHover, step, scrollTo, scrollEl, setOut };
