#!/usr/bin/env python3
"""PDF後処理: 中央フッターにページ番号を焼き込み、タイトルメタデータを設定する。

headless Chromium の print-to-pdf は @page のフッターカウンターに未対応のため、
生成後の PDF に reportlab でページ番号オーバーレイを合成する。

使い方:
  python docs/add_page_numbers.py <input.pdf> <output.pdf> [--title "PDFタイトル"]

依存: pypdf, reportlab
"""
import argparse, io

from pypdf import PdfReader, PdfWriter
from reportlab.lib.colors import Color
from reportlab.pdfgen import canvas


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("input")
    ap.add_argument("output")
    ap.add_argument("--title", default=None, help="PDFメタデータのタイトル")
    args = ap.parse_args()

    reader = PdfReader(args.input)
    writer = PdfWriter()
    n = len(reader.pages)

    for i, page in enumerate(reader.pages, 1):
        w = float(page.mediabox.width)
        h = float(page.mediabox.height)
        buf = io.BytesIO()
        c = canvas.Canvas(buf, pagesize=(w, h))
        c.setFont("Helvetica", 8)
        c.setFillColor(Color(0.45, 0.45, 0.45))
        c.drawCentredString(w / 2, 17, f"{i} / {n}")  # 下端から約6mm
        c.save()
        buf.seek(0)
        overlay = PdfReader(buf).pages[0]
        page.merge_page(overlay)
        writer.add_page(page)

    meta = {k: v for k, v in (reader.metadata or {}).items()}
    if args.title:
        meta["/Title"] = args.title
    if meta:
        writer.add_metadata(meta)

    with open(args.output, "wb") as f:
        writer.write(f)
    print(f"wrote {args.output} ({n} pages, page numbers added)")


if __name__ == "__main__":
    main()
