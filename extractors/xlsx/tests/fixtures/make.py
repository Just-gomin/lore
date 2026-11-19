"""고정 파일 meeting.xlsx와 meeting.ods를 만든다.

Excel과 LibreOffice가 저장하는 것처럼 수식의 계산값을 파일에 넣는다. 추출기는 수식을 계산하지 않고
저장된 값을 읽기 때문이다. 다시 만들려면 이 폴더에서 `python3 make.py`를 실행한다.
"""

import datetime
import zipfile

ZIP_DATE = (2026, 11, 3, 0, 0, 0)


def write(zf, name, text, compress=True):
    info = zipfile.ZipInfo(name, ZIP_DATE)
    info.compress_type = zipfile.ZIP_DEFLATED if compress else zipfile.ZIP_STORED
    zf.writestr(info, text)


def serial(value):
    base = datetime.datetime(1899, 12, 30)
    delta = value - base
    return delta.days + delta.seconds / 86400


def xlsx(path):
    strings = ["항목", "담당", "날짜", "금액", "두 배", "재시도 횟수", "운영", "간격", "비고", "a|b", "줄\n바꿈", "비밀"]
    s = {text: str(i) for i, text in enumerate(strings)}

    def sst():
        items = "".join(f'<si><t xml:space="preserve">{t}</t></si>' for t in strings)
        return (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            f'<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="{len(strings)}" uniqueCount="{len(strings)}">{items}</sst>'
        )

    def sheet(rows, merges=""):
        return (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
            f"<sheetData>{rows}</sheetData>{merges}</worksheet>"
        )

    date = serial(datetime.datetime(2026, 11, 3))
    stamp = serial(datetime.datetime(2026, 11, 4, 10, 30))
    meeting = sheet(
        '<row r="1">'
        f'<c r="A1" t="s"><v>{s["항목"]}</v></c><c r="B1" t="s"><v>{s["담당"]}</v></c>'
        f'<c r="C1" t="s"><v>{s["날짜"]}</v></c><c r="D1" t="s"><v>{s["금액"]}</v></c>'
        f'<c r="E1" t="s"><v>{s["두 배"]}</v></c></row>'
        '<row r="2">'
        f'<c r="A2" t="s"><v>{s["재시도 횟수"]}</v></c><c r="B2" t="s"><v>{s["운영"]}</v></c>'
        f'<c r="C2" s="1"><v>{date}</v></c><c r="D2"><v>3</v></c>'
        '<c r="E2"><f>D2*2</f><v>6</v></c></row>'
        '<row r="4">'
        f'<c r="A4" t="s"><v>{s["간격"]}</v></c><c r="B4"/>'
        f'<c r="C4" s="2"><v>{stamp}</v></c><c r="D4"><v>1.5</v></c>'
        '<c r="E4"><f>D4*2</f><v>3</v></c></row>'
        '<row r="5">'
        f'<c r="A5" t="s"><v>{s["비고"]}</v></c><c r="B5" t="s"><v>{s["a|b"]}</v></c>'
        f'<c r="C5" t="s"><v>{s["줄\n바꿈"]}</v></c>'
        '<c r="E5" t="e"><f>1/0</f><v>#DIV/0!</v></c></row>',
        '<mergeCells count="1"><mergeCell ref="A4:B4"/></mergeCells>',
    )
    secret = sheet(f'<row r="1"><c r="A1" t="s"><v>{s["비밀"]}</v></c></row>')

    with zipfile.ZipFile(path, "w") as zf:
        write(zf, "[Content_Types].xml",
              '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
              '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
              '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
              '<Default Extension="xml" ContentType="application/xml"/>'
              '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
              '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
              '<Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
              '<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>'
              '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>'
              '<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>'
              '</Types>')
        write(zf, "_rels/.rels",
              '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
              '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
              '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
              '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>'
              '</Relationships>')
        write(zf, "docProps/core.xml",
              '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
              '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" '
              'xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" '
              'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
              '<dc:title>정산 회의 자료</dc:title>'
              '<dcterms:created xsi:type="dcterms:W3CDTF">2026-11-01T03:00:00Z</dcterms:created>'
              '<dcterms:modified xsi:type="dcterms:W3CDTF">2026-11-03T03:00:00Z</dcterms:modified>'
              '</cp:coreProperties>')
        write(zf, "xl/workbook.xml",
              '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
              '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
              'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
              '<sheets><sheet name="회의" sheetId="1" r:id="rId1"/>'
              '<sheet name="내부" sheetId="2" state="hidden" r:id="rId2"/></sheets></workbook>')
        write(zf, "xl/_rels/workbook.xml.rels",
              '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
              '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
              '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>'
              '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>'
              '<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>'
              '<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
              '</Relationships>')
        write(zf, "xl/styles.xml",
              '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
              '<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
              '<fonts count="1"><font/></fonts><fills count="1"><fill/></fills><borders count="1"><border/></borders>'
              '<cellStyleXfs count="1"><xf numFmtId="0"/></cellStyleXfs>'
              '<cellXfs count="3"><xf numFmtId="0" xfId="0"/>'
              '<xf numFmtId="14" xfId="0" applyNumberFormat="1"/>'
              '<xf numFmtId="22" xfId="0" applyNumberFormat="1"/></cellXfs>'
              '</styleSheet>')
        write(zf, "xl/sharedStrings.xml", sst())
        write(zf, "xl/worksheets/sheet1.xml", meeting)
        write(zf, "xl/worksheets/sheet2.xml", secret)


def ods(path):
    ns = (
        'xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" '
        'xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" '
        'xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" '
        'xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" '
        'xmlns:of="urn:oasis:names:tc:opendocument:xmlns:of:1.2" '
        'xmlns:dc="http://purl.org/dc/elements/1.1/" '
        'xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" '
        'xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"'
    )

    def text(value):
        return f'<table:table-cell office:value-type="string"><text:p>{value}</text:p></table:table-cell>'

    def number(value, formula=None):
        f = f' table:formula="{formula}"' if formula else ""
        return f'<table:table-cell office:value-type="float" office:value="{value}"{f}><text:p>{value}</text:p></table:table-cell>'

    empty = "<table:table-cell/>"
    rows = [
        text("항목") + text("담당") + text("날짜") + text("금액") + text("두 배"),
        text("재시도 횟수") + text("운영")
        + '<table:table-cell office:value-type="date" office:date-value="2026-11-03"><text:p>2026-11-03</text:p></table:table-cell>'
        + number(3) + number(6, "of:=[.D2]*2"),
        empty * 5,
        '<table:table-cell office:value-type="string" table:number-columns-spanned="2"><text:p>간격</text:p></table:table-cell>'
        + "<table:covered-table-cell/>"
        + '<table:table-cell office:value-type="date" office:date-value="2026-11-04T10:30:00"><text:p>2026-11-04 10:30</text:p></table:table-cell>'
        + number(1.5) + number(3, "of:=[.D4]*2"),
        text("비고") + text("a|b") + '<table:table-cell office:value-type="string"><text:p>줄</text:p><text:p>바꿈</text:p></table:table-cell>' + empty + empty,
    ]
    body = "".join(f"<table:table-row>{row}</table:table-row>" for row in rows)
    content = (
        f'<?xml version="1.0" encoding="UTF-8"?><office:document-content {ns} office:version="1.2">'
        '<office:automatic-styles>'
        '<style:style style:name="shown" style:family="table"><style:table-properties table:display="true"/></style:style>'
        '<style:style style:name="hidden" style:family="table"><style:table-properties table:display="false"/></style:style>'
        '</office:automatic-styles><office:body><office:spreadsheet>'
        f'<table:table table:name="회의" table:style-name="shown">{body}</table:table>'
        f'<table:table table:name="내부" table:style-name="hidden"><table:table-row>{text("비밀")}</table:table-row></table:table>'
        '</office:spreadsheet></office:body></office:document-content>'
    )
    meta = (
        f'<?xml version="1.0" encoding="UTF-8"?><office:document-meta {ns} office:version="1.2"><office:meta>'
        '<dc:title>정산 회의 자료</dc:title><dc:date>2026-11-03T12:00:00</dc:date>'
        '</office:meta></office:document-meta>'
    )
    manifest = (
        f'<?xml version="1.0" encoding="UTF-8"?><manifest:manifest {ns} manifest:version="1.2">'
        '<manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>'
        '<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>'
        '<manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/>'
        '</manifest:manifest>'
    )
    with zipfile.ZipFile(path, "w") as zf:
        write(zf, "mimetype", "application/vnd.oasis.opendocument.spreadsheet", compress=False)
        write(zf, "content.xml", content)
        write(zf, "meta.xml", meta)
        write(zf, "META-INF/manifest.xml", manifest)


if __name__ == "__main__":
    xlsx("meeting.xlsx")
    ods("meeting.ods")
