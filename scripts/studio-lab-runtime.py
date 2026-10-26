#!/usr/bin/env python3
"""Bounded, offline web-app workbench. No shell/npm or authenticated browser profile.
All stdout is line-delimited receipts; credentials enter once on stdin, never argv.
"""
import base64
import contextlib
import hashlib
import http.server
import json
import os
from pathlib import Path
import queue
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

MAX_HTML = 2 * 1024 * 1024
MAX_RESPONSE = 8 * 1024 * 1024
CSP = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; worker-src 'none'; sandbox allow-scripts allow-same-origin"

class Failure(Exception):
    pass

def emit(kind, **fields):
    print(json.dumps(dict(event=kind, **fields), ensure_ascii=False), flush=True)

def private_write(path, value):
    path = Path(path)
    data = value if isinstance(value, bytes) else value.encode('utf-8')
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as stream:
        stream.write(data)

def endpoint(config, suffix):
    base = config['endpoint'].rstrip('/')
    for tail in ['/chat/completions', '/messages', '/models']:
        if base.endswith(tail):
            base = base[:-len(tail)]
    return base + suffix

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        raise Failure('模型服务尝试跳转地址，请直接配置最终 API 地址')

def api(config, suffix, payload=None):
    headers = {'Accept': 'application/json', 'Content-Type': 'application/json'}
    if config['protocol'] == 'anthropic':
        headers.update({'x-api-key': config['key'], 'anthropic-version': '2023-06-01'})
    else:
        headers['Authorization'] = 'Bearer ' + config['key']
    request = urllib.request.Request(endpoint(config, suffix), headers=headers,
                                     data=None if payload is None else json.dumps(payload).encode())
    try:
        with urllib.request.build_opener(NoRedirect).open(request, timeout=150) as response:
            raw = response.read(MAX_RESPONSE + 1)
        if len(raw) > MAX_RESPONSE:
            raise Failure('模型输出过大，请缩小任务')
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise Failure('模型服务返回了无效响应')
        return value
    except urllib.error.HTTPError as error:
        raise Failure('模型接口返回 HTTP %d，请检查协议、模型权限或账户余额' % error.code) from None
    except (urllib.error.URLError, TimeoutError, OSError):
        raise Failure('模型接口连接失败或超时，请检查 API 地址和网络') from None
    except (ValueError, UnicodeError):
        raise Failure('模型接口没有返回有效 JSON，请确认 API 地址和协议') from None

def stream_complete(config, suffix, payload):
    headers = {'Accept':'text/event-stream', 'Content-Type':'application/json'}
    if config['protocol'] == 'anthropic':
        headers.update({'x-api-key':config['key'], 'anthropic-version':'2023-06-01'})
    else:
        headers['Authorization'] = 'Bearer '+config['key']
    payload['stream'] = True
    request = urllib.request.Request(endpoint(config,suffix),headers=headers,data=json.dumps(payload).encode())
    parts, usage, finish, final = [], {}, '', False
    began = progress = time.monotonic()
    size = chars = 0
    try:
        # Per-read timeout protects stalled streams; total deadline bounds a model
        # that continuously emits reasoning without ever delivering a final answer.
        with urllib.request.build_opener(NoRedirect).open(request,timeout=75) as response:
            emit('status',message='已连接模型，正在流式生成代码…')
            while True:
                if time.monotonic()-began>300:
                    raise Failure('模型生成超过 5 分钟上限；未完整返回的代码不会作为可交付作品')
                line=response.readline(1024*1024)
                if not line: break
                size+=len(line)
                if size>MAX_RESPONSE or len(line)>=1024*1024:
                    raise Failure('模型流式输出过大，请缩小任务')
                if not line.startswith(b'data:'): continue
                data=line[5:].strip()
                if data==b'[DONE]': final=True; break
                if not data: continue
                event=json.loads(data)
                if not isinstance(event,dict) or event.get('error') or event.get('type')=='error':
                    raise Failure('模型服务返回流式错误，请检查额度与权限')
                text=''
                if config['protocol']=='anthropic':
                    kind=event.get('type')
                    if kind=='content_block_delta':
                        delta=event.get('delta',{})
                        if delta.get('type')=='text_delta': text=delta.get('text','')
                    elif kind=='message_start': usage.update(event.get('message',{}).get('usage',{}))
                    elif kind=='message_delta':
                        usage.update(event.get('usage',{})); finish=event.get('delta',{}).get('stop_reason') or finish
                    elif kind=='message_stop': final=True; break
                else:
                    usage.update(event.get('usage') or {})
                    choices=event.get('choices') or []
                    if choices:
                        delta=choices[0].get('delta',{})
                        text=delta.get('content') or ''
                        if isinstance(text,list): text=''.join(x.get('text','') for x in text if isinstance(x,dict))
                        finish=choices[0].get('finish_reason') or finish
                if isinstance(text,str) and text:
                    parts.append(text); chars+=len(text)
                if time.monotonic()-progress>5:
                    emit('status',message=('正在生成代码 · 已接收 %d 字符'%chars) if chars else '模型正在组织方案 · 已用 %d 秒'%int(time.monotonic()-began))
                    progress=time.monotonic()
        if not final or not finish:
            raise Failure('模型流式连接提前结束，未收到完整完成标记；已忽略不完整代码')
        if finish in ('length','max_tokens'):
            raise Failure('模型输出被截断，请缩小任务或选择更长输出的模型')
        text=''.join(parts)
        if config.get('key'): text=text.replace(config['key'],'[credential-redacted]')
        if not text.strip(): raise Failure('模型未返回可用代码')
        return text,{k:v for k,v in usage.items() if k in ('prompt_tokens','completion_tokens','input_tokens','output_tokens') and isinstance(v,int)}
    except urllib.error.HTTPError as error:
        raise Failure('模型接口返回 HTTP %d，请检查协议、模型权限或账户余额'%error.code) from None
    except (urllib.error.URLError,TimeoutError,OSError):
        raise Failure('模型流式连接中断或超过 75 秒未返回数据，请检查网络') from None
    except (ValueError,UnicodeError):
        raise Failure('模型返回了无效的流式数据，请确认协议') from None

def complete(config, system, prompt, small=False):
    if config['protocol'] == 'phone':
        emit('model_request', system=system, prompt=prompt)
        line = sys.stdin.readline(MAX_RESPONSE + 1)
        if not line or len(line) > MAX_RESPONSE:
            raise Failure('手机模型响应通道中断或结果过大')
        try:
            reply = json.loads(line)
            text = reply['text']
            if not isinstance(text, str) or not text.strip():
                raise ValueError('empty response')
        except (ValueError, KeyError, TypeError):
            raise Failure('手机模型没有返回有效文本') from None
        return text, {}  # Actual phone usage is recorded by the Rust session bridge.
    if not small:
        if config['protocol']=='anthropic':
            return stream_complete(config,'/messages',{'model':config['model'],'system':system,'messages':[{'role':'user','content':prompt}],'max_tokens':16000})
        payload={'model':config['model'],'messages':[{'role':'system','content':system},{'role':'user','content':prompt}],'max_tokens':16000}
        # Step 3.7 supports an explicit effort dial. These bounded offline apps do
        # not need its lengthy default reasoning before every repair round.
        if urllib.parse.urlsplit(config['endpoint']).hostname in ('api.stepfun.com','api.stepfun.ai') and config['model'].startswith('step-3.7-flash'):
            payload['reasoning_effort']='low'
        return stream_complete(config,'/chat/completions',payload)
    if config['protocol'] == 'anthropic':
        response = api(config, '/messages', {'model': config['model'], 'system': system,
            'messages': [{'role': 'user', 'content': prompt}], 'max_tokens': 64 if small else 16000})
        blocks = response.get('content', [])
        text = '\n'.join(str(b.get('text', '')) for b in blocks if isinstance(b, dict) and b.get('type') == 'text')
        finish = response.get('stop_reason', '')
    else:
        response = api(config, '/chat/completions', {'model': config['model'],
            'messages': [{'role': 'system', 'content': system}, {'role': 'user', 'content': prompt}],
            'stream': False, 'max_tokens': 64 if small else 16000})
        choices = response.get('choices', [])
        if not choices or not isinstance(choices[0], dict):
            raise Failure('模型没有返回回答，请检查模型与协议')
        text = choices[0].get('message', {}).get('content', '')
        if isinstance(text, list):
            text = '\n'.join(x.get('text', '') for x in text if isinstance(x, dict))
        finish = choices[0].get('finish_reason', '')
    if not isinstance(text, str) or not text.strip():
        raise Failure('模型没有返回可用内容')
    if finish in ('length', 'max_tokens'):
        raise Failure('模型输出被截断，请缩小任务或选择更长输出的模型')
    if config.get('key'): text=text.replace(config['key'],'[credential-redacted]')
    usage = response.get('usage', {})
    return text, {k: v for k, v in usage.items() if k in ('prompt_tokens', 'completion_tokens', 'input_tokens', 'output_tokens') and isinstance(v, int)}

SYSTEM = '''You build usable offline HTML mini-apps and games for StepX Companion, desktop and Android WebView.
Return exactly TWO fenced blocks and no surrounding prose. First an ```html block containing the complete standalone <!DOCTYPE html> document, then a ```json block containing {"name":"short name","checks":[...]}. Do not put HTML inside a JSON string: output raw HTML in its own block. Keep code concise.
No external resources, npm, network, navigation, forms submitting to servers, permissions or eval. All CSS/JS inline, images inline SVG/data URLs. Produce a polished, coherent visual design with readable type, clear spacing, touch targets, useful empty states and keyboard support. Use semantic accessible controls with stable IDs. Include a viewport meta tag; support 390px and 1280px widths. Render user text with textContent, never inject input into innerHTML. Do not leave TODO, mock buttons or claim external data.
Implement the user's requested workflow fully within this offline scope. State unavailable external features in the app.
checks is 3 to 24 declarative real browser steps: {"label":"purpose","action":"click|type|key|reload|wait|assert_text|assert_value|assert_count|assert_storage|assert_canvas","selector":"CSS selector","value":"input/key/storage key","expected":"expected string or number","ms":300}.
Click/type use actual browser input; key is Enter/Tab/Space/Escape/ArrowLeft/ArrowRight/ArrowUp/ArrowDown/Backspace. reload refreshes the page in the same origin, allowing verification of localStorage persistence; wait maximum 1500ms. assert_text requires nonempty visible text, assert_value requires form value in expected (value alias is accepted), assert_count exact selector match count, assert_storage localStorage key/value, assert_canvas ensures canvas contains visible nonuniform pixels.
Include at least one interaction (click/type/key), at least two assertions, and assertions meaningfully testing the requested functionality (e.g. add a task and check it appears; game starts, score or state changes). Do not include arbitrary JS in checks. No passed field: independent harness computes success. Browser also checks mobile layout, visible rendered content, and uncaught JavaScript errors.
'''

def _extract_html_title(html):
    for pattern in [r'<title[^>]*>([\s\S]*?)</title>', r'<h1[^>]*>([\s\S]*?)</h1>']:
        match = re.search(pattern, html, re.I)
        if match:
            cleaned = re.sub(r'<[^>]+>', '', match.group(1)).strip()
            if cleaned:
                return cleaned[:80]
    return '可交互单文件作品'

def _default_checks():
    return [
        {'label': '页面焦点交互', 'action': 'key', 'value': 'Tab'},
        {'label': '文档根节点检查', 'action': 'assert_count', 'selector': 'html', 'expected': 1},
        {'label': '页面主体渲染检查', 'action': 'assert_count', 'selector': 'body', 'expected': 1},
    ]

def parse_program(text):
    raw = (text or '').strip()
    if not raw:
        raise Failure('需要完整 HTML 代码块及 name/checks JSON 代码块，不能包含未完成的代码')
    html_block = re.search(r'```html[ \t]*\r?\n([\s\S]*?)\r?\n```', raw, re.I)
    json_block = re.search(r'```json[ \t]*\r?\n([\s\S]*?)\r?\n```', raw, re.I)
    value = None
    from_fenced_or_raw_html = False
    if html_block is not None:
        from_fenced_or_raw_html = True
        html_code = html_block.group(1).strip()
        meta = None
        if json_block is not None:
            blocks = sorted([html_block, json_block], key=lambda b: b.start())
            remaining = raw[:blocks[0].start()] + raw[blocks[0].end():blocks[1].start()] + raw[blocks[1].end():]
            if remaining.strip():
                raise Failure('需要完整 HTML 代码块及 name/checks JSON 代码块，不能包含未完成的代码')
            try:
                parsed = json.loads(json_block.group(1))
                if isinstance(parsed, dict):
                    meta = parsed
            except ValueError:
                meta = None
        if meta is None:
            title = _extract_html_title(html_code)
            meta = {
                'name': title,
                'title': title,
                'summary': '已生成可交互单文件作品',
                'checks': _default_checks(),
            }
        meta['html'] = html_code
        value = meta
    else:
        # Try JSON-only representation first (used by structured callers/tests).
        candidate = raw
        if candidate.startswith('```'):
            candidate = re.sub(r'^```(?:json)?\s*', '', candidate, flags=re.I)
            candidate = re.sub(r'\s*```$', '', candidate)
        try:
            parsed = json.loads(candidate)
            if isinstance(parsed, dict):
                value = parsed
        except ValueError:
            value = None
        if value is None:
            raw_html = re.search(r'(<!DOCTYPE\s+html[\s\S]*?</html>|<html\b[\s\S]*?</html>)', raw, re.I)
            if raw_html is not None:
                from_fenced_or_raw_html = True
                html_code = raw_html.group(1).strip()
                title = _extract_html_title(html_code)
                value = {
                    'name': title,
                    'title': title,
                    'summary': '已生成可交互单文件作品',
                    'html': html_code,
                    'checks': _default_checks(),
                }
            else:
                raise Failure('需要完整 HTML 代码块及 name/checks JSON 代码块，不能包含未完成的代码')
    if not isinstance(value, dict):
        raise Failure('生成结果结构无效')
    html = value.get('html')
    checks = value.get('checks')
    if not isinstance(html, str) or len(html.encode()) > MAX_HTML or not re.search(r'<html\b', html, re.I) or not re.search(r'<body\b', html, re.I):
        raise Failure('需要不超过 2 MiB 的完整 HTML 文档')
    try:
        validate_checks(checks)
    except Failure:
        if from_fenced_or_raw_html:
            checks = _default_checks()
            validate_checks(checks)
        else:
            raise
    extracted_title = _extract_html_title(html)
    name = str(value.get('name') or value.get('title') or '').strip()[:80] or extracted_title or '未命名作品'
    summary = str(value.get('summary') or '已生成可交互单文件作品').strip()[:160]
    return {'name': name, 'title': name, 'summary': summary, 'html': html, 'checks': checks}

def validate_checks(checks):
    allowed = {'click','type','key','reload','wait','assert_text','assert_value','assert_count','assert_storage','assert_canvas'}
    if not isinstance(checks, list) or not 3 <= len(checks) <= 24:
        raise Failure('需要 3–24 个可执行验收步骤')
    actions = assertions = 0
    for check in checks:
        if not isinstance(check, dict) or check.get('action') not in allowed:
            raise Failure('验收步骤包含不支持的操作')
        action = check['action']
        # Common compatible spelling: assert_value.value is the expected value,
        # whereas type.value is the entered text. Normalize before freezing tests.
        if action == 'assert_value' and 'expected' not in check and 'value' in check:
            check['expected'] = check['value']
        if action in ('assert_value','assert_storage') and 'expected' not in check:
            raise Failure('值断言缺少预期值')
        for key in ['label', 'selector', 'value', 'expected']:
            if key in check and (not isinstance(check[key], (str, int, float)) or isinstance(check[key], bool) or len(str(check[key])) > 1000):
                raise Failure('验收步骤参数无效或过长')
        if action not in ('wait','key','reload','assert_storage') and not str(check.get('selector','')).strip():
            raise Failure('验收步骤缺少 CSS 选择器')
        if action == 'wait' and (not isinstance(check.get('ms', 300), (int,float)) or not 0 <= check.get('ms',300) <= 1500):
            raise Failure('验收等待不得超过 1500ms')
        if action == 'assert_text' and not str(check.get('expected','')).strip():
            raise Failure('文字断言不能使用空字符串')
        if action == 'assert_count' and (not isinstance(check.get('expected'), int) or check['expected'] < 0):
            raise Failure('数量断言需要非负整数')
        if action in ('click','type','key'):
            actions += 1
        if action.startswith('assert_'):
            assertions += 1
    if actions < 1 or assertions < 2:
        raise Failure('验收至少包含一次交互及两次结果断言')

class Browser:
    """Ephemeral CDP pipe; no TCP debug port, cookie import, or user browser profile."""
    def __init__(self, executable, headless=True):
        self.pending, self.serial, self.session = {}, 0, None
        self.lock = threading.RLock()
        self.errors, self.blocked = [], []
        self.origin = None
        self.profile = tempfile.mkdtemp(prefix='stepx-lab-')
        if os.name != 'nt':
            os.chmod(self.profile, 0o700)
        incoming, self.writer = os.pipe()
        self.reader, outgoing = os.pipe()
        args = [executable, '--user-data-dir='+self.profile, '--remote-debugging-pipe', '--no-first-run',
                '--no-default-browser-check', '--disable-sync', '--disable-background-networking',
                '--disable-component-update', '--disable-extensions', '--disable-features=Translate',
                '--window-size=1280,900', 'about:blank']
        if headless:
            args.insert(1, '--headless=new')
        kwargs = dict(stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            if os.name == 'nt':
                import msvcrt
                read_handle, write_handle = msvcrt.get_osfhandle(incoming), msvcrt.get_osfhandle(outgoing)
                os.set_handle_inheritable(read_handle, True); os.set_handle_inheritable(write_handle, True)
                startup = subprocess.STARTUPINFO()
                startup.lpAttributeList = {'handle_list': [read_handle, write_handle]}
                args.insert(1, '--remote-debugging-io-pipes=%d,%d' % (read_handle, write_handle))
                kwargs.update(startupinfo=startup, close_fds=True, creationflags=subprocess.CREATE_NO_WINDOW)
                self.proc = subprocess.Popen(args, **kwargs)
                os.set_handle_inheritable(read_handle, False); os.set_handle_inheritable(write_handle, False)
            else:
                wrapper = 'import os,sys; a=int(sys.argv[1]); b=int(sys.argv[2]); x=os.dup(a); y=os.dup(b); os.dup2(x,3); os.dup2(y,4); os.set_inheritable(3,True); os.set_inheritable(4,True); [os.close(fd) for fd in {a,b,x,y}-{3,4}]; os.execv(sys.argv[3],sys.argv[3:])'
                self.proc = subprocess.Popen([sys.executable,'-c',wrapper,str(incoming),str(outgoing),*args], pass_fds=(incoming,outgoing), **kwargs)
            os.close(incoming); os.close(outgoing)
            threading.Thread(target=self.read, daemon=True).start()
            # A cold browser start on a newly installed machine can exceed 12 seconds.
            self.call('Browser.getVersion', timeout=30)
            target = self.call('Target.createTarget', {'url':'about:blank'})['targetId']
            self.session = self.call('Target.attachToTarget', {'targetId':target,'flatten':True})['sessionId']
            for method in ['Page.enable', 'Runtime.enable', 'Log.enable', 'Network.enable']:
                self.call(method, session=self.session)
            self.call('Fetch.enable', {'patterns':[{'urlPattern':'*'}]}, self.session)
            self.call('Browser.setDownloadBehavior', {'behavior':'deny'})
            # Built-in page errors survive accidental model replacement of console handlers.
            self.call('Page.addScriptToEvaluateOnNewDocument', {'source':"window.addEventListener('error',e=>console.error('STEPX_UNCAUGHT:'+e.message));window.addEventListener('unhandledrejection',e=>console.error('STEPX_UNHANDLED:'+String(e.reason)));"}, self.session)
        except Exception as error:
            self.close()
            detail = str(error) if isinstance(error, Failure) else type(error).__name__
            raise Failure('无法启动隔离浏览器验收，请检查 Chrome 或 Edge 安装（'+detail+'）') from None
    def send(self, method, params=None, session=None, expect=True):
        with self.lock:
            self.serial += 1
            ident = self.serial
            waiter = queue.Queue() if expect else None
            if waiter is not None:
                self.pending[ident] = waiter
            message = {'id':ident, 'method':method, 'params':params or {}}
            if session:
                message['sessionId'] = session
            data = json.dumps(message, separators=(',',':')).encode()+b'\0'
            while data:
                count = os.write(self.writer, data)
                data = data[count:]
        return ident, waiter
    def read(self):
        buffer = b''
        try:
            while True:
                block = os.read(self.reader, 65536)
                if not block: break
                buffer += block
                if len(buffer) > 16*1024*1024: break
                while b'\0' in buffer:
                    raw, buffer = buffer.split(b'\0',1)
                    message = json.loads(raw)
                    waiter = self.pending.get(message.get('id'))
                    if waiter is not None:
                        waiter.put(message)
                    elif message.get('method') == 'Fetch.requestPaused':
                        params = message['params']; address = params['request']['url']
                        allowed = self.origin and address == self.origin+'/'
                        if allowed:
                            self.send('Fetch.continueRequest', {'requestId':params['requestId']}, message['sessionId'], False)
                        else:
                            self.blocked.append(urllib.parse.urlsplit(address).scheme or 'unknown')
                            self.send('Fetch.failRequest', {'requestId':params['requestId'],'errorReason':'BlockedByClient'}, message['sessionId'], False)
                    elif message.get('method') == 'Log.entryAdded':
                        entry = message['params'].get('entry', {})
                        if entry.get('level') == 'error' and entry.get('source') in ('security', 'network'):
                            self.errors.append(str(entry.get('text', 'Resource blocked'))[:1000])
                            self.blocked.append('resource')
                    elif message.get('method') == 'Runtime.exceptionThrown':
                        details = message['params']['exceptionDetails']
                        self.errors.append(str(details.get('exception',{}).get('description', details.get('text','JavaScript error')))[:1000])
                    elif message.get('method') == 'Runtime.consoleAPICalled' and message['params'].get('type') == 'error':
                        values = [str(x.get('value', '')) for x in message['params'].get('args',[])]
                        self.errors.append(' '.join(values)[:1000])
        except Exception:
            pass
        for waiter in list(self.pending.values()):
            waiter.put({'error':{}})
    def call(self, method, params=None, session=None, timeout=12):
        ident, waiter = self.send(method, params, session)
        try:
            result = waiter.get(timeout=timeout)
            if 'error' in result:
                raise Failure('浏览器操作未完成：'+method)
            return result.get('result',{})
        except queue.Empty:
            raise Failure('浏览器无响应：'+method) from None
        finally:
            self.pending.pop(ident, None)
    def evaluate(self, expression):
        value = self.call('Runtime.evaluate', {'expression':expression, 'returnByValue':True,'awaitPromise':True}, self.session)
        if 'exceptionDetails' in value:
            raise Failure('页面验收脚本未完成')
        return value.get('result',{}).get('value')
    def viewport(self, width, height):
        self.call('Emulation.setDeviceMetricsOverride', {'width':width,'height':height,'deviceScaleFactor':1,'mobile':width<600}, self.session)
    def navigate(self, origin):
        self.origin = origin
        self.call('Page.navigate', {'url':origin+'/'}, self.session)
        for _ in range(80):
            if self.evaluate('document.readyState') == 'complete' and self.evaluate('location.href') == origin+'/':
                time.sleep(.3)
                return
            time.sleep(.1)
        raise Failure('页面载入超时')
    def close(self):
        if getattr(self, 'proc', None) is not None:
            with contextlib.suppress(Exception):
                self.call('Browser.close',timeout=1)
            with contextlib.suppress(Exception):
                self.proc.terminate(); self.proc.wait(timeout=2)
            with contextlib.suppress(Exception):
                if self.proc.poll() is None:
                    self.proc.kill(); self.proc.wait(timeout=2)
        for field in ['reader','writer']:
            with contextlib.suppress(Exception): os.close(getattr(self,field))
        shutil.rmtree(self.profile,ignore_errors=True)

@contextlib.contextmanager
def serving(html):
    content = html.encode('utf-8')
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args): pass
        def do_GET(self):
            if self.path != '/':
                self.send_error(404); return
            self.send_response(200)
            self.send_header('Content-Type','text/html; charset=utf-8')
            self.send_header('Content-Security-Policy',CSP)
            self.send_header('Cache-Control','no-store')
            self.send_header('X-Content-Type-Options','nosniff')
            self.send_header('Content-Length',str(len(content)))
            self.end_headers(); self.wfile.write(content)
    server = http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
    threading.Thread(target=server.serve_forever,daemon=True).start()
    try: yield 'http://127.0.0.1:'+str(server.server_port)
    finally: server.shutdown(); server.server_close()

def check_step(browser, check):
    action = check['action']
    selector = json.dumps(str(check.get('selector','')))
    expected = json.dumps(check.get('expected',''))
    value = json.dumps(str(check.get('value','')))
    element = 'document.querySelector(%s)' % selector
    if action == 'reload':
        browser.call('Page.reload', {'ignoreCache':True}, browser.session)
        time.sleep(.35)
        for _ in range(50):
            if browser.evaluate('document.readyState') == 'complete': return True
            time.sleep(.1)
        return False
    if action == 'wait':
        time.sleep(check.get('ms',300)/1000); return True
    if action == 'click':
        bounds = browser.evaluate('(()=>{const e=%s;if(!e||e.disabled)return null;e.scrollIntoView({block:"center"});const r=e.getBoundingClientRect();return r.width>0&&r.height>0?{x:r.x+r.width/2,y:r.y+r.height/2}:null})()' % element)
        if not bounds: return False
        for kind in ['mousePressed','mouseReleased']:
            browser.call('Input.dispatchMouseEvent',dict(type=kind,button='left',clickCount=1,**bounds),browser.session)
        time.sleep(.1); return True
    if action == 'type':
        ok = browser.evaluate('(()=>{const e=%s;if(!e||e.disabled||e.readOnly||!(e instanceof HTMLInputElement||e instanceof HTMLTextAreaElement))return false;e.focus();e.select();return true})()' % element)
        if not ok: return False
        browser.call('Input.insertText',{'text':str(check.get('value',''))},browser.session)
        return True
    if action == 'key':
        key = str(check.get('value',''))
        codes = {'Enter':13,'Tab':9,'Space':32,'Escape':27,'ArrowLeft':37,'ArrowUp':38,'ArrowRight':39,'ArrowDown':40,'Backspace':8}
        if key not in codes: raise Failure('验收使用了不支持的按键')
        for kind in ['keyDown','keyUp']:
            browser.call('Input.dispatchKeyEvent',{'type':kind,'key':' ' if key=='Space' else key,'code':key,'windowsVirtualKeyCode':codes[key]},browser.session)
        time.sleep(.1); return True
    if action == 'assert_count':
        return browser.evaluate('document.querySelectorAll(%s).length===%s' % (selector,expected)) is True
    if action == 'assert_storage':
        return browser.evaluate('localStorage.getItem(%s)===String(%s)' % (value,expected)) is True
    if action == 'assert_text':
        return browser.evaluate('(()=>{const e=%s;return !!e&&e.getBoundingClientRect().width>0&&e.getBoundingClientRect().height>0&&e.innerText.includes(String(%s))})()' % (element,expected)) is True
    if action == 'assert_value':
        return browser.evaluate('(()=>{const e=%s;return !!e&&e.value===String(%s)})()' % (element,expected)) is True
    if action == 'assert_canvas':
        return browser.evaluate('(()=>{const e=%s;if(!(e instanceof HTMLCanvasElement)||!e.width||!e.height)return false;const c=document.createElement("canvas");c.width=c.height=32;const x=c.getContext("2d");x.drawImage(e,0,0,32,32);const a=x.getImageData(0,0,32,32).data,s=new Set();for(let i=0;i<a.length;i+=4)s.add(a[i]+","+a[i+1]+","+a[i+2]+","+a[i+3]);return s.size>2})()' % element) is True
    return False

def verify(executable, program, output):
    validate_checks(program['checks'])
    records = []
    browser = Browser(executable)
    try:
        with serving(program['html']) as origin:
            browser.viewport(1280,900); browser.navigate(origin)
            for width,height,label in [(1280,900,'桌面布局'),(390,844,'手机布局')]:
                browser.viewport(width,height); time.sleep(.25)
                state = browser.evaluate('({visible:!!document.body&&document.body.getBoundingClientRect().height>20&&(document.body.innerText.trim().length>8||!!document.querySelector("canvas,svg")),width:document.documentElement.scrollWidth,viewport:innerWidth})')
                passed = bool(state and state.get('visible') and state['width'] <= state['viewport']+4 and state['viewport'] <= width+4)
                records.append(dict(label=label,passed=passed,detail='可见内容与横向溢出检查'))
            for index,check in enumerate(program['checks']):
                try:
                    passed = check_step(browser,check)
                    detail = check['action'] + (' · '+str(check.get('selector','')) if check.get('selector') else '')
                    if not passed and 'expected' in check:
                        detail += ' · 预期 '+str(check['expected'])[:120]
                except Failure as error:
                    passed,detail = False,str(error)
                records.append(dict(label=str(check.get('label') or '交互验收 %d'%(index+1)),passed=passed,detail=detail))
            # The preview is the first-use screen, not the synthetic records used
            # during acceptance. Keep all assertions above before clearing data.
            browser.call('Storage.clearDataForOrigin', {'origin':origin,'storageTypes':'all'}, browser.session)
            browser.navigate(origin)
            time.sleep(.4)
            browser.viewport(390,844)
            data = browser.call('Page.captureScreenshot',{'format':'png','captureBeyondViewport':False},browser.session)['data']
            private_write(output/'preview.png',base64.b64decode(data))
            records.append(dict(label='JavaScript 运行错误',passed=not browser.errors,detail='\n'.join(browser.errors[:5]) or '未发现未捕获错误'))
            records.append(dict(label='离线运行',passed=not browser.blocked,detail='无外部资源请求' if not browser.blocked else '发现并阻止了外部资源请求'))
    finally:
        browser.close()
    return {'passed':all(x['passed'] for x in records),'html_sha256':hashlib.sha256(program['html'].encode('utf-8')).hexdigest(),'checks':records,'limitations':'自动检查覆盖渲染、手机布局与所列交互；不等于所有用户场景、内容准确性或设备兼容性均已验收。'}

def generate(request):
    root = Path(request['root'])
    config = request['config']
    prompt = request['prompt']
    source = request.get('source','')
    if source:
        prompt += '\n\nImprove this existing version, retaining working behavior:\n'+source
    acceptance = None
    for round_no in range(1,4):
        emit('status',message='第 %d / 3 轮：%s'%(round_no,'生成作品' if round_no==1 else '重试生成或根据验收反馈修复'))
        output = root/('round-%d'%round_no); output.mkdir(mode=0o700)
        text=''
        try:
            text,usage = complete(config,SYSTEM,prompt)
            program = parse_program(text)
            if acceptance is None:
                acceptance = program['checks']
            else:
                # Repairs must meet the original observable contract, not rewrite
                # their own expected result to conceal a failed interaction.
                program['checks'] = acceptance
        except Failure as error:
            if text:
                private_write(output/'model-output.txt',text)
            report = {'passed':False,'checks':[{'label':'生成结果','passed':False,'detail':str(error)}]}
            private_write(output/'report.json',json.dumps(report,ensure_ascii=False,indent=2))
            emit('status',message=str(error))
            if round_no == 3: raise
            prompt += '\n\nPrevious generation did not pass: '+str(error)+'. Return exactly two complete fenced blocks: raw HTML, then name/checks JSON. Do not escape HTML into JSON.'
            if text: prompt += '\n\nPrevious invalid response for repair:\n'+text[:MAX_HTML]
            continue
        private_write(output/'index.html',program['html'])
        private_write(output/'checks.json',json.dumps(program['checks'],ensure_ascii=False,indent=2))
        emit('status',message='第 %d 轮：在隔离浏览器中验证页面、手机布局和真实交互'%round_no)
        try:
            report = verify(request['browser'],program,output)
        except Failure as error:
            report = {'passed':False,'html_sha256':hashlib.sha256(program['html'].encode('utf-8')).hexdigest(),'checks':[{'label':'浏览器验收','passed':False,'detail':str(error)}], 'limitations':'浏览器验收未完成，仍可在本地预览或发送到手机体验。'}
        report['usage'] = usage
        private_write(output/'report.json',json.dumps(report,ensure_ascii=False,indent=2))
        passed = report['passed']
        total = len(report['checks']); count = sum(1 for c in report['checks'] if c['passed'])
        artifact = {'id':root.name+'-'+str(round_no),'name':program['name'],'root':str(output),
                    'html_path':str(output/'index.html'),'screenshot_path':str(output/'preview.png') if (output/'preview.png').exists() else None,
                    'report_path':str(output/'report.json'),'passed':passed,'round':round_no,
                    'summary':'%d / %d 项自动检查通过'%(count,total),'created_at':int(time.time())}
        private_write(output/'artifact.json',json.dumps(artifact,ensure_ascii=False,indent=2))
        emit('artifact',artifact=artifact)
        if passed:
            emit('complete',artifact=artifact); return
        prompt = request['prompt']+'\n\nRepair this full HTML and its meaningful acceptance checks. Do not remove functionality or weaken assertions to hide bugs. Return the two fenced HTML and metadata JSON blocks.\n'+program['html']+'\n\nFrozen acceptance contract: keep these selectors and behaviors; do not weaken assertions:\n'+json.dumps(acceptance,ensure_ascii=False)+'\n\nActual independent browser feedback:\n'+json.dumps(report,ensure_ascii=False)
    emit('failed',message='已达到 3 轮上限；作品已保留，仍有未通过的验收项，可调整需求后继续修复。')

def main():
    request = json.loads(sys.stdin.readline(12*1024*1024))
    operation = request.get('op')
    if operation == 'models':
        result = api(request['config'],'/models')
        models = []
        for item in result.get('data',[])[:500]:
            if isinstance(item,dict) and isinstance(item.get('id'),str) and 0 < len(item['id']) <= 256:
                models.append(item['id'])
        emit('result',models=sorted(set(models)))
    elif operation == 'test':
        response,usage = complete(request['config'],'You are an API connectivity probe.','Reply with exactly STUDIO_OK.',True)
        emit('result',ok=bool(response.strip()),model=request['config']['model'],usage=usage)
    elif operation == 'generate':
        generate(request)
    elif operation == 'verify':
        program = {'html':Path(request['html']).read_text(encoding='utf-8'),'checks':request['checks']}
        validate_checks(program['checks'])
        result = verify(request['browser'],program,Path(request['output']))
        emit('result',report=result)
    elif operation == 'preview':
        html = Path(request['html']).read_text(encoding='utf-8')
        browser = Browser(request['browser'],headless=False)
        try:
            with serving(html) as origin:
                browser.viewport(390,844); browser.navigate(origin)
                browser.call('Page.bringToFront',session=browser.session)
                emit('result',ok=True)
                while browser.proc.poll() is None:
                    targets = browser.call('Target.getTargets').get('targetInfos',[])
                    if not any(t.get('url')==origin+'/' for t in targets): break
                    time.sleep(.5)
        finally: browser.close()
    else:
        raise Failure('不支持的工作台操作')

if __name__ == '__main__':
    def stop(*_): raise KeyboardInterrupt()
    signal.signal(signal.SIGTERM,stop)
    try: main()
    except KeyboardInterrupt: emit('cancelled')
    except Failure as error: emit('failed',message=str(error))
    except Exception: emit('failed',message='本地工作台操作未完成，请检查运行环境后重试。')
