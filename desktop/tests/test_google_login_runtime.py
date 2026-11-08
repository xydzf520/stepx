"""Run the phone Google observer on a deterministic JVM Android event loop.

No device, permission mutation, downloaded APK or private data is used. Signatures,
PackageManager and system-service registration are host mocks; production event,
retry, scope, read-back and cancellation logic executes without source rewriting.
"""
from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
JDK = Path(os.environ.get('STEPX_JDK', '/home/stepx-user/.local/opt/android-studio/jbr')) / 'bin'
JAVAC = str(JDK / 'javac') if (JDK / 'javac').is_file() else shutil.which('javac')
JAVA = str(JDK / 'java') if (JDK / 'java').is_file() else shutil.which('java')
STUBS = {
'android/net/Uri.java': '''package android.net; public class Uri { final String p;public Uri(String p){this.p=p;}public String getSchemeSpecificPart(){return p;} }''',
'android/content/Intent.java': '''package android.content; public class Intent {public static final String ACTION_PACKAGE_REPLACED="replaced",ACTION_PACKAGE_FULLY_REMOVED="removed",ACTION_USER_UNLOCKED="unlocked";final String action;final android.net.Uri data;String pkg;public android.os.Bundle extras;public Intent(String a){this(a,null);}public Intent(String a,String p){action=a;data=p==null?null:new android.net.Uri(p);}public Intent setPackage(String p){pkg=p;return this;}public String getPackage(){return pkg;}public android.os.Bundle getExtras(){return extras;}public String getAction(){return action;}public android.net.Uri getData(){return data;} }''',
'android/content/IntentFilter.java': '''package android.content; public class IntentFilter {public final java.util.Set<String> actions=new java.util.HashSet<>();public String scheme;public IntentFilter(String a){actions.add(a);}public void addAction(String a){actions.add(a);}public void addDataScheme(String s){scheme=s;} }''',
'android/content/ComponentName.java': '''package android.content; public class ComponentName {final String p,n;public ComponentName(String p,String n){this.p=p;this.n=n;}public String flattenToString(){return p+"/"+n;}public boolean equals(Object o){return o instanceof ComponentName&&flattenToString().equals(((ComponentName)o).flattenToString());}public int hashCode(){return flattenToString().hashCode();} }''',
'android/content/BroadcastReceiver.java': '''package android.content;public abstract class BroadcastReceiver {public abstract void onReceive(Context c,Intent i);}''',
'android/content/pm/ApplicationInfo.java': '''package android.content.pm;public class ApplicationInfo {public static final int FLAG_SYSTEM=1,FLAG_SUSPENDED=1<<30;public int uid=10200,flags=FLAG_SYSTEM;public boolean enabled=true;}''',
'android/content/pm/Signature.java': '''package android.content.pm;public class Signature {final byte value;public Signature(byte v){value=v;}public byte[] toByteArray(){return new byte[]{value};}}''',
'android/content/pm/SigningInfo.java': '''package android.content.pm;public class SigningInfo {public byte value;public boolean multiple;public byte[] history;public boolean hasPastSigningCertificates(){return history!=null&&history.length>1;}public Signature[] getSigningCertificateHistory(){if(history==null)return null;Signature[] r=new Signature[history.length];for(int i=0;i<r.length;i++)r[i]=new Signature(history[i]);return r;}public Signature[] getApkContentsSigners(){return multiple?new Signature[]{new Signature(value),new Signature(value)}:new Signature[]{new Signature(value)};}}''',
'android/content/pm/PackageInfo.java': '''package android.content.pm;public class PackageInfo {public long version=10,lastUpdateTime=100;public ApplicationInfo applicationInfo=new ApplicationInfo();public SigningInfo signingInfo=new SigningInfo();public String[] requestedPermissions={"android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE","android.permission.DOMAIN_VERIFICATION_AGENT"};public long getLongVersionCode(){return version;}}''',
'android/content/pm/PermissionInfo.java': '''package android.content.pm;public class PermissionInfo {public String packageName="android";}''',
'android/content/pm/ServiceInfo.java': '''package android.content.pm;public class ServiceInfo {public boolean enabled=true,exported=true;public String permission="android.permission.BIND_CREDENTIAL_PROVIDER_SERVICE";}''',
'android/content/pm/ActivityInfo.java': '''package android.content.pm;public class ActivityInfo {public String packageName="com.google.android.gms",name="com.google.android.gms.chimera.GmsIntentOperationService$PersistentTrustedReceiver";}''',
'android/content/pm/ResolveInfo.java': '''package android.content.pm;public class ResolveInfo {public ActivityInfo activityInfo=new ActivityInfo();}''',
'android/content/pm/PackageManager.java': '''package android.content.pm;
public class PackageManager {
 public static final int CERT_INPUT_SHA256=1,GET_SIGNING_CERTIFICATES=1,GET_PERMISSIONS=2,GET_META_DATA=4,MATCH_DISABLED_COMPONENTS=8,PERMISSION_GRANTED=0,COMPONENT_ENABLED_STATE_DISABLED=2,COMPONENT_ENABLED_STATE_DISABLED_USER=3,COMPONENT_ENABLED_STATE_DISABLED_UNTIL_USED=4;
 public static class NameNotFoundException extends Exception {}
 public final java.util.Map<String,PackageInfo> packages=new java.util.HashMap<>();
 public final ServiceInfo service=new ServiceInfo();
 public String absent="",denied="",unavailable="",owner="android";public int reads,checkReads,componentState;public boolean anchorVerified=true;
 public PackageManager(){for(String p:new String[]{"com.google.android.gms","com.android.vending","com.google.android.gsf"}){PackageInfo i=new PackageInfo();i.signingInfo.value=(byte)(p.equals("com.google.android.gms")?0:1);packages.put(p,i);}}
 public boolean hasSigningCertificate(String pkg,byte[] anchor,int type){if(!pkg.equals("com.google.android.gsf")||type!=CERT_INPUT_SHA256||anchor.length!=32)throw new AssertionError();return anchorVerified;}
 public PackageInfo getPackageInfo(String p,int f)throws NameNotFoundException{reads++;if(p.equals(absent)||!packages.containsKey(p))throw new NameNotFoundException();return packages.get(p);}
 public PermissionInfo getPermissionInfo(String p,int f)throws NameNotFoundException{if(p.equals(unavailable))throw new NameNotFoundException();PermissionInfo i=new PermissionInfo();i.packageName=owner;return i;}
 public int checkPermission(String p,String pkg){checkReads++;return p.equals(denied)?-1:0;}
 public int getComponentEnabledSetting(android.content.ComponentName c){return componentState;}
 public ServiceInfo getServiceInfo(android.content.ComponentName c,int f){return service;}
 public java.util.List<ResolveInfo> queryBroadcastReceivers(android.content.Intent i,int f){return java.util.Arrays.asList(new ResolveInfo());}
}''',
'android/content/Context.java': '''package android.content;public class Context {public static final int RECEIVER_NOT_EXPORTED=4,RECEIVER_EXPORTED=2;public boolean failRegister;public final java.util.List<BroadcastReceiver> receivers=new java.util.ArrayList<>();public final android.content.pm.PackageManager pm=new android.content.pm.PackageManager();public android.content.pm.PackageManager getPackageManager(){return pm;}public <T>T getSystemService(Class<T> type){return type.cast(new android.telecom.TelecomManager());}public void registerReceiver(BroadcastReceiver r,IntentFilter f,String permission,android.os.Handler handler,int flags){if(f.actions.contains("com.stepx.companion.action.RECHECK_GOOGLE_RUNTIME") ? !(f.actions.size()==1&&f.scheme==null&&flags==2&&"android.permission.DUMP".equals(permission)) : (permission!=null||flags!=4||!((f.actions.size()==2&&f.actions.contains("replaced")&&f.actions.contains("removed")&&"package".equals(f.scheme))||(f.actions.size()==1&&f.actions.contains("unlocked")&&f.scheme==null))))throw new AssertionError("unbounded receiver");if(failRegister)throw new IllegalStateException();if(!receivers.contains(r))receivers.add(r);}public void unregisterReceiver(BroadcastReceiver r){receivers.remove(r);}public void send(String action,String p){sendIntent(new Intent(action,p));}public void sendIntent(Intent i){for(BroadcastReceiver r:new java.util.ArrayList<>(receivers))r.onReceive(this,i);}}''',
'android/app/ActivityThread.java': '''package android.app;public class ActivityThread {public static final android.content.Context CONTEXT=new android.content.Context();public static ActivityThread currentActivityThread(){return new ActivityThread();}public android.content.Context getSystemContext(){return CONTEXT;}}''',
'android/os/Bundle.java': '''package android.os;public class Bundle {public boolean isEmpty(){return false;}}''',
'android/os/SystemClock.java': '''package android.os;public class SystemClock {public static long elapsed;public static long elapsedRealtime(){return elapsed;}}''',
'android/telecom/TelecomManager.java': '''package android.telecom;public class TelecomManager {public static boolean busy;public boolean isInCall(){return busy;}}''',
'android/os/Build.java': '''package android.os;public class Build {public static String FINGERPRINT="StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/12:user/release-keys";}''',
'android/os/Looper.java': '''package android.os;public class Looper {boolean quit;public void quitSafely(){if(!quit){quit=true;HandlerThread.live--;}}}''',
'android/os/HandlerThread.java': '''package android.os;public class HandlerThread {public static int live;final Looper looper=new Looper();public HandlerThread(String n,int p){}public void start(){live++;}public Looper getLooper(){return looper;}}''',
'android/os/Handler.java': '''package android.os;public class Handler {
 public static final java.util.List<Handler> ALL=new java.util.ArrayList<>();public static final java.util.List<Long> DELAYS=new java.util.ArrayList<>();final java.util.Set<Runnable> pending=new java.util.LinkedHashSet<>();final Looper looper;
 public Handler(Looper l){looper=l;ALL.add(this);}public Looper getLooper(){return looper;}public void removeCallbacks(Runnable r){pending.remove(r);}public void postDelayed(Runnable r,long delay){pending.add(r);DELAYS.add(delay);}public void removeCallbacksAndMessages(Object token){pending.clear();}
 public static int count(){int n=0;for(Handler h:ALL)n+=h.pending.size();return n;}
 public static void drain(){for(Handler h:new java.util.ArrayList<>(ALL))for(Runnable r:new java.util.ArrayList<>(h.pending)){h.pending.remove(r);r.run();}}
}''',
'android/os/Process.java': '''package android.os;public class Process {public static final int THREAD_PRIORITY_BACKGROUND=10;public static int uid=1000;public static int myUid(){return uid;}public static int myPid(){return 1234;}}''',
'android/os/Binder.java': '''package android.os;public class Binder {public static int cleared;public static long clearCallingIdentity(){cleared++;return 42;}public static void restoreCallingIdentity(long t){if(t!=42)throw new AssertionError();cleared--;}}''',
'android/util/AtomicFile.java': '''package android.util;public class AtomicFile {public static int writes;public AtomicFile(java.io.File f){}public java.io.FileOutputStream startWrite()throws Exception{return new java.io.FileOutputStream(java.io.File.createTempFile("google-receipt",".json"));}public void finishWrite(java.io.FileOutputStream f)throws Exception{f.close();writes++;}public void failWrite(java.io.FileOutputStream f){try{f.close();}catch(Exception ignored){}}}''',
'android/system/Os.java': '''package android.system;public class Os {public static void chmod(String p,int mode){if(mode!=0600)throw new AssertionError();}}''',
'org/json/JSONObject.java': '''package org.json;public class JSONObject {public static JSONObject report;public final java.util.Map<String,Object> values=new java.util.HashMap<>();public JSONObject put(String k,Object v){values.put(k,v);if(k.equals("schema"))report=this;return this;}public boolean optBoolean(String k){return Boolean.TRUE.equals(values.get(k));}public String optString(String k){return String.valueOf(values.getOrDefault(k,""));}public String toString(){return values.toString();}}''',
'android/service/credentials/CredentialProviderInfo.java': '''package android.service.credentials;public class CredentialProviderInfo {public boolean isSystemProvider(){return true;}public android.content.ComponentName getComponentName(){return new android.content.ComponentName("com.google.android.gms","com.google.android.gms.auth.api.credentials.credman.service.GoogleIdService");}}''',
'android/service/credentials/CredentialProviderInfoFactory.java': '''package android.service.credentials;public class CredentialProviderInfoFactory {public static CredentialProviderInfo create(android.content.Context c,android.content.pm.ServiceInfo s,boolean a,boolean b,boolean d,boolean e){return new CredentialProviderInfo();}}''',
'mock/CredentialService.java': '''package mock;public class CredentialService {public final Object mLock=new Object();public final Cache mSystemServicesCacheList=new Cache();public static class Cache {public Object list=new java.util.ArrayList<>();public Object get(int i){return list;}public void put(int i,Object o){list=o;}}}''',
'com/android/server/credentials/CredentialManagerServiceImpl.java': '''package com.android.server.credentials;public class CredentialManagerServiceImpl {public final android.service.credentials.CredentialProviderInfo mInfo;public CredentialManagerServiceImpl(mock.CredentialService s,Object lock,int uid,android.service.credentials.CredentialProviderInfo info){mInfo=info;}}''',
'com/android/server/pm/verify/domain/proxy/DomainVerificationProxy.java': '''package com.android.server.pm.verify.domain.proxy;public interface DomainVerificationProxy {android.content.ComponentName getComponentName();}''',
'com/android/server/pm/verify/domain/proxy/DomainVerificationProxyUnavailable.java': '''package com.android.server.pm.verify.domain.proxy;public class DomainVerificationProxyUnavailable implements DomainVerificationProxy {public android.content.ComponentName getComponentName(){return null;}}''',
'com/android/server/pm/verify/domain/proxy/DomainVerificationProxyV2.java': '''package com.android.server.pm.verify.domain.proxy;public class DomainVerificationProxyV2 implements DomainVerificationProxy {public interface Connection {}final android.content.ComponentName component;public DomainVerificationProxyV2(android.content.Context c,Connection x,android.content.ComponentName n){component=n;}public android.content.ComponentName getComponentName(){return component;}}''',
'mock/DomainService.java': '''package mock;import com.android.server.pm.verify.domain.proxy.*;public class DomainService {public final Object mLock=new Object();public final DomainVerificationProxyV2.Connection mConnection=new DomainVerificationProxyV2.Connection(){};public DomainVerificationProxy proxy=new DomainVerificationProxyUnavailable();public DomainVerificationProxy getProxy(){return proxy;}public void setProxy(DomainVerificationProxy p){proxy=p;}}''',
'android/os/ServiceManager.java': '''package android.os;public class ServiceManager {public static boolean absent;public static final CredentialBinder credential=new CredentialBinder();public static final DomainBinder domain=new DomainBinder();public static class CredentialBinder {public final mock.CredentialService this$0=new mock.CredentialService();}public static class DomainBinder {public final mock.DomainService mService=new mock.DomainService();}public static Object getService(String n){if(absent)return null;return n.equals("credential")?credential:domain;}}''',
'stepx/GmsProbe.java': '''package stepx;public class GmsProbe {public static int calls;public static boolean fail,stop,change,clearDenied=true;public static String run(){calls++;if(stop)GoogleLoginRuntime.stopInstalled();android.content.pm.PackageManager pm=android.app.ActivityThread.CONTEXT.pm;if(change)pm.packages.get("com.google.android.gms").version++;if(fail)return "ERROR transient";if(clearDenied)pm.denied="";pm.packages.get("com.google.android.gms").applicationInfo.flags|=1;return "GRANT_RESULT\\nSYSTEM_GMS=true\\n";}}''',
'stepx/GoogleHarness.java': '''package stepx;
import android.content.*;import android.content.pm.*;import android.os.*;import org.json.JSONObject;
public class GoogleHarness {
 static final String GMS="com.google.android.gms",PLAY="com.android.vending",GSF="com.google.android.gsf";
 static final Context context=android.app.ActivityThread.CONTEXT;static final PackageManager pm=context.pm;
 // Test-only provider supplies the two public pins for deterministic signature fixtures.
 // An invalid fixture, multiple signers and wrong UID still run the production rejection.
 public static class Digest extends java.security.MessageDigestSpi {byte value;protected void engineUpdate(byte b){value=b;}protected void engineUpdate(byte[] b,int o,int l){value=b[o];}protected void engineReset(){}protected byte[] engineDigest(){String pin=value==0?"5f2391277b1dbd489000467e4c2fa6af802430080457dce2f618992e9dfb5402":value==1?"7ce83c1b71f3d572fed04c8d40c5cb10ff75e6d87d9df6fbd53f0468c2905053":"0000000000000000000000000000000000000000000000000000000000000000";byte[] r=new byte[32];for(int i=0;i<32;i++)r[i]=(byte)Integer.parseInt(pin.substring(i*2,i*2+2),16);return r;}}
 static void check(boolean v,String m){if(!v)throw new AssertionError(m+" report="+(JSONObject.report==null?"none":JSONObject.report.toString()));}
 static void status(String value){check(JSONObject.report!=null&&value.equals(JSONObject.report.optString("status")),"expected "+value);}
 static Intent recheck(){return new Intent("com.stepx.companion.action.RECHECK_GOOGLE_RUNTIME").setPackage("android");}
 static void update(String pkg){context.send(Intent.ACTION_PACKAGE_REPLACED,pkg);}
 static void install()throws Exception{GoogleLoginRuntime.install();}
 static void ready(){Handler.drain();status("ready");check(JSONObject.report.optBoolean("permissions_verified"),"permissions missing");check(JSONObject.report.optBoolean("signatures_verified"),"signatures missing");check(JSONObject.report.optBoolean("registered"),"provider missing");}
 static void closed(){GoogleLoginRuntime.stopInstalled();Handler.drain();check(context.receivers.isEmpty()&&HandlerThread.live==0&&Handler.count()==0,"leaked observer");check(Binder.cleared==0,"identity not restored");}
 public static void main(String[] args)throws Exception{
  java.security.Provider provider=new java.security.Provider("fixture",1.0,"host signatures"){};provider.put("MessageDigest.SHA-256",Digest.class.getName());java.security.Security.insertProviderAt(provider,1);
  String mode=args[0];
  if(mode.equals("non_system")||mode.equals("firmware")||mode.equals("initial_signer")){
   if(mode.equals("non_system"))android.os.Process.uid=2000;else if(mode.equals("firmware"))Build.FINGERPRINT="other";else pm.packages.get(PLAY).signingInfo.value=2;
   if(mode.equals("non_system")){try{install();throw new AssertionError("accepted untrusted installation");}catch(SecurityException expected){}}else{install();status("blocked");check(!JSONObject.report.optString("error_code").isEmpty(),"missing failure stage");}
   check(context.receivers.isEmpty()&&HandlerThread.live==0,"early observer");return;
  }
  if(mode.equals("gsf_rotation")){pm.packages.get(GSF).signingInfo.value=0;pm.packages.get(GSF).signingInfo.history=new byte[]{2,1,0};install();ready();closed();return;}
  if(mode.startsWith("gsf_invalid_")){SigningInfo signing=pm.packages.get(GSF).signingInfo;signing.value=0;signing.history=new byte[]{1,0};if(mode.endsWith("anchor"))pm.anchorVerified=false;else if(mode.endsWith("history"))signing.history=new byte[]{2,0};else if(mode.endsWith("current"))signing.value=2;else if(mode.endsWith("single"))signing.history=null;install();status("blocked");check(GmsProbe.calls==0&&context.receivers.isEmpty(),"invalid rotation installed");return;}
  if(mode.equals("initial_unmanaged")){pm.denied="android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE";install();Handler.drain();status("needs_google_repair");check(context.receivers.size()==1&&Handler.count()==0,"unmanaged observer missing");update(GMS);Handler.drain();status("needs_google_repair");check(GmsProbe.calls==0,"unmanaged grants");pm.denied="";update(GMS);ready();check(GmsProbe.calls==0,"read-only adoption granted");closed();return;}
  if(mode.equals("registration_failure")){context.failRegister=true;try{install();throw new AssertionError();}catch(IllegalStateException expected){}check(HandlerThread.live==0&&Handler.count()==0,"registration leak");return;}
  if(mode.equals("boot_permission_loss")){pm.denied="android.permission.DOMAIN_VERIFICATION_AGENT";install();ready();check(GmsProbe.calls==1,"authorized boot did not restore");closed();return;}
  install();ready();check(GmsProbe.calls==0,"healthy boot unexpectedly granted");
  switch(mode){
   case "preflight_upgrade_failure":pm.packages.get(GSF).signingInfo.value=2;install();status("blocked");check(context.receivers.isEmpty()&&HandlerThread.live==0&&Handler.count()==0,"old observer survived failed upgrade");check(JSONObject.report.optString("identity_package").equals(GSF),"missing package diagnostic");break;
   case "recheck":context.sendIntent(recheck());check(Handler.count()==1,"no recheck");ready();check(GmsProbe.calls==0&&JSONObject.report.optBoolean("recheck_supported"),"healthy recheck mutated");break;
   case "recheck_throttle":context.sendIntent(recheck());context.sendIntent(recheck());check(Handler.count()==1,"duplicate rechecks");ready();context.sendIntent(recheck());check(Handler.count()==0,"rate limit missing");SystemClock.elapsed=30000;context.sendIntent(recheck());check(Handler.count()==1,"rate limit never resets");ready();break;
   case "recheck_retry_budget":GmsProbe.fail=true;pm.packages.get(GMS).applicationInfo.flags=0;update(GMS);Handler.drain();status("retrying");int delays=Handler.DELAYS.size();context.sendIntent(recheck());check(Handler.DELAYS.size()==delays&&Handler.count()==1,"recheck resets retry budget");break;
   case "recheck_input":context.sendIntent(new Intent("com.stepx.companion.action.RECHECK_GOOGLE_RUNTIME"));context.sendIntent(new Intent("com.stepx.companion.action.RECHECK_GOOGLE_RUNTIME","data").setPackage("android"));Intent extra=recheck();extra.extras=new Bundle();context.sendIntent(extra);check(Handler.count()==0,"unbounded recheck input accepted");break;
   case "call_race":pm.denied="android.permission.DOMAIN_VERIFICATION_AGENT";context.sendIntent(recheck());android.telecom.TelecomManager.busy=true;Handler.drain();status("retrying");check(GmsProbe.calls==0,"active call interrupted");android.telecom.TelecomManager.busy=false;ready();check(GmsProbe.calls==1,"post call retry missing");break;
   case "unlock_retry":ServiceManager.absent=true;context.send(Intent.ACTION_USER_UNLOCKED,null);for(int i=0;i<4;i++)Handler.drain();status("needs_repair");check(Handler.count()==0,"exhaustion missing");ServiceManager.absent=false;context.send(Intent.ACTION_USER_UNLOCKED,null);ready();check(GmsProbe.calls==0,"unlock restarted healthy Google");break;
   case "unlock_permission_loss":pm.denied="android.permission.DOMAIN_VERIFICATION_AGENT";context.send(Intent.ACTION_USER_UNLOCKED,null);ready();check(GmsProbe.calls==1,"authorized unlock did not restore");break;
   case "boot":check(((Number)JSONObject.report.values.get("permission_checks")).intValue()==2,"magic threshold returned");break;
   case "gsf":update(GSF);ready();check(GmsProbe.calls==0,"healthy GSF update restarted Google");break;
   case "coalesces":update(GMS);update(PLAY);update(GSF);check(Handler.count()==1,"not coalesced");ready();check(GmsProbe.calls==0,"healthy coalesced update granted");break;
   case "unrelated":context.send("wrong",GMS);update("other");update(null);check(Handler.count()==0,"unrelated event");break;
   case "repeat_install":pm.denied="android.permission.DOMAIN_VERIFICATION_AGENT";update(GMS);install();ready();check(GmsProbe.calls==1,"install replaced update");check(context.receivers.size()==1,"duplicate listener");break;
   case "retry":GmsProbe.fail=true;pm.packages.get(GMS).applicationInfo.flags=0;update(GMS);Handler.drain();status("retrying");check(JSONObject.report.optBoolean("recovery_pending"),"no pending");GmsProbe.fail=false;ready();check(GmsProbe.calls==2,"retry missing");break;
   case "bounded":GmsProbe.fail=true;pm.packages.get(GMS).applicationInfo.flags=0;update(GMS);for(int i=0;i<4;i++)Handler.drain();status("needs_repair");check(GmsProbe.calls==4&&Handler.count()==0,"unbounded retry");check(Handler.DELAYS.subList(Handler.DELAYS.size()-4,Handler.DELAYS.size()).equals(java.util.Arrays.asList(8000L,15000L,45000L,120000L)),"bad backoff");break;
   case "stop_pending":GmsProbe.fail=true;pm.packages.get(GMS).applicationInfo.flags=0;update(GMS);Handler.drain();GoogleLoginRuntime.stopInstalled();Handler.drain();status("stopped");check(GmsProbe.calls==1,"retry after stop");break;
   case "stop_during_grant":GmsProbe.stop=true;pm.packages.get(GMS).applicationInfo.flags=0;update(GMS);Handler.drain();status("stopped");check(Handler.count()==0,"stop not cancelled");break;
   case "uninstall":update(GMS);context.send(Intent.ACTION_PACKAGE_FULLY_REMOVED,GSF);Handler.drain();status("stopped");check(GmsProbe.calls==0,"repair after removal");break;
   case "suspended":pm.packages.get(PLAY).applicationInfo.flags|=ApplicationInfo.FLAG_SUSPENDED;update(GMS);Handler.drain();status("disabled");check(GmsProbe.calls==0&&Handler.count()==0,"suspended app touched");break;
   case "lost_authorization_grant":pm.denied="android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE";update(GMS);ready();check(GmsProbe.calls==1,"previously authorized grant not restored");break;
   case "system_flag":pm.packages.get(GMS).applicationInfo.flags=0;update(GMS);ready();check(GmsProbe.calls==1,"system state not restored");break;
   case "disabled":pm.packages.get(PLAY).applicationInfo.enabled=false;update(GMS);Handler.drain();status("disabled");check(GmsProbe.calls==0&&Handler.count()==0,"disabled app modified");check(!pm.packages.get(PLAY).applicationInfo.enabled,"reenabled app");break;
   case "component_disabled":pm.service.enabled=false;update(GMS);Handler.drain();status("disabled");check(Handler.count()==0&&GmsProbe.calls==0,"disabled component touched");break;
   case "component_setting":pm.componentState=PackageManager.COMPONENT_ENABLED_STATE_DISABLED;update(GMS);Handler.drain();status("disabled");check(GmsProbe.calls==0,"disabled override touched");break;
   case "provider_declaration":pm.service.exported=false;update(GMS);Handler.drain();status("blocked");check(GmsProbe.calls==0,"invalid provider touched");break;
   case "signer":pm.packages.get(GSF).signingInfo.value=2;update(GMS);Handler.drain();status("blocked");check(GmsProbe.calls==0&&Handler.count()==0&&context.receivers.isEmpty(),"signer not hard stopped");break;
   case "multiple_signers":pm.packages.get(PLAY).signingInfo.multiple=true;update(GMS);Handler.drain();status("blocked");check(GmsProbe.calls==0,"multiple signer mutation");break;
   case "uid":pm.packages.get(GMS).applicationInfo.uid=2000;update(GMS);Handler.drain();status("blocked");check(GmsProbe.calls==0,"wrong uid mutation");break;
   case "firmware_change":Build.FINGERPRINT="other";update(GMS);Handler.drain();status("blocked");check(GmsProbe.calls==0,"wrong firmware mutation");break;
   case "denied":pm.denied="android.permission.DOMAIN_VERIFICATION_AGENT";GmsProbe.clearDenied=false;update(GMS);Handler.drain();status("retrying");check(!JSONObject.report.optBoolean("permissions_verified"),"false permission success");break;
   case "new_declaration":pm.packages.get(PLAY).requestedPermissions=new String[]{"android.permission.MANAGE_USERS"};pm.denied="android.permission.MANAGE_USERS";GmsProbe.clearDenied=false;update(PLAY);Handler.drain();status("retrying");check(JSONObject.report.optString("permission_result").contains("check com.android.vending android.permission.MANAGE_USERS=-1"),"new grant not checked");break;
   case "platform_skip":pm.packages.get(PLAY).requestedPermissions=new String[]{"android.permission.MANAGE_USERS"};pm.unavailable="android.permission.MANAGE_USERS";update(PLAY);ready();check(JSONObject.report.optString("permission_result").contains("SKIP unavailable android.permission.MANAGE_USERS"),"platform availability ignored");break;
   case "owner":pm.owner="other";update(GMS);Handler.drain();status("blocked");check(Handler.count()==0&&GmsProbe.calls==0,"bad owner touched");break;
   case "missing_package":pm.absent=GSF;update(GMS);Handler.drain();status("retrying");check(GmsProbe.calls==0,"incomplete suite mutated");pm.absent="";ready();break;
   case "package_changed":GmsProbe.change=true;pm.packages.get(GMS).applicationInfo.flags=0;update(GMS);Handler.drain();status("retrying");break;
   case "same_version_reinstall":update(GMS);ready();int calls=GmsProbe.calls;pm.packages.get(GMS).lastUpdateTime++;update(GMS);ready();check(GmsProbe.calls==calls,"healthy reinstall restarted Google");check(((Number)JSONObject.report.values.get("gms_updated_at")).longValue()==pm.packages.get(GMS).lastUpdateTime,"same version reinstall stamp stale");break;
   case "provider_retry":ServiceManager.absent=true;update(GMS);Handler.drain();status("retrying");ServiceManager.absent=false;ready();check(GmsProbe.calls==0,"provider retry restarted Google");break;
   default:throw new AssertionError(mode);
  }
  closed();
 }
}''',
}


class ReviewedPermissionParity(unittest.TestCase):
    def test_readback_coverage_is_exact_original_grant_allowlist(self):
        source = (ROOT / 'handoff/google-play-stepx/java/GmsProbe.java').read_text()
        groups = re.search(r'String\[\]\[\] perms=\{\{(.*?)\},\{(.*?)\}\};', source).groups()
        expected = [re.findall(r'"([^"]+)"', group) for group in groups]
        runtime = (ROOT / 'desktop/companion-permissions/GoogleLoginRuntime.java').read_text()
        source = runtime.split('REVIEWED_PERMISSIONS = {', 1)[1].split('    };', 1)[0]
        actual = [re.findall(r'"([^"]+)"', group) for group in re.findall(r'\{(.*?)\}', source, re.S)]
        self.assertEqual(actual, expected)
        self.assertEqual([len(group) for group in actual], [120, 36])


@unittest.skipUnless(JAVAC and JAVA, 'A JDK is required')
class GoogleLoginObserver(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temp.cleanup)
        directory = Path(cls.temp.name)
        sources = []
        for relative, source in STUBS.items():
            target = directory / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(source)
            sources.append(str(target))
        sources.append(str(ROOT / 'desktop/companion-permissions/GoogleLoginRuntime.java'))
        cls.classes = directory / 'classes'
        result = subprocess.run([JAVAC, '--release', '11', '-d', str(cls.classes), *sources], capture_output=True, text=True)
        if result.returncode:
            raise AssertionError(result.stdout + result.stderr)

    def test_native_event_and_recovery_boundaries(self):
        for case in ('boot', 'gsf', 'coalesces', 'unrelated', 'repeat_install', 'retry', 'bounded',
                     'stop_pending', 'stop_during_grant', 'uninstall', 'disabled', 'component_disabled',
                     'gsf_rotation', 'gsf_invalid_anchor', 'gsf_invalid_history', 'gsf_invalid_current', 'gsf_invalid_single', 'preflight_upgrade_failure', 'recheck', 'recheck_throttle', 'recheck_retry_budget', 'recheck_input', 'call_race', 'boot_permission_loss', 'unlock_retry', 'unlock_permission_loss', 'initial_unmanaged', 'suspended', 'lost_authorization_grant', 'system_flag', 'component_setting', 'provider_declaration', 'signer', 'multiple_signers', 'uid', 'firmware_change', 'denied', 'new_declaration',
                     'platform_skip', 'owner', 'missing_package', 'package_changed', 'same_version_reinstall',
                     'provider_retry', 'non_system', 'firmware', 'initial_signer', 'registration_failure'):
            with self.subTest(case=case):
                result = subprocess.run([JAVA, '-cp', str(self.classes), 'stepx.GoogleHarness', case],
                                        capture_output=True, text=True, timeout=15)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
