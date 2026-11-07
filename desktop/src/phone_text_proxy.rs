//! Fixed native StepOS model proxy. There is no agent/session or tool executor.
//! Android supplies its own account headers; no desktop or phone credentials
//! are read, copied or sent by this connector.
use super::{check_cancel, validate_document, InferenceMeta, SelectedText, Source, TextBrief, API_SCRIPT, JSON_SYSTEM, MAX_PROMPT_BYTES};
use crate::{adb::Adb, amoo_usage, device::Device};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, io::{Read, Write}, path::PathBuf, process::Stdio, sync::atomic::{AtomicBool, Ordering}, thread, time::{Duration, Instant}};

const APK_SHA256:&str="ff68746ce6b03f929c0c90f6717918c49309f9b1816d7ae1fecc9341f6fcef1a";
const CHANNEL:&str="stepos_native_proxy";
const ISOLATION:&str="stateless_native_proxy";
const PORT:&str="tcp:18791";
// The audited firmware still selects its Meizu account adapter. The current
// phone has only a StepOS account; a reachable proxy or valid JSON must never
// be promoted into proof that it consumes that account's model allowance.
const ACCOUNT_BINDING_SUPPORTED:bool=false;
pub(super) const ACCOUNT_BINDING_REASON:&str="手机账号支持手动整理；当前客户端尚未适配手机账号的后台纯文本通道，无法开启无人值守生成。解锁或重复验证无法解决；资料同步继续运行";
pub(super) fn probe_supported()->bool {ACCOUNT_BINDING_SUPPORTED}
#[derive(Clone,Debug,Serialize,Deserialize)]
struct Proof {schema:u32,device:String,account_scope:String,apk_sha256:String,checked_ms:i64,channel:String,isolation:String,tool_calls:u32,actual_model:String}
fn path(device:&str,account:&str)->PathBuf {
    crate::resources::user_state().join("stepx-companion/native-text-proof")
        .join(format!("{}.json",crate::context_ledger::scope_id(device,Some(account))))
}
pub(super) fn available(device:&str,account:Option<&str>)->bool {
    if !ACCOUNT_BINDING_SUPPORTED {return false;}
    let Some(account)=account.filter(|account|!account.is_empty()) else {return false;};
    let path=path(device,account);
    let Ok(meta)=fs::symlink_metadata(&path) else {return false;};
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len()>8192 {return false;}
    let Ok(bytes)=fs::read(path) else {return false;};
    serde_json::from_slice::<Proof>(&bytes).is_ok_and(|p|p.schema==1 && p.device==device && p.account_scope==account
        && p.apk_sha256==APK_SHA256 && p.channel==CHANNEL && p.isolation==ISOLATION && p.tool_calls==0)
}
fn save_proof(device:&str,account:&str,actual_model:&str)->Result<()> {
    let path=path(device,account);let parent=path.parent().context("原生模型验证目录无效")?;
    fs::create_dir_all(parent)?;ensure!(!parent.is_symlink(),"原生模型验证目录不能是链接");
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;fs::set_permissions(parent,fs::Permissions::from_mode(0o700))?;}
    if path.exists() {ensure!(!path.is_symlink(),"原生模型验证记录不能是链接");}
    let proof=Proof {schema:1,device:device.into(),account_scope:account.into(),apk_sha256:APK_SHA256.into(),
        checked_ms:chrono::Utc::now().timestamp_millis(),channel:CHANNEL.into(),isolation:ISOLATION.into(),tool_calls:0,actual_model:actual_model.into()};
    let mut temporary=tempfile::NamedTempFile::new_in(parent)?;temporary.write_all(&serde_json::to_vec(&proof)?)?;
    temporary.as_file().sync_all()?;temporary.persist(path).map_err(|error|error.error)?;Ok(())
}
fn current_account(adb:&Adb,device:&Device,expected:Option<&str>)->Result<(String,String)> {
    let transport=device.resolve_active_transport(adb)?;
    ensure!(adb.shell(&transport,&["getprop","ro.serialno"])?.trim()==device.serial,"手机身份已变化，未请求模型");
    let account=amoo_usage::account_scope_fresh(adb,device)?.context("请先登录手机 Amoo 账号")?;
    ensure!(expected.is_some_and(|expected|expected==account),"手机账号已变化或未绑定，未发送后台上下文");
    Ok((transport,account))
}
fn verify_apk(adb:&Adb,transport:&str)->Result<()> {
    let output=adb.shell(transport,&["pm","path","com.stepos.stepclaw"])?;
    let paths:Vec<_>=output.lines().filter_map(|line|line.trim().strip_prefix("package:")).collect();
    ensure!(paths.len()==1 && paths[0].starts_with('/') && !paths[0].chars().any(char::is_control),"无法确认原生模型代理所在系统包");
    let checksum=adb.shell(transport,&["sha256sum",paths[0]])?;
    ensure!(checksum.split_whitespace().next()==Some(APK_SHA256),"手机原生模型代理版本已变化，须重新核验无工具转发能力；未改用会话或独立 API");
    Ok(())
}
pub(super) fn ensure(adb:&Adb,device:&Device,expected:Option<&str>,require_proof:bool)->Result<()> {
    ensure!(ACCOUNT_BINDING_SUPPORTED,"{}",ACCOUNT_BINDING_REASON);
    let (transport,account)=current_account(adb,device,expected)?;verify_apk(adb,&transport)?;
    ensure!(!require_proof || available(&device.serial,Some(&account)),"请先验证当前手机账号的专用无工具模型通道，再开启后台生成");
    // Merely connecting performs no HTTP inference and creates no native task.
    let (stream,_,_)=adb.connect_forwarded_tcp(&transport,PORT,Duration::from_secs(3))?;drop(stream);
    current_account(adb,device,Some(&account))?;Ok(())
}

const SCRIPT:&str=r#"
def proxy_http_failure(response):
    # Never surface vendor error messages: they can echo prompts or headers.
    raw=response.read(16385)
    metadata={'status':response.status,'response_bytes':min(len(raw),16385),'truncated':len(raw)>16384,'classification':'unknown'}
    try:data=json.loads(raw) if len(raw)<=16384 else None
    except (ValueError,UnicodeError):data=None
    if isinstance(data,dict):
        known=('error','detail','message','code','type','status')
        metadata['fields']=[key for key in known if key in data]
        metadata['other_fields']=sum(key not in known for key in data)
        error=data.get('error')
        details=error if isinstance(error,dict) else data
        for key in ('code','type'):
            value=details.get(key)
            allowed=('model_not_found','not_found','invalid_request_error','authentication_error','permission_error','rate_limit_error','insufficient_quota','unsupported_parameter')
            if isinstance(value,str):metadata['error_'+key]=value if value in allowed else 'other'
        # Inspect only in memory, emitting a finite category rather than text.
        fragments=[details.get('code'),details.get('type'),details.get('message'),data.get('detail'),error if isinstance(error,str) else None]
        words=' '.join(value.lower() for value in fragments if isinstance(value,str))
        if 'model' in words and any(value in words for value in ('not found','not_found','does not exist','unknown model','unsupported model')):category='model_not_found'
        elif any(value in words for value in ('quota','credit','balance','billing')):category='quota'
        elif any(value in words for value in ('rate limit','rate_limit','too many requests')):category='rate_limit'
        elif any(value in words for value in ('unauthorized','authentication','invalid api key','permission denied','forbidden')):category='authentication'
        elif any(value in words for value in ('not found','not_found','no route','cannot post','no static resource')):category='path_not_found'
        elif any(value in words for value in ('unsupported parameter','unsupported_parameter','unknown parameter')):category='unsupported_parameter'
        else:category='unknown'
        metadata['classification']=category
    return json.dumps(metadata,ensure_ascii=True,separators=(',',':'))

def proxy_run():
    import http.client
    request=json.loads(sys.stdin.buffer.read(262145).decode('utf-8'))
    port=int(sys.argv[1])
    if not 0<port<65536: raise ReplyError('原生模型代理端口无效')
    body={'model':'stepos-model-auto','messages':[{'role':'system','content':request['system']},{'role':'user','content':request['prompt']}],
          'stream':True,'tools':[],'tool_choice':'none','max_tokens':16384}
    connection=http.client.HTTPConnection('127.0.0.1',port,timeout=180)
    try:
        # This native proxy already forwards under /model-proxy. Its live
        # upstream rejects the bundled gateway config's extra /v1 prefix.
        connection.request('POST','/chat/completions',body=json.dumps(body).encode('utf-8'),
                           headers={'Content-Type':'application/json','User-Agent':'StepX-Native-Text/1.0','Connection':'close'})
        response=connection.getresponse()
        if response.status!=200: raise ReplyError('手机原生模型代理请求失败；未改用其他来源。诊断 '+proxy_http_failure(response))
        actual_model=response.getheader('X-Aigw-Model-Name') or ''
        if response.headers.get_content_type()=='text/event-stream':
            data=read_openai_sse(response,'',allow_json_done=True)
        else:
            raw=response.read(524289)
            if len(raw)>524288: raise ReplyError('手机模型响应超过容量上限')
            data=json.loads(raw)
            DIAGNOSTIC['terminal_proof']='provider_stop'
        if not isinstance(data,dict) or data.get('error'): raise ReplyError('手机模型响应结构无效')
        choices=data.get('choices')
        if not isinstance(choices,list) or len(choices)!=1 or not isinstance(choices[0],dict): raise ReplyError('手机模型没有返回唯一完整结果')
        choice=choices[0];message=choice.get('message') or {}
        if not isinstance(message,dict) or message.get('tool_calls') or message.get('function_call'): raise ReplyError('手机模型返回工具调用，未执行，也未保存为成果')
        reason=choice.get('finish_reason')
        if reason!='stop' and not (reason is None and DIAGNOSTIC.get('terminal_proof')=='vendor_done_complete_json_object'):
            raise ReplyError('手机模型没有完整结束输出，未保存为成果')
        text=message.get('content')
        if not isinstance(text,str) or len(text.encode('utf-8'))>65536 or not complete_json_object(text):
            raise ReplyError('手机模型未返回完整、非空且无重复字段的 JSON 对象')
        returned=data.get('model')
        if isinstance(returned,str) and returned and returned!='stepos-model-auto': actual_model=returned
        if actual_model=='stepos-model-auto':actual_model=''
        usage=data.get('usage') or {}
        safe_usage={k:v for k,v in usage.items() if isinstance(v,int) and not isinstance(v,bool) and v>=0 and k in ('prompt_tokens','completion_tokens','total_tokens','input_tokens','output_tokens','cache_read_input_tokens','cache_creation_input_tokens')} if isinstance(usage,dict) else {}
        print(json.dumps({'text':text,'model':actual_model[:100],'usage':safe_usage,'terminal_proof':DIAGNOSTIC.get('terminal_proof'),'tool_calls':0},ensure_ascii=False))
    finally:connection.close()
try:
    proxy_run()
except ReplyError as error:
    sys.stderr.write(str(error));sys.exit(1)
except Exception as error:
    sys.stderr.write('手机原生模型响应失败（'+type(error).__name__+'），原始上下文保留；未改用会话或独立 API');sys.exit(1)
"#;
#[derive(Deserialize)]
struct Reply {text:String,model:String,usage:Value,tool_calls:u32}
fn request(adb:&Adb,transport:&str,prompt:&str,cancel:&AtomicBool)->Result<Reply> {
    check_cancel(cancel)?;
    ensure!(!prompt.trim().is_empty() && prompt.len()+JSON_SYSTEM.len()+1024<=MAX_PROMPT_BYTES,"后台资料为空或超过原生模型单次容量");
    let (_,port,_)=adb.connect_forwarded_tcp(transport,PORT,Duration::from_secs(3))?;
    let definitions=API_SCRIPT.rsplit_once("\ntry:\n    run()\n").context("原生模型解析器缺失")?.0;
    let payload=serde_json::to_vec(&json!({"system":JSON_SYSTEM,"prompt":prompt}))?;
    ensure!(payload.len()<=MAX_PROMPT_BYTES,"后台证据序列化后超过容量，尚未发送");
    let mut command=crate::resources::python_command();command.arg("-I").arg("-c").arg(format!("{definitions}\n{SCRIPT}")).arg(port.to_string())
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(target_os="linux")] {use std::os::unix::process::CommandExt;let parent=std::process::id() as libc::pid_t;
        unsafe {command.pre_exec(move || {if libc::prctl(libc::PR_SET_PDEATHSIG,libc::SIGKILL)!=0 {return Err(std::io::Error::last_os_error());}if libc::getppid()!=parent {libc::_exit(125);}Ok(())});}}
    let mut child=command.spawn().context("无法启动手机原生模型连接器")?;
    let stdout=child.stdout.take().context("模型输出不可读")?;let stderr=child.stderr.take().context("模型状态不可读")?;
    let output=thread::spawn(move || {let mut bytes=vec![];stdout.take(400000).read_to_end(&mut bytes).map(|_|bytes)});
    let errors=thread::spawn(move || {let mut bytes=vec![];stderr.take(2048).read_to_end(&mut bytes).map(|_|bytes)});
    let input=child.stdin.take().context("模型输入不可写").and_then(|mut input|input.write_all(&payload).map_err(Into::into));
    let deadline=Instant::now()+Duration::from_secs(185);
    let status=loop {
        if input.is_err() || cancel.load(Ordering::Acquire) || Instant::now()>=deadline {let _=child.kill();let _=child.wait();break None;}
        match child.try_wait() {Ok(Some(status))=>break Some(status),Ok(None)=>thread::sleep(Duration::from_millis(25)),Err(_)=>{let _=child.kill();let _=child.wait();break None;}}
    };
    let bytes=output.join().map_err(|_|anyhow::anyhow!("原生模型输出读取失败"))??;
    let errors=errors.join().map_err(|_|anyhow::anyhow!("原生模型状态读取失败"))??;
    check_cancel(cancel)?;input?;let status=status.context("手机原生模型请求超时；未改用其他来源")?;
    ensure!(status.success(),"{}",String::from_utf8_lossy(&errors));
    serde_json::from_slice(&bytes).context("手机原生模型没有返回有效结果")
}
pub(super) fn generate(adb:&Adb,device:&Device,expected:Option<&str>,prompt:&str,cancel:&AtomicBool,require_proof:bool,
    record:&mut dyn FnMut(&InferenceMeta)->Result<()>) -> Result<SelectedText> {
    check_cancel(cancel)?;ensure(adb,device,expected,require_proof)?;
    let (transport,account)=current_account(adb,device,expected)?;
    // Native HTTP proxy owns account authentication. There is no API key, native
    // agent conversation, execution loop or tool dispatch in this code path.
    let reply=request(adb,&transport,prompt,cancel)?;
    current_account(adb,device,Some(&account)).map_err(|_|anyhow::anyhow!(
        "手机模型请求已发出，但返回后无法确认原手机身份及账号；结果未采用，本次用量待核对；未改用其他来源"
    ))?;check_cancel(cancel)?;
    ensure!(reply.tool_calls==0,"原生模型工具隔离回执无效");
    let markdown=validate_document(&reply.text)?;
    let usage=amoo_usage::parse_message(&json!({"usage":reply.usage}));
    let inference=InferenceMeta {source:Source::PhoneAccount,model:reply.model.clone(),provider:"StepOS 原生模型代理".into(),
        usage:usage.has_tokens().then_some(usage),account_scope:Some(account.clone()),observed_ms:chrono::Utc::now().timestamp_millis(),
        session_cleaned:false,temporary_session_key:None,channel:Some(CHANNEL.into()),isolation:Some(ISOLATION.into()),session_cleanup:Some("not_required_stateless".into())};
    record(&inference)?;
    // Bootstrap must persist its capability receipt before enabling automation.
    // Already verified runs must not lose a valid result to an incidental proof
    // refresh failure; account, APK and response are verified on every request.
    if !require_proof {save_proof(&device.serial,&account,&reply.model)?;}
    Ok(SelectedText {brief:TextBrief {markdown,model:reply.model,session_cleaned:false},inference:Some(inference)})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_native_account_adapter_blocks_probe_and_automation_before_io() {
        let adb=Adb {path:PathBuf::from("/nonexistent-stepx-account-audit-adb")};
        let device=Device {transport:"unreachable-test-transport".into(),serial:"account-audit-test".into(),model:String::new(),android:String::new(),product:String::new()};
        for require_proof in [false,true] {
            let error=ensure(&adb,&device,Some("opaque-account-scope"),require_proof).unwrap_err();
            assert_eq!(error.to_string(),ACCOUNT_BINDING_REASON);
        }
        assert!(!available(&device.serial,Some("opaque-account-scope")));
    }
    #[test]
    fn proxy_connector_never_advertises_tools_credentials_or_an_agent_session() {
        let definitions=API_SCRIPT.rsplit_once("\ntry:\n    run()\n").unwrap().0;
        let fixture=r#"
import io, email.message, contextlib, unittest.mock as mock
class Response(io.BytesIO):
    def __init__(self,payload,status=200,kind='application/json'):
        super().__init__(payload);self.status=status;self.headers=email.message.Message();self.headers['Content-Type']=kind
    def getheader(self,key):return 'actual-native-model' if key=='X-Aigw-Model-Name' else self.headers.get(key)
for body,accepted,status in [
    ({'choices':[{'finish_reason':'stop','message':{'content':'{"ok":true}'}}],'usage':{'prompt_tokens':4,'completion_tokens':2}},True,200),
    ({'choices':[{'finish_reason':'length','message':{'content':'{"ok":true}'}}]},False,200),
    ({'choices':[{'message':{'content':'{"ok":true}'}}]},False,200),
    ({'choices':[{'finish_reason':'stop','message':{'content':'{"ok":true}','tool_calls':[{'id':'forbidden'}]}}]},False,200),
    ({'choices':[{'finish_reason':'stop','message':{'content':'partial'}}]},False,200),
    ({'choices':[{'finish_reason':'stop','message':{'content':'{"key":1,"key":2}'}}]},False,200),
    ({'choices':[{'finish_reason':'stop','message':{'content':'{"ok":true}'}}]},False,302),
]:
    captured={}
    class Connection:
        def __init__(self,host,port,timeout):assert host=='127.0.0.1' and port==12345
        def request(self,method,path,body,headers):
            assert method=='POST' and path=='/chat/completions'
            captured.update(json.loads(body));assert not any(k.lower()=='authorization' for k in headers)
        def getresponse(self):return Response(json.dumps(body).encode(),status)
        def close(self):pass
    namespace=globals().copy();namespace['sys']=mock.Mock();namespace['sys'].argv=['fixture','12345'];namespace['sys'].stdin=io.TextIOWrapper(io.BytesIO(json.dumps({'system':'system','prompt':'fixture'}).encode()));namespace['sys'].stderr=io.StringIO();namespace['sys'].exit=lambda code:(_ for _ in ()).throw(SystemExit(code))
    output=io.StringIO()
    with mock.patch('http.client.HTTPConnection',Connection),contextlib.redirect_stdout(output):
        try:exec(NATIVE_SCRIPT,namespace)
        except SystemExit:assert not accepted
        else:assert accepted
    assert captured['tools']==[] and captured['tool_choice']=='none' and 'sessionId' not in captured
    if accepted:
        result=json.loads(output.getvalue());assert result['tool_calls']==0 and result['model']=='actual-native-model'
    else:assert output.getvalue()==''
namespace=globals().copy()
exec(NATIVE_SCRIPT.split('\ntry:\n    proxy_run()')[0],namespace)
for payload,category in [
    ({'error':{'code':'model_not_found','type':'invalid_request_error','message':'Unknown model private-account-marker'}},'model_not_found'),
    ({'detail':'Not Found'},'path_not_found'),
    ({'error':'request_failed','message':'No static resource v1/chat/completions.','status':404},'path_not_found'),
    ({'error':{'message':'Authentication failed for private-account-marker'}},'authentication'),
    ({'error':{'code':'private-account-marker','message':'Balance exhausted'}},'quota'),
    ({'error':{'message':'private-account-marker'}},'unknown'),
]:
    safe=namespace['proxy_http_failure'](Response(json.dumps(payload).encode(),404))
    assert 'private-account-marker' not in safe
    assert json.loads(safe)['classification']==category
safe=namespace['proxy_http_failure'](Response(b'x'*16385,404))
assert json.loads(safe)['truncated'] is True and json.loads(safe)['classification']=='unknown'
"#;
        let script=format!("{definitions}\nNATIVE_SCRIPT={}\n{fixture}",serde_json::to_string(SCRIPT).unwrap());
        let output=crate::resources::python_command().arg("-I").arg("-c").arg(script).output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    }
}
