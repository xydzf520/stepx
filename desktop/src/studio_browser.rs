//! A Studio-only browser owned by Companion. No default-browser profile is read.
//! The phone's explicitly requested handoff travels in memory over inherited pipes.
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Stdio},
    sync::{mpsc, Mutex, OnceLock},
    time::Duration,
};

const ORIGINS: [&str; 3] = [
    "https://studio.stepx.com/",
    "https://studio-api.stepx.com/",
    "https://login-studio.stepx.com/",
];

#[derive(Clone, Debug, Deserialize)]
pub struct BrowserReceipt {
    pub url: String,
    pub scope: String,
    pub browser: String,
}

struct Worker {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Value>,
    profile_root: Option<tempfile::TempDir>,
}
static WORKER: OnceLock<Mutex<Option<Worker>>> = OnceLock::new();

/// Locate an installed browser executable; never inspect its profile or account.
pub fn browser_path() -> Result<PathBuf> {
    let mut paths = Vec::new();
    if let Some(explicit) = std::env::var_os("STEPX_STUDIO_BROWSER") {
        paths.push(PathBuf::from(explicit));
    }
    #[cfg(target_os = "macos")]
    for app in ["Google Chrome", "Microsoft Edge", "Chromium"] {
        paths.push(PathBuf::from(format!(
            "/Applications/{app}.app/Contents/MacOS/{app}"
        )));
        if let Some(home) = crate::resources::user_home() {
            paths.push(
                PathBuf::from(home).join(format!("Applications/{app}.app/Contents/MacOS/{app}")),
            );
        }
    }
    #[cfg(windows)]
    for key in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(key) {
            for suffix in [
                "Google/Chrome/Application/chrome.exe",
                "Microsoft/Edge/Application/msedge.exe",
                "Chromium/Application/chrome.exe",
            ] {
                paths.push(PathBuf::from(&base).join(suffix));
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            for name in if cfg!(windows) {
                &["chrome.exe", "msedge.exe", "chromium.exe"][..]
            } else {
                &[
                    "google-chrome",
                    "google-chrome-stable",
                    "chromium",
                    "chromium-browser",
                    "microsoft-edge",
                    "microsoft-edge-stable",
                    "brave-browser",
                    "brave",
                ][..]
            } {
                paths.push(directory.join(name));
            }
        }
    }
    #[cfg(target_os = "linux")]
    for candidate in [
        "/opt/google/chrome/google-chrome",
        "/opt/google/chrome/chrome",
        "/opt/microsoft/msedge/msedge",
        "/opt/brave.com/brave/brave",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/microsoft-edge-stable",
        "/usr/bin/microsoft-edge",
        "/snap/bin/chromium",
    ] {
        paths.push(PathBuf::from(candidate));
    }
    for path in paths {
        if path.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if std::fs::metadata(&path)?.permissions().mode() & 0o111 == 0 {
                    continue;
                }
            }
            return Ok(path);
        }
    }
    bail!("请先安装 Google Chrome 或 Microsoft Edge，再点击继续编辑；Studio 使用独立会话，不会读取您的浏览器账号")
}

fn validated_request(url: &str, handoff: &Value) -> Result<Value> {
    let (canonical, kind) = crate::studio::parse_link(url)?;
    ensure!(
        kind == crate::studio::LinkKind::Project,
        "请从官网项目的“继续编辑”进入构建器"
    );
    let scope = handoff["scope"]
        .as_str()
        .context("手机未提供官网账号确认")?;
    ensure!(
        scope.len() == 64 && scope.bytes().all(|b| b.is_ascii_hexdigit()),
        "官网账号确认格式无效"
    );
    let cookies = handoff["cookies"]
        .as_array()
        .context("手机未提供官网会话，请重新连接官网登录")?;
    ensure!(
        !cookies.is_empty() && cookies.len() <= 64,
        "官网会话为空或过大，请重新登录"
    );
    let mut cleaned = Vec::with_capacity(cookies.len());
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0;
    for cookie in cookies {
        let origin = cookie["url"].as_str().context("官网会话来源无效")?;
        let name = cookie["name"].as_str().context("官网会话格式无效")?;
        let value = cookie["value"].as_str().context("官网会话格式无效")?;
        ensure!(ORIGINS.contains(&origin), "官网会话包含未授权来源");
        ensure!(
            !name.is_empty()
                && name.len() <= 256
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)),
            "官网会话名称无效"
        );
        // CookieManager returns Cookie-header syntax; RFC 6265 also allows a
        // value wrapped in one pair of DQUOTE characters. Preserve that pair
        // for the server, while rejecting delimiters/control bytes inside it.
        let payload = value
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(value);
        ensure!(
            value.len() <= 16384
                && payload
                    .bytes()
                    .all(|b| (0x21..=0x7e).contains(&b) && !b";\"\\".contains(&b)),
            "官网会话内容无效"
        );
        ensure!(
            seen.insert((origin.to_owned(), name.to_owned())),
            "官网会话存在重复项"
        );
        bytes += name.len() + value.len();
        ensure!(bytes <= 65536, "官网会话过大");
        cleaned.push(json!({"url":origin,"name":name,"value":value}));
    }
    Ok(json!({"op":"open","url":canonical,"scope":scope.to_ascii_lowercase(),"cookies":cleaned}))
}

impl Worker {
    fn spawn() -> Result<Self> {
        let executable = browser_path()?;
        let profile_root = tempfile::Builder::new()
            .prefix("stepx-studio-session-")
            .tempdir()?;
        let mut command = crate::resources::python_command();
        command
            .arg("-u")
            .arg("-c")
            .arg(BROWSER_WORKER)
            .arg(executable)
            .arg(profile_root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .context("无法启动 Studio 工作窗口，请检查完整安装包中的 Python 运行环境")?;
        let input = child.stdin.take().context("无法建立 Studio 控制通道")?;
        let stdout = child.stdout.take().context("无法建立 Studio 状态通道")?;
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                // Worker stdout contains only compact receipts, never browser output.
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() > 32768 => break,
                    Ok(_) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&line) {
                            if tx.send(value).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });
        Ok(Self {
            child,
            input,
            output,
            profile_root: Some(profile_root),
        })
    }
    fn request(&mut self, request: &Value) -> Result<Value> {
        serde_json::to_writer(&mut self.input, request).context("Studio 工作窗口已关闭，请重试")?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        self.output
            .recv_timeout(Duration::from_secs(75))
            .context("Studio 官网验证超时，请检查网络后重试")
    }
    fn shutdown(&mut self) {
        let _ = self.input.write_all(b"{\"op\":\"close\"}\n");
        let _ = self.input.flush();
        // The helper owns profile cleanup and closes only its own browser process.
        let mut exited = false;
        for _ in 0..70 {
            if self.child.try_wait().ok().flatten().is_some() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !exited {
            let _ = self.child.kill();
            let _ = self.child.wait();
            // Chromium also closes on a CDP-pipe disconnect. Give its storage
            // processes time to release the disposable profile before cleanup.
            std::thread::sleep(Duration::from_millis(700));
        }
        if let Some(profile) = self.profile_root.take() {
            let path = profile.path().to_path_buf();
            if profile.close().is_err() {
                for _ in 0..10 {
                    if std::fs::remove_dir_all(&path).is_ok() || !path.exists() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Import the companion's own authenticated session into our isolated window.
/// Success is reported only after the server account, project ownership and route agree.
pub fn open_editor(url: &str, handoff: &Value) -> Result<BrowserReceipt> {
    let request = validated_request(url, handoff)?;
    let mut slot = WORKER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| anyhow::anyhow!("Studio 工作窗口状态异常"))?;
    if slot
        .as_mut()
        .is_some_and(|w| w.child.try_wait().ok().flatten().is_some())
    {
        *slot = None;
    }
    if slot.is_none() {
        *slot = Some(Worker::spawn()?);
    }
    let result = match slot.as_mut().unwrap().request(&request) {
        Ok(result) => result,
        Err(error) => {
            if let Some(mut worker) = slot.take() {
                worker.shutdown();
            }
            return Err(error);
        }
    };
    ensure!(
        result["ok"] == true,
        "{}",
        match result["code"].as_str().unwrap_or("") {
            "login" => "手机 Studio 官网登录已过期，请在伴侣中重新登录",
            "account" => "电脑与手机的 Studio 账号未通过一致性核对，已关闭工作窗口",
            "project" => "当前账号中没有此项目，请刷新官网项目列表",
            "route" => "官网没有进入指定编辑页面，请重试",
            "browser" => "无法打开 Studio 工作窗口，请检查 Chrome 或 Edge 是否可正常启动",
            _ => "Studio 官网连接未完成，请检查网络后重试",
        }
    );
    let receipt: BrowserReceipt =
        serde_json::from_value(result).context("Studio 工作窗口回执无效")?;
    ensure!(
        receipt.url == request["url"] && receipt.scope == request["scope"],
        "Studio 工作窗口核对结果不一致"
    );
    Ok(receipt)
}

/// Logout/device switch must close this browser so another account cannot inherit it.
pub fn close_session() {
    if let Some(lock) = WORKER.get() {
        if let Ok(mut slot) = lock.lock() {
            if let Some(mut worker) = slot.take() {
                worker.shutdown();
            }
        }
    }
}

// Python's stdlib provides Windows inheritable HANDLE lists and POSIX fd remapping
// without adding a browser automation runtime to every release. This code controls
// only the browser process it creates; it never enumerates user/browser profiles.
const BROWSER_WORKER: &str = r#"
import os, sys, json, subprocess, tempfile, shutil, threading, queue, time, hashlib
from urllib.parse import urlsplit

class Failure(Exception):
    def __init__(self, code): self.code=code

class Browser:
    def __init__(self, executable, scope):
        self.scope=scope; self.pending={}; self.serial=0; self.target=None; self.session=None
        self.profile=tempfile.mkdtemp(prefix='profile-',dir=sys.argv[2])
        if os.name!='nt': os.chmod(self.profile,0o700)
        incoming, self.writer=os.pipe(); self.reader, outgoing=os.pipe()
        args=[executable,'--user-data-dir='+self.profile,'--remote-debugging-pipe','--no-first-run','--no-default-browser-check','--disable-sync','--disable-background-networking','--new-window','about:blank']
        kwargs={'stdin':subprocess.DEVNULL,'stdout':subprocess.DEVNULL,'stderr':subprocess.DEVNULL}
        try:
            if os.name=='nt':
                import msvcrt
                read_handle=msvcrt.get_osfhandle(incoming); write_handle=msvcrt.get_osfhandle(outgoing)
                os.set_handle_inheritable(read_handle,True); os.set_handle_inheritable(write_handle,True)
                info=subprocess.STARTUPINFO(); info.lpAttributeList={'handle_list':[read_handle,write_handle]}
                args.insert(1,'--remote-debugging-io-pipes=%d,%d'%(read_handle,write_handle))
                kwargs.update(startupinfo=info,close_fds=True,creationflags=subprocess.CREATE_NO_WINDOW)
                self.proc=subprocess.Popen(args,**kwargs)
                os.set_handle_inheritable(read_handle,False); os.set_handle_inheritable(write_handle,False)
            else:
                # Do the remap in a tiny exec wrapper, avoiding preexec_fn in a
                # multithreaded process and preserving exactly the intended fds.
                wrapper='import os,sys; a=int(sys.argv[1]); b=int(sys.argv[2]); x=os.dup(a); y=os.dup(b); os.dup2(x,3); os.dup2(y,4); os.set_inheritable(3,True); os.set_inheritable(4,True); [os.close(fd) for fd in {a,b,x,y}-{3,4}]; os.execv(sys.argv[3],sys.argv[3:])'
                self.proc=subprocess.Popen([sys.executable,'-c',wrapper,str(incoming),str(outgoing),*args],pass_fds=(incoming,outgoing),**kwargs)
            os.close(incoming); os.close(outgoing)
            threading.Thread(target=self.read,daemon=True).start()
            self.call('Browser.getVersion',timeout=12)
        except Exception:
            self.close(); raise Failure('browser')
    def read(self):
        buffer=b''
        try:
            while True:
                block=os.read(self.reader,65536)
                if not block: break
                buffer+=block
                if len(buffer)>8*1024*1024: break
                while b'\0' in buffer:
                    message,buffer=buffer.split(b'\0',1)
                    value=json.loads(message)
                    waiter=self.pending.get(value.get('id'))
                    if waiter is not None: waiter.put(value)
        except Exception: pass
        for waiter in list(self.pending.values()): waiter.put({'error':{}})
    def call(self, method, params=None, session=None, timeout=12):
        self.serial+=1; ident=self.serial; waiter=queue.Queue(); self.pending[ident]=waiter
        message={'id':ident,'method':method,'params':params or {}}
        if session: message['sessionId']=session
        try:
            data=json.dumps(message,separators=(',',':')).encode()+b'\0'
            while data:
                count=os.write(self.writer,data); data=data[count:]
            answer=waiter.get(timeout=timeout)
            if 'error' in answer: raise Failure('connection')
            return answer.get('result',{})
        except Exception: raise Failure('connection')
        finally: self.pending.pop(ident,None)
    def evaluate(self, expression, timeout=12):
        result=self.call('Runtime.evaluate',{'expression':expression,'awaitPromise':True,'returnByValue':True},self.session,timeout)
        if 'exceptionDetails' in result: raise Failure('connection')
        return result.get('result',{}).get('value')
    def attach(self, url):
        targets=self.call('Target.getTargets').get('targetInfos',[])
        # Reopen the same editor without discarding unsent edits. Other project
        # editors stay in their own tabs under this same isolated browser session.
        target=next((t['targetId'] for t in targets if t['type']=='page' and t['url']==url),None)
        existing=target is not None
        if target is None:
            target=next((t['targetId'] for t in targets if t['type']=='page' and t['url']=='about:blank'),None)
        if target is None: target=self.call('Target.createTarget',{'url':'about:blank'})['targetId']
        if target!=self.target or self.session is None:
            self.target=target
            self.session=self.call('Target.attachToTarget',{'targetId':target,'flatten':True})['sessionId']
            self.call('Page.enable',session=self.session)
        self.call('Target.activateTarget',{'targetId':target})
        return existing
    def navigate(self,url):
        result=self.call('Page.navigate',{'url':url},self.session)
        if result.get('errorText'): raise Failure('connection')
        deadline=time.monotonic()+15
        while time.monotonic()<deadline:
            try:
                state=self.evaluate('({url:location.href,ready:document.readyState,text:document.body?document.body.innerText.length:0})',timeout=3)
                if state and state.get('url')==url and state.get('ready')=='complete' and state.get('text',0)>20: return
                if state and 'login-studio.stepx.com' in state.get('url',''): raise Failure('login')
            except Failure as e:
                if e.code=='login': raise
            time.sleep(.15)
        raise Failure('route')
    def open(self, request):
        existing=self.attach(request['url'])
        self.call('Storage.clearCookies')
        cookies=[]
        for item in request['cookies']:
            cookies.append({'name':item['name'],'value':item['value'],'url':item['url'],'path':'/','secure':True,'httpOnly':True,'sameSite':'Lax'})
        self.call('Storage.setCookies',{'cookies':cookies})
        if not existing: self.navigate('https://studio.stepx.com/projects')
        # Return only identity and the requested project's existence to the local
        # worker. Neither values nor the project library are printed or persisted.
        parts=urlsplit(request['url']).path.split('/'); kind=parts[2]; project=parts[3]
        expression='''(async()=>{const a=await fetch('/api/identity/me',{credentials:'include'});if(!a.ok)return {code:'login'};const me=await a.json();const b=await fetch('/api/projects',{credentials:'include'});if(!b.ok)return {code:'connection'};const p=await b.json();return {id:me.userId,found:(p.projects||[]).some(x=>x.id===%s&&x.builderType===%s)}})()'''%(json.dumps(project),json.dumps(kind))
        check=self.evaluate(expression)
        if not check or check.get('code'): raise Failure((check or {}).get('code','connection'))
        scope=hashlib.sha256(('studio-web\n'+str(check.get('id',''))).encode()).hexdigest()
        if scope!=request['scope']: raise Failure('account')
        if not check.get('found'): raise Failure('project')
        if not existing: self.navigate(request['url'])
        actual=self.evaluate('location.href')
        if actual!=request['url']: raise Failure('route')
        self.call('Page.bringToFront',session=self.session)
        return {'ok':True,'url':actual,'scope':scope,'browser':os.path.basename(sys.argv[1])}
    def close(self):
        try:
            if hasattr(self,'proc') and self.proc.poll() is None:
                try: self.call('Browser.close',timeout=1)
                except Exception: pass
                try: self.proc.wait(timeout=2)
                except Exception: self.proc.terminate(); self.proc.wait(timeout=2)
        except Exception:
            try: self.proc.kill()
            except Exception: pass
        for name in ['writer','reader']:
            try: os.close(getattr(self,name))
            except Exception: pass
        shutil.rmtree(self.profile,ignore_errors=True)

browser=None
try:
    while True:
        line=sys.stdin.buffer.readline(131073)
        if not line or len(line)>131072: break
        request=None
        try:
            request=json.loads(line)
            if request.get('op')=='close': break
            if request.get('op')!='open': raise Failure('request')
            if browser is not None and (browser.scope!=request['scope'] or browser.proc.poll() is not None):
                browser.close(); browser=None
            if browser is None: browser=Browser(sys.argv[1],request['scope'])
            answer=browser.open(request)
        except Failure as e:
            if browser is not None: browser.close(); browser=None
            answer={'ok':False,'code':e.code}
        except Exception:
            if browser is not None: browser.close(); browser=None
            answer={'ok':False,'code':'connection'}
        # Do not include exceptions, request input or browser logs in receipts.
        print(json.dumps(answer,separators=(',',':')),flush=True)
        request=None; line=b''
finally:
    if browser is not None: browser.close()
    shutil.rmtree(sys.argv[2],ignore_errors=True)
"#;

#[cfg(test)]
mod tests {
    use super::*;
    fn handoff() -> Value {
        json!({"scope":"a".repeat(64),"cookies":[{"url":ORIGINS[0],"name":"session","value":"opaque-value"}]})
    }
    #[test]
    fn handoff_is_origin_and_project_scoped() {
        assert!(
            validated_request("https://studio.stepx.com/builder/app/p_example", &handoff()).is_ok()
        );
        for bad in [
            "http://studio.stepx.com/builder/app/p_example",
            "https://studio.stepx.com.evil.test/builder/app/p_example",
            "https://studio-api.stepx.com/t/example",
            "https://studio.stepx.com/builder/app/p_example?token=secret",
        ] {
            assert!(validated_request(bad, &handoff()).is_err());
        }
        for bad in [
            "http://studio.stepx.com/",
            "https://stepx.com/",
            "https://studio.stepx.com/other",
            "https://studio.stepx.com:444/",
        ] {
            let mut value = handoff();
            value["cookies"][0]["url"] = bad.into();
            assert!(
                validated_request("https://studio.stepx.com/builder/app/p_example", &value)
                    .is_err()
            );
        }
    }
    #[test]
    fn malformed_or_duplicate_session_data_never_reaches_browser() {
        let mut quoted = handoff();
        quoted["cookies"][0]["value"] = "\"opaque-value\"".into();
        assert!(
            validated_request("https://studio.stepx.com/builder/app/p_example", &quoted).is_ok()
        );
        for bad in ["a\r\nb", "a;b", "a\"b", "a\\b"] {
            let mut value = handoff();
            value["cookies"][0]["value"] = bad.into();
            assert!(
                validated_request("https://studio.stepx.com/builder/app/p_example", &value)
                    .is_err()
            );
        }
        let mut value = handoff();
        let duplicate = value["cookies"][0].clone();
        value["cookies"].as_array_mut().unwrap().push(duplicate);
        assert!(
            validated_request("https://studio.stepx.com/builder/app/p_example", &value).is_err()
        );
        assert!(!BROWSER_WORKER.contains("--remote-debugging-port"));
    }
}
