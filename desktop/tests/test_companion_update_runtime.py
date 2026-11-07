"""Execute the real package observer against a deterministic Android event loop.

This covers event scope and lifecycle, not Android permission implementation;
CompanionGrant retains the existing identity/owner/baseline validation.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
JDK = Path(os.environ.get('STEPX_JDK', '/home/stepx-user/.local/opt/android-studio/jbr')) / 'bin'
JAVAC = str(JDK / 'javac') if (JDK / 'javac').is_file() else shutil.which('javac')
JAVA = str(JDK / 'java') if (JDK / 'java').is_file() else shutil.which('java')
STUBS = {
    'android/net/Uri.java': '''package android.net; public class Uri { final String p; public Uri(String p){this.p=p;} public String getSchemeSpecificPart(){return p;} }''',
    'android/content/Intent.java': '''package android.content; public class Intent { public static final String ACTION_PACKAGE_REPLACED="replaced",ACTION_PACKAGE_FULLY_REMOVED="removed"; final String action; final android.net.Uri data; public Intent(String a,String p){action=a;data=p==null?null:new android.net.Uri(p);} public String getAction(){return action;} public android.net.Uri getData(){return data;} }''',
    'android/content/IntentFilter.java': '''package android.content; public class IntentFilter { public final java.util.Set<String> actions=new java.util.HashSet<>(); public String scheme; public IntentFilter(String a){actions.add(a);} public void addAction(String a){actions.add(a);} public void addDataScheme(String s){scheme=s;} }''',
    'android/content/BroadcastReceiver.java': '''package android.content; public abstract class BroadcastReceiver { public abstract void onReceive(Context c,Intent i); }''',
    'android/content/pm/ApplicationInfo.java': '''package android.content.pm; public class ApplicationInfo { public boolean enabled=true; }''',
    'android/content/pm/PackageManager.java': '''package android.content.pm; public class PackageManager { public final ApplicationInfo app=new ApplicationInfo(); public ApplicationInfo getApplicationInfo(String p,int flags){if(!p.equals("com.stepx.companion.app"))throw new AssertionError(p);return app;} }''',
    'android/content/Context.java': '''package android.content; public class Context { public static final int RECEIVER_NOT_EXPORTED=4; public boolean failRegister; public final java.util.List<BroadcastReceiver> receivers=new java.util.ArrayList<>(); final android.content.pm.PackageManager pm=new android.content.pm.PackageManager(); public android.content.pm.PackageManager getPackageManager(){return pm;} public void registerReceiver(BroadcastReceiver r,IntentFilter f,String permission,android.os.Handler handler,int flags){if(f.actions.size()!=2||!f.actions.contains("replaced")||!f.actions.contains("removed")||!"package".equals(f.scheme)||flags!=RECEIVER_NOT_EXPORTED)throw new AssertionError("unbounded receiver");if(failRegister)throw new IllegalStateException("registration failed");receivers.add(r);} public void unregisterReceiver(BroadcastReceiver r){if(!receivers.remove(r))throw new AssertionError("not registered");} public void send(String action,String p){for(BroadcastReceiver r:new java.util.ArrayList<>(receivers))r.onReceive(this,new Intent(action,p));} }''',
    'android/app/ActivityThread.java': '''package android.app; public class ActivityThread { public static final android.content.Context CONTEXT=new android.content.Context(); public static ActivityThread currentActivityThread(){return new ActivityThread();} public android.content.Context getSystemContext(){return CONTEXT;} }''',
    'android/os/Looper.java': '''package android.os; public class Looper {}''',
    'android/os/HandlerThread.java': '''package android.os; public class HandlerThread { public static int live; public HandlerThread(String n,int p){} public void start(){live++;} public Looper getLooper(){return new Looper();} public void quitSafely(){live--;} }''',
    'android/os/Handler.java': '''package android.os; public class Handler { public static final java.util.List<Handler> ALL=new java.util.ArrayList<>(); final java.util.Set<Runnable> pending=new java.util.LinkedHashSet<>(); public Handler(Looper l){ALL.add(this);} public void removeCallbacks(Runnable r){pending.remove(r);} public void postDelayed(Runnable r,long delay){if(delay!=2000)throw new AssertionError("unexpected delay");pending.add(r);} public void removeCallbacksAndMessages(Object token){pending.clear();} public static void drain(){for(Handler h:new java.util.ArrayList<>(ALL))for(Runnable r:new java.util.ArrayList<>(h.pending)){h.pending.remove(r);r.run();}} }''',
    'android/os/Process.java': '''package android.os; public class Process { public static final int THREAD_PRIORITY_BACKGROUND=10; public static int uid=1000; public static int myUid(){return uid;} }''',
    'android/os/Binder.java': '''package android.os; public class Binder { public static int cleared; public static long clearCallingIdentity(){cleared++;return 42;} public static void restoreCallingIdentity(long token){if(token!=42)throw new AssertionError();cleared--;} }''',
    'android/util/Log.java': '''package android.util; public class Log { public static int errors; public static void i(String t,String m){} public static void w(String t,String m){errors++;} }''',
    'org/json/JSONObject.java': '''package org.json; public class JSONObject { final java.util.Map<String,Object> values=new java.util.HashMap<>(); public JSONObject(){} public JSONObject(String raw){values.put("status",raw);values.put("all_granted",raw.equals("success"));} public JSONObject put(String k,Object v){values.put(k,v);return this;} public String optString(String k){return optString(k,"");} public String optString(String k,String fallback){return (String)values.getOrDefault(k,fallback);} public boolean optBoolean(String k){return Boolean.TRUE.equals(values.get(k));} }''',
    'stepx/CompanionGrant.java': '''package stepx; public class CompanionGrant { public static int grants; public static boolean fail; static String restoreAfterUpdate(String s,String baseline,int uid){if(!s.equals("pinned")||!baseline.equals("durable-baseline")||uid!=10218)throw new AssertionError("identity or recovery baseline changed");grants++;return fail?"failed":"success";} }''',
    'stepx/PairingDaemon.java': '''package stepx; public class PairingDaemon { public static int starts; public static void installSystem(String signer,int uid){if(!signer.equals("pinned")||uid!=10218)throw new AssertionError();starts++;} }''',
    'stepx/UpdateHarness.java': '''package stepx;
import android.content.*;
import android.os.*;
public class UpdateHarness {
 static final String PACKAGE="com.stepx.companion.app";
 static final Context context=android.app.ActivityThread.CONTEXT;
 static void check(boolean v){if(!v)throw new AssertionError();}
 static void install()throws Exception{CompanionUpdateRuntime.install("pinned",10218,"durable-baseline");}
 static void update(){context.send(Intent.ACTION_PACKAGE_REPLACED,PACKAGE);}
 public static void main(String[] args)throws Exception{
  String mode=args[0];
  if(mode.equals("non_system")){
   android.os.Process.uid=2000;
   try{install();throw new AssertionError();}catch(SecurityException expected){}
   check(context.receivers.isEmpty()&&HandlerThread.live==0);return;
  }
  if(mode.equals("registration_failure")){
   context.failRegister=true;
   try{install();throw new AssertionError();}catch(IllegalStateException expected){}
   check(context.receivers.isEmpty()&&HandlerThread.live==0);return;
  }
  install();Handler.drain();check(CompanionGrant.grants==0);
  int expected=0;
  switch(mode){
   case "update": update();expected=1;break;
   case "coalesces": update();update();expected=1;break;
   case "unrelated": context.send(Intent.ACTION_PACKAGE_REPLACED,"com.other.app");context.send(Intent.ACTION_PACKAGE_REPLACED,null);break;
   case "wrong_action": context.send("manual-revoke",PACKAGE);break;
   case "disabled": context.getPackageManager().app.enabled=false;update();break;
   case "revoke": update();CompanionUpdateRuntime.stopInstalled();update();break;
   case "uninstall": update();context.send(Intent.ACTION_PACKAGE_FULLY_REMOVED,PACKAGE);update();break;
   case "replace_monitor": update();install();check(context.receivers.size()==1&&HandlerThread.live==1);update();expected=1;break;
   case "grant_failure": CompanionGrant.fail=true;update();expected=1;break;
   default: throw new AssertionError(mode);
  }
  Handler.drain();check(CompanionGrant.grants==expected);check(Binder.cleared==0);
  check(PairingDaemon.starts==(mode.equals("grant_failure")?0:expected));
  if(mode.equals("grant_failure"))check(android.util.Log.errors==1);
  CompanionUpdateRuntime.stopInstalled();Handler.drain();
  check(context.receivers.isEmpty()&&HandlerThread.live==0);
 }
}''',
}


@unittest.skipUnless(JAVAC and JAVA, 'A JDK is required')
class CompanionUpdateObserver(unittest.TestCase):
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
        sources.append(str(ROOT / 'desktop/companion-permissions/CompanionUpdateRuntime.java'))
        cls.classes = directory / 'classes'
        subprocess.run([JAVAC, '--release', '11', '-d', str(cls.classes), *sources],
                       check=True, capture_output=True, text=True)

    def test_package_update_and_revocation_boundaries(self):
        for case in ('update', 'coalesces', 'unrelated', 'wrong_action', 'disabled',
                     'revoke', 'uninstall', 'replace_monitor', 'grant_failure',
                     'non_system', 'registration_failure'):
            with self.subTest(case=case):
                result = subprocess.run([JAVA, '-cp', str(self.classes), 'stepx.UpdateHarness', case],
                                        capture_output=True, text=True, timeout=15)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
