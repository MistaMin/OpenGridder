#!/usr/bin/env python3
"""Build the audited preview manual; requires ReportLab."""
from pathlib import Path
from html import escape
from reportlab.lib import colors
from reportlab.lib.styles import getSampleStyleSheet
from reportlab.lib.enums import TA_LEFT
from reportlab.lib.pagesizes import A4
from reportlab.platypus import SimpleDocTemplate, Paragraph, Spacer

root = Path(__file__).resolve().parents[1]
destination = root / 'output/pdf/OpenGridder-User-Manual.pdf'
destination.parent.mkdir(parents=True, exist_ok=True)
styles = getSampleStyleSheet()
styles['Normal'].fontName = 'Helvetica'
styles['Normal'].fontSize = 10
styles['Normal'].leading = 14
styles['Normal'].spaceAfter = 9
styles['Normal'].alignment = TA_LEFT
styles['Heading1'].textColor = colors.HexColor('#164c60')
styles['Heading2'].textColor = colors.HexColor('#164c60')
story = []
for block in (root / 'Docs/UserManual.md').read_text().strip().split('\n\n'):
    if block.startswith('# '):
        story += [Paragraph(escape(block[2:]), styles['Heading1']), Spacer(1, 10)]
    elif block.startswith('## '):
        story.append(Paragraph(escape(block[3:]), styles['Heading2']))
    else:
        story.append(Paragraph(escape(' '.join(block.splitlines())), styles['Normal']))

def footer(canvas, doc):
    canvas.setFont('Helvetica', 8)
    canvas.setFillColor(colors.HexColor('#53616b'))
    canvas.drawString(42, 28, 'OpenGridder | AAX preview | 24 October 2026')
    canvas.drawRightString(A4[0] - 42, 28, str(doc.page))

SimpleDocTemplate(str(destination), pagesize=A4, rightMargin=42, leftMargin=42,
                  topMargin=40, bottomMargin=44, title='OpenGridder AAX Preview Manual',
                  author='OpenGrid Studio').build(story, onFirstPage=footer, onLaterPages=footer)
print(destination)
