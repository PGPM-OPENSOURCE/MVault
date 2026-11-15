#!/usr/bin/env python3
"""compare.py <ref_dir> <rust_dir> <scenario> [--out dir]

Compares a Rust snapshot with the Electron reference: every text run (by its trimmed text) with
its position and colour, and the two screenshots pixel by pixel. Writes <scenario>-cmp.png
(reference | rust | difference x4) and prints a short report."""
import json, re, sys, os
import numpy as np
import io
from PIL import Image, ImageCms

def rgb(c):
    m = re.findall(r'[\d.]+', c or '')
    return tuple(int(float(x)) for x in m[:3]) if len(m) >= 3 else None

def norm(t):
    return re.sub(r'\s+', ' ', t).strip()

def main():
    ref_dir, rust_dir, sc = sys.argv[1:4]
    out = sys.argv[sys.argv.index('--out') + 1] if '--out' in sys.argv else rust_dir
    R = json.load(open(os.path.join(ref_dir, sc + '.json')))
    S = json.load(open(os.path.join(rust_dir, sc + '.json')))
    upper = lambda t: t['t'].upper() if t.get('tt') == 'uppercase' else t['t']
    ref = [(norm(upper(t)), t) for t in R['texts'] if norm(t['t'])]
    rust = [(norm(t['t']), t) for t in S['texts'] if norm(t['t'])]
    used = set()
    bad, missing = [], []
    for k, t in ref:
        cands = [(i, u) for i, (kk, u) in enumerate(rust) if kk == k and i not in used]
        if not cands:
            cands = [(i, u) for i, (kk, u) in enumerate(rust) if i not in used and (kk.startswith(k[:20]) or k.startswith(kk[:20])) and len(kk) > 3]
        if not cands:
            missing.append((k, t))
            continue
        i, u = min(cands, key=lambda c: abs(c[1]['x'] - t['x']) + abs(c[1]['y'] - t['y']))
        used.add(i)
        dx, dy = u['x'] - t['x'], u['y'] - t['y']
        # Range rects of non-16px text start at the content box; the recorder gives the line box
        if abs(dx) > 0.6 or abs(dy) > 0.6 or rgb(u['c']) != rgb(t['c']):
            bad.append((k, t, u, dx, dy))
    extra = [k for i, (k, _) in enumerate(rust) if i not in used]
    overlay = any(b['w'] >= R['vw'] - 1 and b['h'] >= R['vh'] - 1 and rgb(b['bg']) == (12, 12, 12) for b in R['boxes'])
    print(f"== {sc}: {len(ref)} ref texts, {len(rust)} rust texts, {len(bad)} off, {len(missing)} missing, {len(extra)} extra")
    for k, t, u, dx, dy in bad[:40]:
        col = '' if rgb(u['c']) == rgb(t['c']) else f" colour ref{rgb(t['c'])} rust{rgb(u['c'])}"
        print(f"  off  dx={dx:+7.2f} dy={dy:+7.2f} ref=({t['x']:.2f},{t['y']:.2f}){col} |{k[:70]}")
    for k, t in missing[:40]:
        print(f"  {'hidden?' if overlay else 'MISSING'} ({t['x']:.1f},{t['y']:.1f}) |{k[:80]}")
    for k in extra[:20]:
        print(f"  extra |{k[:80]}")
    a = Image.open(os.path.join(ref_dir, sc + '.png'))
    if a.info.get('icc_profile'):
        # Chromium's screenshot is in the display's colour space: bring it back to sRGB
        src = ImageCms.ImageCmsProfile(io.BytesIO(a.info['icc_profile']))
        a = ImageCms.profileToProfile(a.convert('RGB'), src, ImageCms.createProfile('sRGB'), outputMode='RGB')
    a = a.convert('RGB')
    b = Image.open(os.path.join(rust_dir, sc + '.png')).convert('RGB')
    if b.size != a.size:
        print(f"  size ref {a.size} rust {b.size}")
        b = b.resize(a.size)
    A = np.asarray(a).astype(np.int16)
    B = np.asarray(b).astype(np.int16)
    D = np.abs(A - B).max(axis=2)
    print(f"  pixels differing >24: {100.0 * (D > 24).mean():.3f}%   >64: {100.0 * (D > 64).mean():.3f}%")
    diff = Image.fromarray(np.clip(D * 4, 0, 255).astype(np.uint8)).convert('RGB')
    W, H = a.size
    cmp = Image.new('RGB', (W * 3 + 20, H), (60, 0, 60))
    cmp.paste(a, (0, 0)); cmp.paste(b, (W + 10, 0)); cmp.paste(diff, (2 * W + 20, 0))
    cmp = cmp.resize((cmp.width // 2, cmp.height // 2))
    cmp.save(os.path.join(out, sc + '-cmp.png'))

if __name__ == '__main__':
    main()
