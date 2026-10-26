"""Derive the .picasa.ini `filters=` parameter layout for each filter from Picasa's filterdesc.xml.

Field order (from the serializer at 0x8fdab0 and the filterdesc parser at 0x9023c0):
  <id>=<0|1>
  [cursor x, y]        ,%f,%f   when <cursor persist="1">
  [slider 0..2]        ,%f      each slider unless persist="0"
  [colorcircle color]  ,%08x    when any <colorcircle>
  [colorwheel colors]  ,%08x    one per <colorwheel>
  [slider 3]           ,%f
  [rotation]           ,%d      when filter has rotate="1" (value & 3)
crop64 and rot are special-cased: crop64=1,<%I64x rect64>, rot=1,<%d>.
"""
import re
import sys
import xml.etree.ElementTree as ET

text = open(sys.argv[1], encoding='utf-8', errors='replace').read()
# The file embeds MXML with namespaced tags/attributes (cnt:, ctrl:, mx:, eff:) that lack declarations.
text = re.sub(r'(</?|\s)([A-Za-z]+):(?=[A-Za-z_])', r'\1\2_', text)
root = ET.fromstring(text)

def label(el):
    l = el.find('label')
    return (l.text or '').strip() if l is not None else ''

for f in root.findall('filter'):
    fid = f.get('id')
    fields = []
    if fid == 'crop64':
        fields = ['rect64 (hex, 4×16 bit: left,top,right,bottom ×65535)']
    elif fid == 'rot':
        fields = ['rotation %d']
    else:
        cur = f.find('cursor')
        if cur is not None and cur.get('persist') == '1':
            fields += [f'cursor.x', f'cursor.y'] if cur.get('coords') != 'pixel' else ['cursor.x(px)', 'cursor.y(px)']
        sliders = {int(s.get('id')): s for s in f.findall('sliders/slider')}
        for i in range(3):
            s = sliders.get(i)
            if s is not None and s.get('persist') != '0':
                fields.append(f'{label(s) or "slider"+str(i)} (range {s.findtext("range","?")}, offset {s.findtext("offset","0")}, default {s.findtext("default","0")})')
        if f.findall('.//colorcircle'):
            fields.append('color %08x (colorcircle)')
        for _ in f.findall('.//colorwheel'):
            fields.append('color %08x (colorwheel)')
        s = sliders.get(3)
        if s is not None and s.get('persist') != '0':
            fields.append(f'{label(s)} (range {s.findtext("range","?")}, offset {s.findtext("offset","0")}, default {s.findtext("default","0")})')
        if f.get('rotate') == '1':
            fields.append('rotation %d (0..3)')
        if f.find('effect') is not None:
            fields.append('(Picnik MXML effect; parameters not in filters= layout)')
    attrs = ' '.join(f'{k}={v}' for k, v in f.attrib.items() if k != 'id')
    print(f'{fid}\t{label(f)}\t{attrs}\t' + ' | '.join(fields))
