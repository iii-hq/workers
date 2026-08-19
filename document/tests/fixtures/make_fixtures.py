#!/usr/bin/env python3
"""Regenerate the committed fixture corpus in this directory.

The files are assembled here from the parts each format requires rather than
exported from an office suite, so they stay under a few kilobytes, their
content is known exactly, and they carry no third-party licensing.

Usage:

    python3 tests/fixtures/make_fixtures.py
"""

import base64
import zipfile
from pathlib import Path

OUT = Path(__file__).resolve().parent

# Fixed timestamp so a regeneration with no content change produces a
# byte-identical file and shows up as no diff at all.
ZIP_DATE = (2026, 1, 1, 0, 0, 0)

# A 1x1 red PNG. Small enough to read as a literal, real enough that a decoder
# accepts it, which is what the asset assertions need.
DOT_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8"
    "z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
)

RELS_NS = 'xmlns="http://schemas.openxmlformats.org/package/2006/relationships"'
CT_NS = 'xmlns="http://schemas.openxmlformats.org/package/2006/content-types"'
REL_TYPE = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"


def write_zip(path: Path, entries):
    """Write a package with deterministic entry order and timestamps."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, data in entries:
            info = zipfile.ZipInfo(name, date_time=ZIP_DATE)
            info.compress_type = zipfile.ZIP_DEFLATED
            zf.writestr(info, data)
    print(f"wrote {path.name} ({path.stat().st_size} bytes)")


def docx():
    """A Word document: one heading, one paragraph, one two-row table."""
    ct = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types {CT_NS}>
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"""

    rels = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships {RELS_NS}>
<Relationship Id="rId1" Type="{REL_TYPE}/officeDocument" Target="word/document.xml"/>
</Relationships>"""

    w = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
    document = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document {w}><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Quarterly Notes</w:t></w:r></w:p>
<w:p><w:r><w:t>The engine handled every request without a restart.</w:t></w:r></w:p>
<w:tbl>
<w:tr><w:tc><w:p><w:r><w:t>Metric</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Value</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>Requests</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>21480</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>
</w:body></w:document>"""

    write_zip(
        OUT / "sample.docx",
        [
            ("[Content_Types].xml", ct),
            ("_rels/.rels", rels),
            ("word/document.xml", document),
        ],
    )


def xlsx():
    """A workbook: one sheet, inline strings, no shared string table."""
    ct = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types {CT_NS}>
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"""

    rels = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships {RELS_NS}>
<Relationship Id="rId1" Type="{REL_TYPE}/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"""

    ns = 'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"'
    r_ns = 'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
    workbook = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook {ns} {r_ns}>
<sheets><sheet name="Throughput" sheetId="1" r:id="rId1"/></sheets>
</workbook>"""

    wb_rels = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships {RELS_NS}>
<Relationship Id="rId1" Type="{REL_TYPE}/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"""

    sheet = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet {ns}><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>scenario</t></is></c><c r="B1" t="inlineStr"><is><t>jobs per second</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>echo</t></is></c><c r="B2"><v>21480</v></c></row>
<row r="3"><c r="A3" t="inlineStr"><is><t>fanout</t></is></c><c r="B3"><v>2184</v></c></row>
</sheetData></worksheet>"""

    write_zip(
        OUT / "sample.xlsx",
        [
            ("[Content_Types].xml", ct),
            ("_rels/.rels", rels),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", wb_rels),
            ("xl/worksheets/sheet1.xml", sheet),
        ],
    )


def pptx():
    """A one-slide deck carrying an embedded image, so asset extraction has
    something real to pull out."""
    ct = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types {CT_NS}>
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="png" ContentType="image/png"/>
<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
<Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"""

    rels = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships {RELS_NS}>
<Relationship Id="rId1" Type="{REL_TYPE}/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"""

    p_ns = (
        'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
        'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"'
    )

    presentation = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation {p_ns}>
<p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
</p:presentation>"""

    pres_rels = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships {RELS_NS}>
<Relationship Id="rId1" Type="{REL_TYPE}/slide" Target="slides/slide1.xml"/>
</Relationships>"""

    slide = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld {p_ns}><p:cSld><p:spTree>
<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
<p:grpSpPr/>
<p:sp>
<p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
<p:spPr/>
<p:txBody><a:bodyPr/><a:p><a:r><a:t>Three Primitives</a:t></a:r></a:p></p:txBody>
</p:sp>
<p:sp>
<p:nvSpPr><p:cNvPr id="3" name="Body"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
<p:spPr/>
<p:txBody><a:bodyPr/><a:p><a:r><a:t>Worker, function, trigger.</a:t></a:r></a:p></p:txBody>
</p:sp>
<p:pic>
<p:nvPicPr><p:cNvPr id="4" name="Diagram" descr="the engine diagram"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
<p:blipFill><a:blip r:embed="rId1"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>
<p:spPr/>
</p:pic>
</p:spTree></p:cSld></p:sld>"""

    slide_rels = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships {RELS_NS}>
<Relationship Id="rId1" Type="{REL_TYPE}/image" Target="../media/image1.png"/>
</Relationships>"""

    write_zip(
        OUT / "sample.pptx",
        [
            ("[Content_Types].xml", ct),
            ("_rels/.rels", rels),
            ("ppt/presentation.xml", presentation),
            ("ppt/_rels/presentation.xml.rels", pres_rels),
            ("ppt/slides/slide1.xml", slide),
            ("ppt/slides/_rels/slide1.xml.rels", slide_rels),
            ("ppt/media/image1.png", DOT_PNG),
        ],
    )


def rtf():
    """Rich Text is plain text on the wire, which makes it the cheapest check
    that a signature-carrying non-package format is recognised."""
    path = OUT / "sample.rtf"
    path.write_text(
        r"{\rtf1\ansi\deff0 {\b Release notes}\par The queue drained in 40 ms.\par}",
        encoding="ascii",
    )
    print(f"wrote {path.name} ({path.stat().st_size} bytes)")


def csv():
    path = OUT / "sample.csv"
    path.write_text("scenario,jobs_per_second\necho,21480\nfanout,2184\n", encoding="utf-8")
    print(f"wrote {path.name} ({path.stat().st_size} bytes)")


if __name__ == "__main__":
    docx()
    xlsx()
    pptx()
    rtf()
    csv()
