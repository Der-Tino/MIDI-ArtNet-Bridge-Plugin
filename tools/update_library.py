#!/usr/bin/env python3
"""Packt die QLC+-Fixture-Bibliothek in library/qlcplus-fixtures.pack.

Aufruf:
    git clone --depth 1 https://github.com/mcallegari/qlcplus /tmp/qlcplus
    python3 tools/update_library.py /tmp/qlcplus

Format (zlib-komprimiert): pro Fixture eine Kopfzeile
    Hersteller<TAB>Modell<TAB>Typ<TAB>Dateiname<TAB>Bytes<LF>
gefolgt von genau <Bytes> Bytes QXF-XML (UTF-8).
"""
import pathlib, subprocess, sys, zlib
import xml.etree.ElementTree as ET

def text(root, tag):
    for el in root.iter():
        if el.tag.split('}')[-1] == tag:
            return (el.text or '').strip()
    return ''

def clean(s):
    return s.replace('\t', ' ').replace('\n', ' ').strip()

def main():
    src = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else '/tmp/qlcplus')
    fixdir = src / 'resources' / 'fixtures'
    out = pathlib.Path(__file__).resolve().parent.parent / 'library'
    files = sorted(fixdir.rglob('*.qxf'), key=lambda p: str(p).lower())
    buf, n, skipped = bytearray(), 0, 0
    for f in files:
        data = f.read_bytes()
        try:
            root = ET.fromstring(data)
        except ET.ParseError:
            skipped += 1
            continue
        manu, model, typ = text(root, 'Manufacturer'), text(root, 'Model'), text(root, 'Type')
        if not manu or not model:
            skipped += 1
            continue
        head = '\t'.join(clean(x) for x in (manu, model, typ, f.name, str(len(data)))) + '\n'
        buf += head.encode('utf-8') + data
        n += 1
    packed = zlib.compress(bytes(buf), 9)
    (out / 'qlcplus-fixtures.pack').write_bytes(packed)
    try:
        rev = subprocess.check_output(['git', '-C', str(src), 'log', '-1', '--format=%h %ad', '--date=short'], text=True).strip()
    except Exception:
        rev = 'unbekannt'
    (out / 'VERSION.txt').write_text(f'QLC+ fixtures {rev}\n{n} Fixtures\n', encoding='utf-8')
    print(f'{n} Fixtures gepackt ({skipped} übersprungen), {len(buf)/1e6:.1f} MB → {len(packed)/1e6:.2f} MB, Stand {rev}')

if __name__ == '__main__':
    main()
