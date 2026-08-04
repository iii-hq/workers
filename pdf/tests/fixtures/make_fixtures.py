"""Generate the committed test PDFs from raw PDF syntax.

Written by hand rather than exported from an application: the documents stay
under two kilobytes, their content is known exactly, and nothing here is
third-party. Run from the worker root:

    python3 tests/fixtures/make_fixtures.py
"""

from pathlib import Path

HERE = Path(__file__).parent


def build(objects: list[bytes]) -> bytes:
    """Assemble numbered objects into a PDF with a correct xref table."""
    out = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for number, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += f"{number} 0 obj\n".encode() + body + b"\nendobj\n"

    xref_at = len(out)
    out += f"xref\n0 {len(objects) + 1}\n".encode()
    out += b"0000000000 65535 f \n"
    for offset in offsets[1:]:
        out += f"{offset:010d} 00000 n \n".encode()
    out += (
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n"
        f"startxref\n{xref_at}\n%%EOF\n"
    ).encode()
    return bytes(out)


def stream(content: bytes) -> bytes:
    return b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream"


def text_two_page() -> bytes:
    """Two pages of real text, at known positions and sizes."""
    page_one = (
        b"BT /F1 24 Tf 72 700 Td (Quarterly Report) Tj ET\n"
        b"BT /F1 12 Tf 72 660 Td (Revenue rose to 4.2 million in the period.) Tj ET\n"
        b"BT /F1 12 Tf 72 640 Td (Costs held flat against the prior quarter.) Tj ET\n"
    )
    # Three text operators per page, deliberately: the default
    # min_text_ops_per_page is 3, and a page under it does not count as a text
    # page. A two-operator page here would make the fixture's own confidence
    # score an artefact of the threshold rather than of the content.
    page_two = (
        b"BT /F1 24 Tf 72 700 Td (Appendix) Tj ET\n"
        b"BT /F1 12 Tf 72 660 Td (Figures are unaudited and stated in euro.) Tj ET\n"
        b"BT /F1 12 Tf 72 640 Td (Comparatives have been restated where needed.) Tj ET\n"
    )
    return build([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>",
        stream(page_one),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>",
        stream(page_two),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ])


def no_text() -> bytes:
    """A page that draws a filled rectangle and nothing else.

    There is no text operator anywhere, which is what a scanned page looks like
    to a parser that reads content streams.
    """
    page = b"0.2 0.2 0.2 rg 72 500 468 250 re f\n"
    return build([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << >> /Contents 4 0 R >>",
        stream(page),
    ])


if __name__ == "__main__":
    for name, data in [
        ("text-two-page.pdf", text_two_page()),
        ("no-text.pdf", no_text()),
    ]:
        (HERE / name).write_bytes(data)
        print(f"{name}: {len(data)} bytes")
