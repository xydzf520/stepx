# Fixed Office text reader; never runs document macros or embedded scripts.
import os, sys, zipfile, posixpath, json, xml.etree.ElementTree as ET

def is_ignored_workspace_entry(name):
    return (
        not name
        or name.startswith('.')
        or name.startswith('~$')
        or name.endswith(('.tmp', '.temp', '.swp', '.crdownload', '.part', '.stepxlink'))
        or name in ('StepX Companion', '__pycache__', 'node_modules', 'target')
    )

def search_files(root_dir, query):
    matches = []
    if not query or len(query.encode('utf-8')) >= 500:
        return matches
    for entry in sorted(os.listdir(root_dir))[:100]:
        if is_ignored_workspace_entry(entry):
            continue
        full_path = os.path.join(root_dir, entry)
        if not os.path.isfile(full_path) or os.path.islink(full_path):
            continue
        try:
            with open(full_path, 'r', encoding='utf-8', errors='ignore') as f:
                for idx, line in enumerate(f, 1):
                    if query in line:
                        matches.append({'path': entry, 'line': idx, 'text': line.rstrip('\r\n')[:500]})
                        if len(matches) >= 50:
                            return matches
        except OSError:
            continue
    return matches

p = sys.argv[1]
preview = '--preview' in sys.argv[2:]
preview_sheets = []
MAX_XML = 8 * 1024 * 1024
MAX_TEXT = 65536
parts = []
size = 0

def append(value):
    global size
    if not value or size >= MAX_TEXT:
        return
    value = str(value)
    b = value.encode('utf-8')
    available = MAX_TEXT - size
    if len(b) > available:
        value = b[:available].decode('utf-8', errors='ignore')
    parts.append(value)
    size += len(value.encode('utf-8')) + 1

def tree(z, name):
    info = z.getinfo(name)
    if info.file_size > MAX_XML:
        raise ValueError('文档 XML 过大')
    data = z.read(name)
    if b'<!DOCTYPE' in data or b'<!ENTITY' in data:
        raise ValueError('不支持带外部实体的文档')
    return ET.fromstring(data)

with zipfile.ZipFile(p) as z:
    names = z.namelist()
    if len(names) > 10000 or len(set(names)) != len(names) or sum(i.file_size for i in z.infolist()) > 64 * 1024 * 1024:
        raise ValueError('文档内部文件过多或解压大小超过 64 MB')
    if 'word/document.xml' in names:
        ns = {'w':'http://schemas.openxmlformats.org/wordprocessingml/2006/main'}
        W_P = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}p'
        W_TBL = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}tbl'
        W_VAL = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}val'
        root = tree(z, 'word/document.xml')
        body = root.find('w:body', ns) or root
        def format_para(paragraph):
            text = ''.join(t.text or '' for t in paragraph.findall('.//w:t', ns)).strip()
            if not text:
                return ''
            pstyle_el = paragraph.find('w:pPr/w:pStyle', ns)
            pstyle = (pstyle_el.get(W_VAL) or pstyle_el.get('val') or '') if pstyle_el is not None else ''
            pstyle_norm = pstyle.lower().replace(' ', '').replace('-', '')
            sz_el = paragraph.find('.//w:rPr/w:sz', ns)
            sz = (sz_el.get(W_VAL) or sz_el.get('val') or '') if sz_el is not None else ''
            has_bold = paragraph.find('.//w:rPr/w:b', ns) is not None
            has_italic = paragraph.find('.//w:rPr/w:i', ns) is not None
            has_ind = paragraph.find('w:pPr/w:ind', ns) is not None
            has_num = paragraph.find('w:pPr/w:numPr', ns) is not None
            if pstyle_norm in ('heading1', '1', 'title') or (not pstyle and has_bold and sz in ('36', '30')):
                return '# ' + text
            if pstyle_norm in ('heading2', '2', 'subtitle') or (not pstyle and has_bold and sz == '26'):
                return '## ' + text
            if pstyle_norm in ('heading3', '3'):
                return '### ' + text
            if pstyle_norm in ('listparagraph', 'bullet', 'listitem') or has_num or text.startswith('•'):
                return '- ' + text.lstrip('•').strip()
            if pstyle_norm in ('quote', 'intensequote', 'blockquote') or (not pstyle and has_italic and has_ind):
                return '> ' + text
            return text
        def format_table(tbl):
            rows = []
            for tr in tbl.findall('w:tr', ns):
                cells = []
                for tc in tr.findall('w:tc', ns):
                    cell_parts = []
                    for p_node in tc.findall('.//w:p', ns):
                        t_val = ''.join(n.text or '' for n in p_node.findall('.//w:t', ns)).strip()
                        if t_val:
                            cell_parts.append(t_val)
                    cells.append(' '.join(cell_parts).replace('|', '\\|').replace('\r', ' ').replace('\n', ' '))
                if cells:
                    rows.append(cells)
            if not rows:
                return
            cols = max(len(r) for r in rows)
            if cols == 0:
                return
            norm = [r + [''] * (cols - len(r)) for r in rows]
            append('| ' + ' | '.join(norm[0]) + ' |')
            append('| ' + ' | '.join(['---'] * cols) + ' |')
            for r in norm[1:]:
                append('| ' + ' | '.join(r) + ' |')
        def walk_docx(container):
            for child in container:
                if child.tag == W_P:
                    line = format_para(child)
                    if line:
                        append(line)
                elif child.tag == W_TBL:
                    format_table(child)
                else:
                    walk_docx(child)
        walk_docx(body)
    elif 'xl/workbook.xml' in names:
        ns = {'s':'http://schemas.openxmlformats.org/spreadsheetml/2006/main'}
        shared = []
        if 'xl/sharedStrings.xml' in names:
            for item in tree(z, 'xl/sharedStrings.xml').findall('s:si', ns):
                shared.append(''.join(t.text or '' for t in item.findall('.//s:t', ns)))
        rel = {'r': 'http://schemas.openxmlformats.org/package/2006/relationships'}
        relationships = {n.get('Id'):n.get('Target') for n in tree(z, 'xl/_rels/workbook.xml.rels').findall('r:Relationship', rel) if n.get('TargetMode') != 'External'}
        sheets = []
        for sheet in tree(z, 'xl/workbook.xml').findall('s:sheets/s:sheet', ns):
            target = relationships.get(sheet.get('{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id'))
            if target:
                name = posixpath.normpath('xl/' + target) if not target.startswith('/') else target.lstrip('/')
                if name.startswith('xl/worksheets/') and name in names: sheets.append((sheet.get('name',name),name))
        for label, name in sheets[:(3 if preview else 30)]:
            if not preview:
                append('工作表: ' + label)
            preview_rows = []
            for row in tree(z, name).findall('.//s:row', ns):
                if preview and len(preview_rows) >= 20:
                    break
                values = []
                preview_cells = [''] * 8
                for cell in row.findall('s:c', ns):
                    value = cell.findtext('s:v', default='', namespaces=ns)
                    typ = cell.get('t')
                    if typ == 's':
                        try: value = shared[int(value)]
                        except (ValueError, IndexError): value = '[无效共享字符串]'
                    elif typ == 'inlineStr':
                        value = ''.join(t.text or '' for t in cell.findall('.//s:t', ns))
                    formula = cell.findtext('s:f', default='', namespaces=ns)
                    if preview and len(preview_rows) < 20:
                        # Keep empty columns in place and use cached results only.
                        # Never evaluate workbook formulas, links or macros.
                        column = 0
                        for c in cell.get('r', ''):
                            if not ('A' <= c <= 'Z'): break
                            column = column * 26 + ord(c) - ord('A') + 1
                            if column > 8: break
                        if 1 <= column <= 8:
                            preview_cells[column-1] = (value if value != '' or not formula else '尚未计算')[:160]
                    if not preview:
                        if formula: value = f'={formula} → {value if value != "" else "[尚无计算缓存]"}'
                        values.append(f'{cell.get("r", "")}: {value}')
                if preview:
                    preview_rows.append({'row':row.get('r',''), 'cells':preview_cells})
                    if len(preview_rows) >= 20:
                        break
                else:
                    append(' | '.join(values))
                    if size >= MAX_TEXT: break
            if preview:
                preview_sheets.append({'name':label, 'rows':preview_rows})
                if len(preview_sheets) >= 3: break
            elif size >= MAX_TEXT:
                break
    elif 'ppt/presentation.xml' in names:
        ns = {'a':'http://schemas.openxmlformats.org/drawingml/2006/main'}
        rel = {'r': 'http://schemas.openxmlformats.org/package/2006/relationships'}
        relationships = {n.get('Id'):n.get('Target') for n in tree(z, 'ppt/_rels/presentation.xml.rels').findall('r:Relationship', rel) if n.get('TargetMode') != 'External'}
        slides = []
        pns = {'p':'http://schemas.openxmlformats.org/presentationml/2006/main'}
        for slide in tree(z, 'ppt/presentation.xml').findall('p:sldIdLst/p:sldId', pns):
            target = relationships.get(slide.get('{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id'))
            if target:
                name = posixpath.normpath('ppt/' + target) if not target.startswith('/') else target.lstrip('/')
                if name.startswith('ppt/slides/') and name in names: slides.append(name)
        for index, name in enumerate(slides[:100],1):
            append('幻灯片: ' + str(index))
            for node in tree(z, name).findall('.//a:t', ns): append(node.text or '')
            if size >= MAX_TEXT: break
    else:
        raise ValueError('未识别的 Office 文档')
if preview:
    if not any(s.get('rows') for s in preview_sheets):
        raise ValueError('文档没有可提取的文字')
    sys.stdout.write(json.dumps({'sheets':preview_sheets},ensure_ascii=False))
else:
    if size >= MAX_TEXT:
        parts.append('[达到 64 KB 阅读上限，后续内容未读取]')
    result = '\n'.join(parts)
    if not result.strip():
        raise ValueError('文档没有可提取的文字')
    sys.stdout.write(result)
