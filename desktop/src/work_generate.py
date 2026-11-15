# Fixed, bounded Office tools. Inputs are data; no eval, macros, or external fetches.
import io, json, math, re, sys, zipfile, xml.etree.ElementTree as ET
from xml.sax.saxutils import escape, quoteattr

NS_REL = 'http://schemas.openxmlformats.org/package/2006/relationships'
NS_DOCREL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
NS_CT = 'http://schemas.openxmlformats.org/package/2006/content-types'
W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
A = 'http://schemas.openxmlformats.org/drawingml/2006/main'
P = 'http://schemas.openxmlformats.org/presentationml/2006/main'
S = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
XML = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
MAX_PACKAGE = 64 * 1024 * 1024


def txt(value):
    return escape(''.join(c for c in str(value) if c in '\t\n\r' or (32 <= ord(c) <= 0xD7FF) or (0xE000 <= ord(c) <= 0xFFFD) or (0x10000 <= ord(c) <= 0x10FFFF)))


def rels(entries):
    return XML + f'<Relationships xmlns="{NS_REL}">' + ''.join(f'<Relationship Id="rId{i}" Type="{NS_DOCREL}/{kind}" Target={quoteattr(target)}/>' for i, kind, target in entries) + '</Relationships>'


def types(entries):
    return XML + f'<Types xmlns="{NS_CT}"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>' + ''.join(f'<Override PartName={quoteattr(name)} ContentType={quoteattr(kind)}/>' for name, kind in entries) + '</Types>'


def bounded_string(value, name, limit):
    if not isinstance(value, str) or len(value) > limit:
        raise ValueError(f'{name} 必须为不超过 {limit} 字的文字')
    return value


def column(i):
    result = ''
    while i:
        i, rem = divmod(i - 1, 26)
        result = chr(65 + rem) + result
    return result


# Deliberately small spreadsheet expression language. Formula-looking strings
# remain literal text; formulas require an explicit {"formula": "..."} cell.
class Formula:
    TOKEN = re.compile(r'\s*(?:(\d+(?:\.\d*)?(?:[eE][+-]?\d+)?|\.\d+)|([A-Za-z_$][A-Za-z0-9_$]*)|("(?:[^"]|"")*")|(>=|<=|<>|[+\-*/^(),:<>=%]))')

    def __init__(self, text, cell):
        self.cell = cell
        text = bounded_string(text, 'formula', 2000).strip().lstrip('=')
        self.tokens = []
        pos = 0
        while pos < len(text):
            m = self.TOKEN.match(text, pos)
            if not m:
                if text[pos:].strip() == '': break
                raise ValueError('公式包含不支持的字符或外部引用')
            self.tokens.append(next(g for g in m.groups() if g is not None))
            pos = m.end()
        if not self.tokens or len(self.tokens) > 400:
            raise ValueError('公式为空或过长')
        for token in self.tokens:
            if re.fullmatch(r'[A-Za-z_$][A-Za-z0-9_$]*', token) and not re.fullmatch(r'\$?[A-Za-z]{1,3}\$?[1-9][0-9]{0,4}', token) and token.upper() not in ('TRUE','FALSE','SUM','AVERAGE','MIN','MAX','COUNT','ROUND','ABS','IF'):
                raise ValueError(f'公式标识符不受支持：{token}')
        self.i = 0

    def peek(self):
        return self.tokens[self.i] if self.i < len(self.tokens) else ''

    def take(self):
        t = self.peek()
        if not t: raise ValueError('公式不完整')
        self.i += 1
        return t

    @staticmethod
    def number(value):
        if value is None or value == '': return 0.0
        if isinstance(value, (int, float)) and math.isfinite(value): return value
        raise ValueError('算术公式引用了非数值单元格')

    def expression(self, level=0):
        if level > 80: raise ValueError('公式嵌套过深')
        left = self.sum(level + 1)
        if self.peek() in ('=', '<>', '<', '>', '<=', '>='):
            op = self.take()
            right = self.sum(level + 1)
            if op == '=': return left == right
            if op == '<>': return left != right
            if op == '<': return left < right
            if op == '>': return left > right
            if op == '<=': return left <= right
            return left >= right
        return left

    def sum(self, level):
        left = self.product(level)
        while self.peek() in ('+', '-'):
            op = self.take(); right = self.product(level)
            left = self.number(left) + self.number(right) if op == '+' else self.number(left) - self.number(right)
        return left

    def product(self, level):
        left = self.power(level)
        while self.peek() in ('*', '/'):
            op = self.take(); right = self.number(self.power(level)); left = self.number(left)
            if op == '*': left *= right
            else: left /= right
        return left

    def power(self, level):
        left = self.atom(level)
        while self.peek() == '^':
            self.take(); right = self.number(self.atom(level + 1)); left = self.number(left)
            if abs(right) > 12 or abs(left) > 1e12: raise ValueError('幂运算超出计算范围')
            left = left ** right
        return left

    def atom(self, level):
        if level > 80: raise ValueError('公式嵌套过深')
        token = self.take()
        if token in ('+', '-'):
            value = self.number(self.atom(level + 1))
            return value if token == '+' else -value
        if token == '(':
            value = self.expression(level + 1)
            if self.take() != ')': raise ValueError('公式括号不匹配')
        elif token.startswith('"'):
            value = token[1:-1].replace('""', '"')
        elif re.fullmatch(r'(?:\d+(?:\.\d*)?(?:[eE][+-]?\d+)?|\.\d+)', token):
            value = float(token)
        elif re.fullmatch(r'\$?[A-Za-z]{1,3}\$?[1-9][0-9]{0,4}', token):
            first = token.replace('$', '').upper()
            if self.peek() == ':':
                self.take(); last = self.take().replace('$', '').upper()
                value = self.range(first, last)
            else:
                value = self.cell(first)
                if value is None: value = 0
        elif token.upper() in ('TRUE', 'FALSE') and self.peek() != '(':
            value = token.upper() == 'TRUE'
        else:
            function = token.upper()
            if function not in ('SUM', 'AVERAGE', 'MIN', 'MAX', 'COUNT', 'ROUND', 'ABS', 'IF'):
                raise ValueError(f'不支持公式函数 {function}；仅支持 SUM/AVERAGE/MIN/MAX/COUNT/ROUND/ABS/IF')
            if self.take() != '(': raise ValueError('公式函数缺少左括号')
            if function == 'IF':
                # Select the branch before evaluating it, including divide-by-zero
                # guards such as IF(B2=0,0,A2/B2).
                groups, current, depth = [], [], 0
                while True:
                    part = self.take()
                    if part == ')' and depth == 0:
                        groups.append(current); break
                    if part == ',' and depth == 0:
                        groups.append(current); current = []; continue
                    if part == '(': depth += 1
                    if part == ')': depth -= 1
                    current.append(part)
                if len(groups) != 3 or not all(groups): raise ValueError('IF 需要三个参数')
                condition = Formula(''.join(groups[0]), self.cell).value()
                return Formula(''.join(groups[1] if bool(condition) else groups[2]), self.cell).value()
            args = []
            if self.peek() != ')':
                while True:
                    args.append(self.expression(level + 1))
                    if self.peek() != ',': break
                    self.take()
            if self.take() != ')': raise ValueError('公式函数缺少右括号')
            flat = [x for arg in args for x in (arg if isinstance(arg, list) else [arg])]
            nums = [x for x in flat if isinstance(x, (int, float)) and not isinstance(x, bool)]
            if function == 'SUM': value = sum(nums)
            elif function == 'COUNT': value = len(nums)
            elif function in ('AVERAGE', 'MIN', 'MAX'):
                if not nums: raise ValueError(f'{function} 没有数值')
                value = sum(nums) / len(nums) if function == 'AVERAGE' else (min(nums) if function == 'MIN' else max(nums))
            elif function == 'ROUND':
                if len(args) != 2: raise ValueError('ROUND 需要两个参数')
                places = self.number(args[1])
                if places != int(places) or abs(places) > 15: raise ValueError('ROUND 精度无效')
                # Decimal ROUND_HALF_UP matches spreadsheet ties, not Python bankers rounding.
                from decimal import Decimal, ROUND_HALF_UP
                value = float(Decimal(str(self.number(args[0]))).quantize(Decimal(1).scaleb(-int(places)), rounding=ROUND_HALF_UP))
            elif function == 'ABS':
                if len(args) != 1: raise ValueError('ABS 需要一个参数')
                value = abs(self.number(args[0]))
        if self.peek() == '%':
            self.take(); value = self.number(value) / 100
        return value

    def range(self, first, last):
        def coord(s):
            m = re.fullmatch(r'([A-Z]{1,3})([1-9][0-9]{0,4})', s)
            if not m: raise ValueError('单元格范围无效')
            c = 0
            for letter in m[1]: c = c * 26 + ord(letter) - 64
            return c, int(m[2])
        x1, y1 = coord(first); x2, y2 = coord(last)
        if x1 > x2 or y1 > y2 or x2 > 256 or y2 > 10000 or (x2-x1+1)*(y2-y1+1) > 10000:
            raise ValueError('公式引用范围过大或无效')
        return [self.cell(column(x)+str(y)) for y in range(y1, y2+1) for x in range(x1, x2+1)]

    def value(self):
        value = self.expression()
        if self.i != len(self.tokens) or isinstance(value, list): raise ValueError('公式包含未处理内容')
        if value is not None and not isinstance(value, (str, bool, int, float)): raise ValueError('公式结果不是可写入的单元格值')
        if isinstance(value, (int, float)) and not math.isfinite(value): raise ValueError('公式结果不是有限数字')
        return value


def spreadsheet(z, payload):
    rows = payload.get('rows', [])
    if not isinstance(rows, list) or not rows or len(rows) > 10000:
        raise ValueError('rows 应为 1–10000 行的二维数组')
    data, cached, pending = {}, {}, set()
    for ri, row in enumerate(rows, 1):
        if not isinstance(row, list) or len(row) > 256: raise ValueError('每行最多 256 列')
        for ci, value in enumerate(row, 1):
            if isinstance(value, dict):
                if set(value)-{'formula', 'value', 'format'} or (('formula' in value) == ('value' in value)):
                    raise ValueError('单元格对象需要 formula 或 value，并可包含 format')
                if 'formula' in value and not isinstance(value['formula'], str): raise ValueError('formula 必须为文字')
                if 'value' in value and not (value['value'] is None or isinstance(value['value'], (str, int, float, bool))): raise ValueError('value 必须是普通单元格值')
            elif not (value is None or isinstance(value, (str, int, float, bool))):
                raise ValueError('单元格仅支持文字、有限数字、布尔值、空值或公式对象')
            if isinstance(value, (int, float)) and not math.isfinite(value): raise ValueError('单元格数字必须有限')
            data[column(ci)+str(ri)] = value
    def cell(address):
        if address in cached: return cached[address]
        if address in pending: raise ValueError('公式存在循环引用')
        if len(pending) > 100: raise ValueError('公式依赖过深')
        value = data.get(address)
        if isinstance(value, dict):
            if 'formula' in value:
                pending.add(address)
                try: value = Formula(value['formula'], cell).value()
                finally: pending.discard(address)
            else: value = value['value']
        if isinstance(value, (int, float)) and not math.isfinite(value): raise ValueError('单元格数字必须有限')
        cached[address] = value
        return value
    body = []
    for ri, row in enumerate(rows, 1):
        cells = []
        for ci, item in enumerate(row, 1):
            address = column(ci) + str(ri); value = cell(address)
            fmt = item.get('format', '') if isinstance(item, dict) else ''
            if fmt not in ('', 'number', 'currency', 'percent'): raise ValueError('format 仅支持 number/currency/percent')
            style = {'number':2, 'currency':3, 'percent':4}.get(fmt, 1 if ri == 1 else 0)
            formula = '<f>'+txt(item['formula'].strip().lstrip('='))+'</f>' if isinstance(item, dict) and 'formula' in item else ''
            if isinstance(value, bool): contents = f' t="b">{formula}<v>{int(value)}</v>'
            elif isinstance(value, (int, float)): contents = f'>{formula}<v>{value}</v>'
            elif formula: contents = f' t="str">{formula}<v>{txt(value or "")}</v>'
            else: contents = f' t="inlineStr"><is><t xml:space="preserve">{txt("" if value is None else value)}</t></is>'
            cells.append(f'<c r="{address}" s="{style}"'+contents+'</c>')
        body.append(f'<row r="{ri}" ht="{28 if ri == 1 else 22}" customHeight="1">'+''.join(cells)+'</row>')
    maxcol = max(map(len, rows)) or 1
    dimension = f'A1:{column(maxcol)}{len(rows)}'
    widths = payload.get('column_widths', [22]*maxcol)
    if not isinstance(widths, list) or len(widths) != maxcol or not all(isinstance(v, (int, float)) and math.isfinite(v) and 8 <= v <= 80 for v in widths): raise ValueError('column_widths 每列需要 8–80 之间的宽度')
    cols = ''.join(f'<col min="{i}" max="{i}" width="{width}" customWidth="1"/>' for i,width in enumerate(widths, 1))
    sheet = XML + f'<worksheet xmlns="{S}"><dimension ref="{dimension}"/><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews><cols>{cols}</cols><sheetData>'+''.join(body)+f'</sheetData><autoFilter ref="{dimension}"/></worksheet>'
    z.writestr('xl/worksheets/sheet1.xml', sheet)
    name = bounded_string(payload.get('sheet_name', '工作表'), 'sheet_name', 31)
    if not name or any(c in name for c in '[]:*?/\\'): raise ValueError('工作表名称无效')
    z.writestr('xl/workbook.xml', XML + f'<workbook xmlns="{S}" xmlns:r="{NS_DOCREL}"><sheets><sheet name={quoteattr(name)} sheetId="1" r:id="rId1"/></sheets><calcPr calcId="191029" fullCalcOnLoad="1"/></workbook>')
    z.writestr('xl/styles.xml', XML+f'<styleSheet xmlns="{S}"><numFmts count="1"><numFmt numFmtId="164" formatCode="&quot;¥&quot;#,##0.00"/></numFmts><fonts count="2"><font><sz val="11"/><name val="Aptos"/></font><font><b/><color rgb="FFFFFFFF"/><sz val="11"/><name val="Aptos"/></font></fonts><fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FF233247"/><bgColor indexed="64"/></patternFill></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="5"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="0" applyAlignment="1"><alignment vertical="center"/></xf><xf numFmtId="2" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="10" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>')
    z.writestr('xl/_rels/workbook.xml.rels', rels([(1, 'worksheet', 'worksheets/sheet1.xml'), (2, 'styles', 'styles.xml')]))
    z.writestr('_rels/.rels', rels([(1, 'officeDocument', 'xl/workbook.xml')]))
    z.writestr('[Content_Types].xml', types([('/xl/workbook.xml','application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml'),('/xl/worksheets/sheet1.xml','application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml'),('/xl/styles.xml','application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml')]))


def document(z, payload):
    paragraphs = payload.get('paragraphs', [])
    if not isinstance(paragraphs, list) or len(paragraphs) > 2000:
        raise ValueError('paragraphs 最多 2000 段')
    title = bounded_string(payload.get('title', ''), 'title', 200)
    nodes = []
    if title:
        nodes.append('<w:p><w:pPr><w:spacing w:after="260"/></w:pPr><w:r><w:rPr><w:b/><w:color w:val="233247"/><w:sz w:val="36"/></w:rPr><w:t xml:space="preserve">'+txt(title)+'</w:t></w:r></w:p>')
    for p in paragraphs:
        style = 'body'
        if isinstance(p, dict): style, p = p.get('style', 'body'), p.get('text')
        p = bounded_string(p, 'paragraph.text', 20000)
        if style not in ('heading1', 'heading2', 'body', 'bullet', 'quote'): raise ValueError('段落 style 不受支持')
        size = {'heading1':30, 'heading2':26}.get(style, 22)
        decoration = '<w:b/><w:color w:val="233247"/>' if style.startswith('heading') else ('<w:i/><w:color w:val="53677F"/>' if style == 'quote' else '')
        prop = '<w:keepNext/><w:spacing w:before="260" w:after="160"/>' if style.startswith('heading') else '<w:spacing w:after="160" w:line="320" w:lineRule="auto"/>'
        if style in ('bullet', 'quote'): prop += '<w:ind w:left="360"/>'
        for line in p.split('\n'):
            if style == 'bullet': line = '•  ' + line
            nodes.append(f'<w:p><w:pPr>{prop}</w:pPr><w:r><w:rPr><w:rFonts w:ascii="Aptos" w:eastAsia="Microsoft YaHei"/><w:sz w:val="{size}"/>{decoration}</w:rPr><w:t xml:space="preserve">'+txt(line)+'</w:t></w:r></w:p>')
    z.writestr('word/document.xml', XML+f'<w:document xmlns:w="{W}"><w:body>'+''.join(nodes)+'<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr></w:body></w:document>')
    z.writestr('_rels/.rels', rels([(1, 'officeDocument', 'word/document.xml')]))
    z.writestr('[Content_Types].xml', types([('/word/document.xml','application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml')]))


def safe_package(path):
    z = zipfile.ZipFile(path)
    infos = z.infolist()
    if len(infos) > 10000 or len({i.filename for i in infos}) != len(infos) or sum(i.file_size for i in infos) > MAX_PACKAGE:
        z.close(); raise ValueError('Office 文件内部大小或文件数超过限制')
    if any('vbaproject' in i.filename.lower() or i.filename.startswith('_xmlsignatures/') for i in infos):
        z.close(); raise ValueError('不能修改带宏或数字签名的 Office 文件')
    return z


def parse_xml(data):
    if len(data) > 8*1024*1024 or b'<!DOCTYPE' in data or b'<!ENTITY' in data:
        raise ValueError('文档 XML 过大或包含外部实体')
    return ET.fromstring(data)


def edit_document(z, payload, source):
    replacements = payload.get('replacements')
    if not isinstance(replacements, list) or not 1 <= len(replacements) <= 100:
        raise ValueError('replacements 需要 1–100 组精确替换')
    with safe_package(source) as original:
        if 'word/document.xml' not in original.namelist(): raise ValueError('来源不是 Word 文档')
        xmls = {}
        for name in original.namelist():
            if name.startswith('word/') and name.endswith('.xml'):
                data = original.read(name)
                root = parse_xml(data)
                if list(root.iter('{'+W+'}p')): xmls[name] = [data, root, False]
        for replacement in replacements:
            find = bounded_string(replacement.get('find'), 'find', 10000)
            replace = bounded_string(replacement.get('replace'), 'replace', 10000)
            if not find or '\n' in find or '\n' in replace: raise ValueError('每次替换限定同一段落；find 不能为空')
            expected = replacement.get('expected_count', 1)
            if not isinstance(expected, int) or isinstance(expected, bool) or not 1 <= expected <= 10000:
                raise ValueError('expected_count 必须为 1–10000')
            matches, count = [], 0
            for name, (_, root, _) in xmls.items():
                for paragraph in root.iter('{'+W+'}p'):
                    nodes = list(paragraph.iter('{'+W+'}t'))
                    text = ''.join(n.text or '' for n in nodes)
                    starts = []
                    for match in re.finditer(re.escape(find), text):
                        count += 1
                        if count > expected: raise ValueError(f'精确替换匹配超过 {expected} 处；未生成修改文件')
                        starts.append(match.start())
                    if starts: matches.append((name, nodes, text, starts))
            if count != expected:
                raise ValueError(f'精确替换匹配 {count} 处，expected_count={expected}；未生成修改文件')
            if max(0, len(replace)-len(find))*count > 4*1024*1024:
                raise ValueError('替换内容增长超过 4 MB')
            for name, nodes, text, starts in matches:
                offsets, lengths, offset = [], [], 0
                for node in nodes:
                    length = len(node.text or '')
                    offsets.append(offset); lengths.append(length); offset += length
                for start in reversed(starts):
                    end = start + len(find)
                    first = next(i for i in range(len(nodes)) if offsets[i]+lengths[i] > start)
                    last = next(i for i in range(len(nodes)) if offsets[i]+lengths[i] >= end)
                    prefix = (nodes[first].text or '')[:start-offsets[first]]
                    suffix = (nodes[last].text or '')[end-offsets[last]:]
                    nodes[first].text = prefix + replace + (suffix if first == last else '')
                    nodes[first].set('{http://www.w3.org/XML/1998/namespace}space', 'preserve')
                    for i in range(first+1, last): nodes[i].text = ''
                    if first != last: nodes[last].text = suffix
                xmls[name][2] = True
        for item in original.infolist():
            data = original.read(item.filename)
            if item.filename in xmls and xmls[item.filename][2]:
                before, root, _ = xmls[item.filename]
                ns = list(ET.iterparse(io.BytesIO(before), events=['start-ns']))
                for _, (prefix, uri) in ns:
                    if not re.fullmatch(r'ns\d+', prefix): ET.register_namespace(prefix, uri)
                data = ET.tostring(root, encoding='utf-8', xml_declaration=True)
                # Preserve declarations referenced by mc:Ignorable attribute values.
                old_root = re.search(rb'<[A-Za-z_][^>]*>', before).group(0)
                new_root = re.search(rb'<[A-Za-z_][^>]*>', data).group(0)
                declarations = re.findall(rb'xmlns(?::[\w.-]+)?=["\'][^"\']+["\']', old_root)
                missing = [d for d in declarations if d.split(b'=')[0]+b'=' not in new_root]
                if missing: data = data.replace(new_root, new_root[:-1]+b' '+b' '.join(missing)+b'>', 1)
            z.writestr(item, data)


def text_shape(identity, x, y, width, height, lines, size, color, bold=False):
    paragraphs = ''.join(f'<a:p><a:pPr marL="0" indent="0"><a:lnSpc><a:spcPct val="115000"/></a:lnSpc><a:spcAft><a:spcPts val="1000"/></a:spcAft></a:pPr><a:r><a:rPr lang="zh-CN" sz="{size*100}" b="{int(bold)}"><a:solidFill><a:srgbClr val="{color}"/></a:solidFill><a:latin typeface="Aptos"/><a:ea typeface="Microsoft YaHei"/></a:rPr><a:t>{txt(line)}</a:t></a:r><a:endParaRPr lang="zh-CN"/></a:p>' for line in lines)
    return f'<p:sp><p:nvSpPr><p:cNvPr id="{identity}" name="Text {identity}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{width}" cy="{height}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap="square" lIns="0" tIns="0" rIns="0" bIns="0"><a:normAutofit/></a:bodyPr><a:lstStyle/>{paragraphs}</p:txBody></p:sp>'


def fit_font(lines, width, height, largest, smallest):
    # Conservative CJK-aware layout estimate; refuse dense slides instead of
    # silently clipping them. Actual Office rendering still uses normal autofit.
    for size in range(largest, smallest-1, -1):
        capacity = width / 12700 / size * 0.90
        count = sum(max(1, math.ceil(sum(0.55 if ord(c) < 128 else 1 for c in line) / capacity)) for line in lines)
        if count * size * 1.15 + 10*len(lines) <= height / 12700:
            return size
    raise ValueError('幻灯片文字过多；请缩短标题或拆成更多页面，不要压缩成密集长段落')


def presentation(z, payload):
    slides = payload.get('slides')
    if not isinstance(slides, list) or not 1 <= len(slides) <= 40: raise ValueError('slides 需要 1–40 页')
    group = '<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>'
    attributes = f'xmlns:a="{A}" xmlns:r="{NS_DOCREL}" xmlns:p="{P}"'
    entries = [('/ppt/presentation.xml','application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml'),('/ppt/slideMasters/slideMaster1.xml','application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml'),('/ppt/slideLayouts/slideLayout1.xml','application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml'),('/ppt/theme/theme1.xml','application/vnd.openxmlformats-officedocument.theme+xml')]
    for index, slide in enumerate(slides, 1):
        if not isinstance(slide, dict): raise ValueError('每页必须是对象')
        title = bounded_string(slide.get('title', ''), 'slide.title', 80)
        subtitle = bounded_string(slide.get('subtitle', ''), 'slide.subtitle', 160)
        bullets = slide.get('bullets', [])
        if not isinstance(bullets, list) or len(bullets) > 6: raise ValueError('每页最多 6 个要点，请拆分页面')
        bullets = [bounded_string(v, 'bullet', 160) for v in bullets]
        if not title: raise ValueError('每页必须有标题')
        cover = slide.get('layout') == 'cover' or (index == 1 and not bullets)
        if cover and bullets: raise ValueError('封面只放标题和副标题，请将要点放在内容页')
        dark = cover or slide.get('layout') == 'section'
        bg, fg, muted = ('18263B', 'FFFFFF', 'BDD0E3') if dark else ('F7F9FC', '18263B', '53677F')
        shapes = text_shape(2, 690000, 430000, 10800000, 500000, ['AMOO WORK  /  '+f'{index:02d}'], 12, '18B89A', True)
        title_height = 1500000 if cover else 900000
        title_size = fit_font([title], 10700000, title_height, 38 if cover else 30, 22)
        shapes += text_shape(3, 690000, 1700000 if cover else 1170000, 10700000, title_height, [title], title_size, fg, True)
        if subtitle: shapes += text_shape(4, 710000, 3300000 if cover else 2110000, 10300000, 700000, [subtitle], fit_font([subtitle],10300000,700000,19,14), muted)
        if bullets:
            lines = ['•  '+v for v in bullets]
            body_height = 3100000 if subtitle else 3600000
            shapes += text_shape(5, 800000, 2880000 if subtitle else 2350000, 10200000, body_height, lines, fit_font(lines,10200000,body_height,21,16), fg)
        shapes += text_shape(6, 710000, 6350000, 10000000, 300000, [f'{index} / {len(slides)}'], 10, muted)
        z.writestr(f'ppt/slides/slide{index}.xml', XML+f'<p:sld {attributes}><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="{bg}"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{group}{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>')
        z.writestr(f'ppt/slides/_rels/slide{index}.xml.rels', rels([(1, 'slideLayout', '../slideLayouts/slideLayout1.xml')]))
        entries.append((f'/ppt/slides/slide{index}.xml','application/vnd.openxmlformats-officedocument.presentationml.slide+xml'))
    z.writestr('ppt/presentation.xml', XML+f'<p:presentation {attributes}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>'+''.join(f'<p:sldId id="{255+i}" r:id="rId{i+1}"/>' for i in range(1,len(slides)+1))+'</p:sldIdLst><p:sldSz cx="12192000" cy="6858000" type="screen16x9"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>')
    z.writestr('ppt/_rels/presentation.xml.rels', rels([(1,'slideMaster','slideMasters/slideMaster1.xml')]+[(i+1,'slide',f'slides/slide{i}.xml') for i in range(1,len(slides)+1)]))
    cmap = '<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>'
    z.writestr('ppt/slideMasters/slideMaster1.xml', XML+f'<p:sldMaster {attributes}><p:cSld><p:spTree>{group}</p:spTree></p:cSld>{cmap}<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>')
    z.writestr('ppt/slideMasters/_rels/slideMaster1.xml.rels', rels([(1,'slideLayout','../slideLayouts/slideLayout1.xml'),(2,'theme','../theme/theme1.xml')]))
    z.writestr('ppt/slideLayouts/slideLayout1.xml', XML+f'<p:sldLayout {attributes} type="blank" preserve="1"><p:cSld name="Blank"><p:spTree>{group}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>')
    z.writestr('ppt/slideLayouts/_rels/slideLayout1.xml.rels', rels([(1,'slideMaster','../slideMasters/slideMaster1.xml')]))
    colors = [('dk1','18263B'),('lt1','FFFFFF'),('dk2','233247'),('lt2','F7F9FC'),('accent1','18B89A'),('accent2','467BCE'),('accent3','E2A43B'),('accent4','B96582'),('accent5','708BA9'),('accent6','82B5A2'),('hlink','2563EB'),('folHlink','7C3AED')]
    schemes = ''.join(f'<a:{k}><a:srgbClr val="{v}"/></a:{k}>' for k,v in colors)
    font = '<a:latin typeface="Aptos"/><a:ea typeface="Microsoft YaHei"/><a:cs typeface="Arial"/>'
    fill = '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
    line = '<a:ln w="9525">'+fill+'<a:prstDash val="solid"/></a:ln>'
    z.writestr('ppt/theme/theme1.xml', XML+f'<a:theme xmlns:a="{A}" name="Amoo Work"><a:themeElements><a:clrScheme name="Amoo">{schemes}</a:clrScheme><a:fontScheme name="Amoo"><a:majorFont>{font}</a:majorFont><a:minorFont>{font}</a:minorFont></a:fontScheme><a:fmtScheme name="Amoo"><a:fillStyleLst>{fill*3}</a:fillStyleLst><a:lnStyleLst>{line*3}</a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst>{fill*3}</a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>')
    z.writestr('_rels/.rels', rels([(1,'officeDocument','ppt/presentation.xml')]))
    z.writestr('[Content_Types].xml', types(entries))


def main():
    payload = json.load(sys.stdin)
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, 'w', compression=zipfile.ZIP_DEFLATED) as z:
        tool = payload.get('tool')
        if tool == 'write_document': document(z, payload)
        elif tool == 'write_spreadsheet': spreadsheet(z, payload)
        elif tool == 'write_presentation': presentation(z, payload)
        elif tool == 'edit_document': edit_document(z, payload, sys.argv[1])
        else: raise ValueError('未知文件工具')
    result = buf.getvalue()
    if len(result) > 4*1024*1024: raise ValueError('生成文件超过 4 MB')
    sys.stdout.buffer.write(result)


if __name__ == '__main__':
    try:
        main()
    except PermissionError:
        sys.stderr.write('目标文件正在被其他程序占用，请关闭后重试\n')
        sys.exit(2)
    except (zipfile.BadZipFile, ET.ParseError, KeyError):
        sys.stderr.write('文档格式已损坏或受密码保护，无法读取\n')
        sys.exit(2)
    except RuntimeError as error:
        if 'encrypted' in str(error).lower() or 'password' in str(error).lower():
            sys.stderr.write('文档格式已损坏或受密码保护，无法读取\n')
        else:
            sys.stderr.write((str(error) or '文档生成失败')[:400] + '\n')
        sys.exit(2)
    except (ValueError, TypeError, ZeroDivisionError, OverflowError) as error:
        sys.stderr.write(str(error)[:400] + '\n')
        sys.exit(2)
    except OSError:
        sys.stderr.write('无法读写目标文档，请检查文件权限或磁盘空间\n')
        sys.exit(2)
