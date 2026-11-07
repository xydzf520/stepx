"""Offline contract check for diagnostic forwarding; does not connect a device."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
JAVA = Path('/home/stepx-user/.local/opt/android-studio/jbr/bin')
ANDROID = Path('/home/stepx-user/Android/Sdk/platforms/android-36/android.jar')


class ModelDiagnosticsTest(unittest.TestCase):
    def test_original_results_errors_and_continuations_are_forwarded(self):
        with tempfile.TemporaryDirectory(prefix='stepx-model-observer-') as tmp:
            work = Path(tmp)
            files = {
                'org/json/JSONObject.java': '''package org.json;
import java.util.*; public class JSONObject {
private final Map<String,Object> data=new LinkedHashMap<>();
public JSONObject(){} public JSONObject(String s){}
public JSONObject put(String k,Object v){data.put(k,v);return this;}
public String toString(){return data.toString();}
}''',
                'org/json/JSONArray.java': '''package org.json;
import java.util.*; public class JSONArray {
private final List<Object> data=new ArrayList<>();
public JSONArray put(Object v){data.add(v);return this;}
public int length(){return data.size();}
public String toString(){return data.toString();}
}''',
                'H7/c.java': '''package H7; public interface c {
Object getContext(); void resumeWith(Object result);
}''',
                'kotlin/Result.java': '''package kotlin; public class Result {
public static class Failure { public final Throwable exception;
public Failure(Throwable t){exception=t;}}
}''',
                'kotlin/coroutines/intrinsics/CoroutineSingletons.java': '''package kotlin.coroutines.intrinsics;
public enum CoroutineSingletons { COROUTINE_SUSPENDED }
''',
                'ObserverContract.java': '''import java.lang.reflect.*;
import java.util.*;
public class ObserverContract {
public interface Cell { Object generateRequestPayload(Object input); }
public interface Service { Object chat(Object input,H7.c continuation); }
public interface Handler { Object handle(Object a,Object b,Object c); }
static final Object input=new Object(),output=new Object(),context=new Object();
static final RuntimeException originalError=new RuntimeException("SECRET_PROMPT_https://private.invalid/token");
static Object received;
static int callbacks;
public static class GoodCell implements Cell {
public Object generateRequestPayload(Object value){check(value==input);return output;}}
public static class BadCell implements Cell {
public Object generateRequestPayload(Object value){throw originalError;}}
public static class AsyncService implements Service {
public Object chat(Object value,H7.c continuation){check(value==input);check(continuation.getContext()==context);
Object failure=new kotlin.Result.Failure(originalError);received=failure;continuation.resumeWith(failure);
return kotlin.coroutines.intrinsics.CoroutineSingletons.COROUTINE_SUSPENDED;}}
public static class Caller implements H7.c {
public Object getContext(){return context;} public void resumeWith(Object r){check(r==received);callbacks++;}}
public static class NativeHandler implements Handler {
int calls;
public Object handle(Object a,Object b,Object c){check(a==input&&b==output&&c==context);calls++;return output;}}
static void check(boolean b){if(!b)throw new AssertionError();}
static Object wrap(Class<?> iface,Object original,boolean service)throws Exception {
Class<?> d=Class.forName("stepx.NativeGuiModelDiagnostics");
Method m=d.getDeclaredMethod("observer",ClassLoader.class,Class.class,Object.class,boolean.class);
m.setAccessible(true);return m.invoke(null,ObserverContract.class.getClassLoader(),iface,original,service);}
static Object stat(String field)throws Exception {
Field f=Class.forName("stepx.NativeGuiModelDiagnostics").getDeclaredField(field);f.setAccessible(true);return f.get(null);}
static byte[] errorPacket(String reason)throws Exception {
java.io.ByteArrayOutputStream body=new java.io.ByteArrayOutputStream();
body.write(new byte[]{8,(byte)0x92,0x4e,18});
byte[] value=reason.getBytes(java.nio.charset.StandardCharsets.UTF_8);
int length=value.length;while(length>=128){body.write((length&127)|128);length>>>=7;}body.write(length);body.write(value);
return body.toByteArray();}
static String classify(Method classify,byte[] body,int messageType)throws Exception {
org.json.JSONObject metadata=new org.json.JSONObject();classify.invoke(null,body,messageType,metadata);return metadata.toString();}
public static void main(String[]args)throws Exception {
check(((Cell)wrap(Cell.class,new GoodCell(),false)).generateRequestPayload(input)==output);
try {((Cell)wrap(Cell.class,new BadCell(),false)).generateRequestPayload(input);throw new AssertionError();}
catch(RuntimeException e){check(e==originalError);}
Object suspended=((Service)wrap(Service.class,new AsyncService(),true)).chat(input,new Caller());
check(suspended==kotlin.coroutines.intrinsics.CoroutineSingletons.COROUTINE_SUSPENDED && callbacks==1);
check(stat("requestCalls").equals(2)&&stat("requestGenerated").equals(1));
check(stat("serviceCalls").equals(1)&&stat("serviceResponses").equals(0)&&stat("errors").equals(2));
String events=stat("events").toString();
check(events.contains("REQUEST_GENERATION_ERROR")&&events.contains("SERVICE_CHAT_ASYNC_ERROR"));
check(!events.contains("SECRET_PROMPT")&&!events.contains("private.invalid")&&!events.contains("token"));
Class<?> ed=Class.forName("stepx.NativeGuiEventDiagnostics");
Method factory=ed.getDeclaredMethod("makeObserver",ClassLoader.class,Class.class,Object.class);factory.setAccessible(true);
NativeHandler delegate=new NativeHandler();
Handler handler=(Handler)factory.invoke(null,ObserverContract.class.getClassLoader(),Handler.class,delegate);
check(handler.handle(input,output,context)==output&&delegate.calls==1);
// Observation sees a deliberately unsupported fake header, yet forwarding must proceed.
Method shape=ed.getDeclaredMethod("wireShape",byte[].class,org.json.JSONObject.class);shape.setAccessible(true);
java.io.ByteArrayOutputStream body=new java.io.ByteArrayOutputStream();
body.write(new byte[]{8,(byte)0x93,3,18});
byte[] reason="SECRET_REASON_https://private.invalid/token".getBytes(java.nio.charset.StandardCharsets.UTF_8);
body.write(reason.length);body.write(reason);
org.json.JSONObject metadata=new org.json.JSONObject();shape.invoke(null,body.toByteArray(),metadata);
String summary=metadata.toString();check(summary.contains("error_schema_candidate_code=403"));
check(!summary.contains("SECRET_REASON")&&!summary.contains("private.invalid")&&!summary.contains("token"));
org.json.JSONObject completion=new org.json.JSONObject();shape.invoke(null,new byte[]{10,2,123,125},completion);
check(!completion.toString().contains("error_schema_candidate_code"));
shape.invoke(null,new byte[0],new org.json.JSONObject());
Method classify=ed.getDeclaredMethod("classifyErrorCandidate",byte[].class,int.class,org.json.JSONObject.class);classify.setAccessible(true);
byte[] rpcError=errorPacket("rpc error: code = Unimplemented desc = unknown service SECRET_REASON_https://private.invalid/token");
String rpcSummary=classify(classify,rpcError,4);
check(rpcSummary.contains("reason_classification_candidate=rpc_unimplemented")&&rpcSummary.contains("unknown_service=true"));
check(!rpcSummary.contains("SECRET_REASON")&&!rpcSummary.contains("private.invalid")&&!rpcSummary.contains("/token"));
check(classify(classify,rpcError,1).equals("{}"));
check(classify(classify,body.toByteArray(),4).equals("{}")); // Other error codes remain unclassified.
byte[] extra=Arrays.copyOf(rpcError,rpcError.length+2);extra[extra.length-2]=24;extra[extra.length-1]=1;
check(classify(classify,extra,4).equals("{}"));
check(classify(classify,new byte[]{8,(byte)0x92,0x4e,18,1,(byte)0xff},4).equals("{}"));
check(classify(classify,new byte[]{10,2,123,125},4).equals("{}"));
check(classify(classify,errorPacket("model not found"),4).contains("reason_classification_candidate=model_unavailable"));
check(classify(classify,errorPacket("Invalid argument"),4).contains("invalid_parameter=true"));
check(classify(classify,errorPacket("Unavailable connection refused no healthy upstream routing timeout"),4).contains("routing_timeout=true"));
check(classify(classify,errorPacket("permission denied"),4).contains("reason_classification_candidate=forbidden"));
check(classify(classify,errorPacket("opaque SECRET_REASON"),4).contains("reason_classification_candidate=unclassified"));
String resourceSummary=classify(classify,errorPacket("service not found; session_not_found; agentXYZ; rpc error: code = NotFound; ClientAgent AgentCall; step3p7-mtp3-0520-mm-reasoning-effort-fp8-aos-prod; SECRET_REASON_credential=https://private.invalid/token?account=123456789"),4);
check(resourceSummary.contains("service_not_found=true")&&resourceSummary.contains("session_not_found=true"));
check(resourceSummary.contains("agent=true")&&resourceSummary.contains("agent_not_found=false"));
check(resourceSummary.contains("mentions_default_model_id=true")&&resourceSummary.contains("mentions_client_agent=true"));
check(resourceSummary.contains("mentions_agent_call=true")&&resourceSummary.contains("grpc_not_found=true"));
check(!resourceSummary.contains("SECRET_REASON")&&!resourceSummary.contains("private.invalid")&&!resourceSummary.contains("123456789"));
check(!resourceSummary.contains("credential")&&!resourceSummary.contains("step3p7")&&!resourceSummary.contains("agentXYZ"));
check(classify(classify,errorPacket("microservice not found; service not founder; unknown_resource_kind SECRET_REASON"),4).contains("service_not_found=false"));
check(!classify(classify,errorPacket("unknown_resource_kind SECRET_REASON"),4).contains("unknown_resource_kind"));
check(classify(classify,errorPacket("x".repeat(4097)),4).equals("{}"));
byte[] duplicateReason=Arrays.copyOf(rpcError,rpcError.length+2);duplicateReason[duplicateReason.length-2]=18;
check(classify(classify,duplicateReason,4).equals("{}"));
System.out.println("observer forwarding and redaction contracts passed");
}}
''',
            }
            sources = []
            for name, source in files.items():
                path = work / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(source)
                sources.append(str(path))
            sources.append(str(ROOT / 'desktop/gui-native/NativeGuiModelDiagnostics.java'))
            sources.append(str(ROOT / 'desktop/gui-native/NativeGuiEventDiagnostics.java'))
            subprocess.run([str(JAVA / 'javac'), '--release', '11', '-cp', str(ANDROID),
                            '-d', str(work / 'classes'), *sources], check=True, capture_output=True, text=True)
            result = subprocess.run([str(JAVA / 'java'), '-cp', f'{work / "classes"}:{ANDROID}',
                                     'ObserverContract'], check=True, capture_output=True, text=True)
            self.assertIn('contracts passed', result.stdout)


if __name__ == '__main__':
    unittest.main()
