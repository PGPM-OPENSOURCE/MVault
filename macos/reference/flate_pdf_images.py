"""reference/pgpm/_fixture-lite-QR.pdf is the paper backup the Electron app printed (jsPDF stores its
QR images uncompressed: 82 MB). This made the 400 KB copy kept here:

    python3 reference/flate_pdf_images.py printed.pdf reference/pgpm/_fixture-lite-QR.pdf

Losslessly Flate-compresses the uncompressed image streams of a simple PDF (no object streams,
direct /Length values), rewriting the cross-reference table. Pixel data is unchanged."""
import re, sys, zlib

src, dst = sys.argv[1], sys.argv[2]
b = open(src, 'rb').read()
header_end = b.index(b'\n', b.index(b'\n') + 1) + 1  # "%PDF-1.x" + binary comment line


def dict_end(buf, i):
    """Index just past the '>>' matching the '<<' at buf[i]."""
    depth = 0
    while i < len(buf):
        if buf.startswith(b'<<', i):
            depth += 1
            i += 2
        elif buf.startswith(b'>>', i):
            depth -= 1
            i += 2
            if depth == 0:
                return i
        else:
            i += 1
    raise ValueError('unbalanced dictionary')


objs = []  # (num, gen, dict bytes or None, body)
pos = header_end
obj_re = re.compile(rb'(\d+) (\d+) obj\s*')
xref_at = b.rindex(b'xref', 0, b.rindex(b'startxref'))
while True:
    m = obj_re.search(b, pos)
    if not m or m.start() >= xref_at:
        break
    start = m.end()
    if b.startswith(b'<<', start):
        de = dict_end(b, start)
        d = b[start + 2:de - 2]
        sm = re.match(rb'\s*stream\r?\n', b[de:de + 16])
        if sm:
            length = int(re.search(rb'/Length (\d+)\s', d + b' ').group(1))
            data_start = de + sm.end()
            data = b[data_start:data_start + length]
            end = b.index(b'endobj', data_start + length)
            objs.append((int(m.group(1)), int(m.group(2)), d, data))
            pos = end + len(b'endobj')
            continue
    end = b.index(b'endobj', start)
    objs.append((int(m.group(1)), int(m.group(2)), None, b[start:end]))
    pos = end + len(b'endobj')

trailer = b[b.rindex(b'trailer'):b.rindex(b'startxref')]
out = bytearray(b[:header_end])
offsets = {}
saved = 0
for num, gen, d, data in objs:
    offsets[num] = len(out)
    if d is None:
        out += b'%d %d obj\n' % (num, gen) + data.strip() + b'\nendobj\n'
        continue
    if b'/Subtype /Image' in d and b'/Filter' not in d:
        z = zlib.compress(data, 9)
        saved += len(data) - len(z)
        data = z
        # /DecodeParms was inert without a filter; with Flate it would be read as predictor settings
        d = re.sub(rb'/DecodeParms\s*<<[^>]*>>\s*', b'', d)
        d = d.rstrip() + b'\n/Filter /FlateDecode\n'
    d = re.sub(rb'/Length \d+', b'/Length %d' % len(data), d)
    out += b'%d %d obj\n<<' % (num, gen) + d + b'>>\nstream\n' + data + b'\nendstream\nendobj\n'
xref = len(out)
size = max(offsets) + 1
out += b'xref\n0 %d\n0000000000 65535 f \n' % size
for i in range(1, size):
    out += (b'%010d 00000 n \n' % offsets[i]) if i in offsets else b'0000000000 65535 f \n'
out += trailer.strip() + b'\nstartxref\n%d\n%%%%EOF\n' % xref
open(dst, 'wb').write(out)
print(f'{len(objs)} objects, {len(b)} -> {len(out)} bytes ({saved} saved)')
