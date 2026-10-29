#!/usr/bin/env python3
"""Collects the interface texts into pico.pot and brings every <lang>.po in
this folder up to date with it (`make po`).

Texts are found in crates/pico/src as tr!("…"), trn!("…", "…", n) and
tr_mark!("…"), and in crates/pico-core as the effects' and collage layouts'
names. Translations already made are kept; new texts come in untranslated,
texts no longer used are dropped. Standard library only.
"""

import re
import sys
from pathlib import Path

PO = Path(__file__).resolve().parent
ROOT = PO.parent.parent.parent
UI = ROOT / "crates/pico/src"
CORE = ROOT / "crates/pico-core/src"

STRING = r'"((?:[^"\\]|\\.)*)"'
MARKED = re.compile(r"\b(tr|tr_mark)!\(\s*" + STRING)
PLURAL = re.compile(r"\btrn!\(\s*" + STRING + r"\s*,\s*" + STRING)
# Names the core crate keeps for the interface to show.
CORE_TEXTS = {
    "effects.rs": re.compile(r'(?:label: |slider\(|color\()' + STRING),
    "collage.rs": re.compile(r"\(Layout::\w+, " + STRING + r"\)"),
    # The labels of the File block in Properties, and the flash's two words.
    "exif.rs": re.compile(r'\bpush\(\s*"([A-Z](?:[^"\\]|\\.)*)"|"(Fired|Did not fire)"'),
}


def rust_text(literal):
    """The text of a Rust string literal's inside."""
    out, chars = [], iter(literal)
    for c in chars:
        if c != "\\":
            out.append(c)
            continue
        c = next(chars, "")
        out.append({"n": "\n", "t": "\t", "\n": ""}.get(c, c))
    return "".join(out)


def po_quote(text):
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t") + '"'


def line_of(source, offset):
    return source.count("\n", 0, offset) + 1


def collect():
    """msgid → (plural or None, places), in the order first found."""
    texts = {}

    def add(text, plural, place):
        entry = texts.setdefault(text, [plural, []])
        entry[0] = entry[0] or plural
        entry[1].append(place)

    for path in sorted(UI.glob("*.rs")):
        if path.name == "tests.rs":
            continue
        source = path.read_text()
        name = path.relative_to(ROOT)
        for m in MARKED.finditer(source):
            add(rust_text(m.group(2)), None, f"{name}:{line_of(source, m.start())}")
        for m in PLURAL.finditer(source):
            add(rust_text(m.group(1)), rust_text(m.group(2)), f"{name}:{line_of(source, m.start())}")
    for file, pattern in CORE_TEXTS.items():
        source = (CORE / file).read_text()
        name = (CORE / file).relative_to(ROOT)
        for m in pattern.finditer(source):
            text = next(g for g in m.groups() if g is not None)
            add(rust_text(text), None, f"{name}:{line_of(source, m.start())}")
    return texts


def entry(text, plural, places, forms):
    lines = [f"#: {' '.join(places[:4])}"]
    lines.append(f"msgid {po_quote(text)}")
    if plural is None:
        lines.append(f"msgstr {po_quote(forms[0] if forms else '')}")
    else:
        lines.append(f"msgid_plural {po_quote(plural)}")
        for i, form in enumerate(forms or ["", ""]):
            lines.append(f"msgstr[{i}] {po_quote(form)}")
    return "\n".join(lines)


def read_po(path):
    """The header and msgid → forms of a .po file."""
    header, texts = "", {}
    blocks = path.read_text().split("\n\n")
    for block in blocks:
        msgid, forms, field = None, {}, None
        for line in block.splitlines():
            line = line.strip()
            m = re.match(r'msgid ' + STRING + '$', line)
            if m:
                msgid, field = rust_text(m.group(1)), "id"
                continue
            if line.startswith("msgid_plural"):
                field = None
                continue
            m = re.match(r'msgstr(?:\[(\d+)\])? ' + STRING + '$', line)
            if m:
                field = int(m.group(1) or 0)
                forms[field] = rust_text(m.group(2))
                continue
            m = re.match(STRING + "$", line)
            if m and field == "id":
                msgid += rust_text(m.group(1))
            elif m and field is not None:
                forms[field] += rust_text(m.group(1))
        if msgid == "":
            header = block
        elif msgid is not None:
            texts[msgid] = [forms[i] for i in sorted(forms)]
    return header, texts


TEMPLATE_HEADER = r'''# Pico interface texts. Made by `make po`; do not edit.
msgid ""
msgstr ""
"Project-Id-Version: Pico\n"
"MIME-Version: 1.0\n"
"Content-Type: text/plain; charset=UTF-8\n"
"Content-Transfer-Encoding: 8bit\n"'''


def main():
    texts = collect()
    pot = [TEMPLATE_HEADER] + [entry(t, plural, places, None) for t, (plural, places) in texts.items()]
    (PO / "pico.pot").write_text("\n\n".join(pot) + "\n")
    for path in sorted(PO.glob("*.po")):
        header, old = read_po(path)
        blocks = [header.strip()]
        missing = 0
        for text, (plural, places) in texts.items():
            forms = old.get(text)
            if not forms or not any(forms):
                missing += 1
            blocks.append(entry(text, plural, places, forms))
        path.write_text("\n\n".join(blocks) + "\n")
        print(f"{path.name}: {len(texts) - missing} of {len(texts)} translated")
    print(f"pico.pot: {len(texts)} texts")


if __name__ == "__main__":
    sys.exit(main())
