"""Run the real Java cleanup against a deterministic in-process SDK double.

Each case is a fresh JVM. The doubles model ownership identities and disposal,
not Android privileges or actual Binder/display behavior; real-device proof is
still required. This catches accidental broad finish/map clearing and false
success when a display remains after the original callback returns.
"""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
JAVA = Path('/home/stepx-user/.local/opt/android-studio/jbr/bin')
SOURCES = {
'android/os/Looper.java': '''package android.os; public class Looper { static final Looper MAIN=new Looper(); public static Looper getMainLooper(){return MAIN;} public static Looper myLooper(){return MAIN;} }''',
'android/os/Handler.java': '''package android.os; public class Handler { public final java.util.Set<Runnable> callbacks=new java.util.HashSet<>(); public Looper getLooper(){return Looper.getMainLooper();} public boolean hasCallbacks(Runnable r){return callbacks.contains(r);} public void removeCallbacks(Runnable r){callbacks.remove(r);} }''',
'android/os/Build.java': '''package android.os; public class Build { public static class VERSION { public static int SDK_INT=36; } }''',
'android/os/Process.java': '''package android.os; public class Process { public static int myUid(){return 1000;} public static int myPid(){return 100;} }''',
'android/os/Binder.java': '''package android.os; public class Binder { public static int getCallingUid(){return 1000;} }''',
'android/os/SystemClock.java': '''package android.os; public class SystemClock { public static long elapsedRealtime(){return 10000;} }''',
'android/content/pm/ApplicationInfo.java': '''package android.content.pm; public class ApplicationInfo { public int uid=1000; }''',
'android/content/Context.java': '''package android.content; public class Context { public Context getApplicationContext(){return this;} public String getPackageName(){return "com.stepos.ability";} public android.content.pm.ApplicationInfo getApplicationInfo(){return new android.content.pm.ApplicationInfo();} public ClassLoader getClassLoader(){return getClass().getClassLoader();} public <T> T getSystemService(Class<T> c){return c.cast(android.hardware.display.DisplayManager.INSTANCE);} }''',
'android/view/Display.java': '''package android.view; public class Display { public static final int DEFAULT_DISPLAY=0; public final int id; public int owner=1000,type=5; public Display(int i){id=i;} public int getDisplayId(){return id;} public int getOwnerUid(){return owner;} public int getType(){return type;} public boolean isValid(){return android.hardware.display.DisplayManager.INSTANCE.getDisplay(id)!=null;} }''',
'android/hardware/display/DisplayManager.java': '''package android.hardware.display; public class DisplayManager { public static final DisplayManager INSTANCE=new DisplayManager(); public final java.util.Map<Integer,android.view.Display> displays=new java.util.HashMap<>(); public android.view.Display getDisplay(int i){return displays.get(i);} }''',
'android/hardware/display/VirtualDisplay.java': '''package android.hardware.display; public class VirtualDisplay { public final android.view.Display display; public VirtualDisplay(android.view.Display d){display=d;} public android.view.Display getDisplay(){return display;} }''',
'org/json/JSONObject.java': '''package org.json; public class JSONObject { public static final Object NULL=new Object(); public final java.util.Map<String,Object> data=new java.util.HashMap<>(); public JSONObject put(String k,Object v){data.put(k,v);return this;} public boolean optBoolean(String k){return Boolean.TRUE.equals(data.get(k));} public Object get(String k){return data.get(k);} }''',
'org/json/JSONArray.java': '''package org.json; public class JSONArray { final java.util.List<Object> data=new java.util.ArrayList<>(); public JSONArray put(Object o){data.add(o);return this;} public int length(){return data.size();} public Object get(int i){return data.get(i);} }''',
'w9/a.java': '''package w9; public class a { public boolean locked; public boolean tryLock(Object o){if(locked)return false;locked=true;return true;} public void unlock(Object o){if(!locked)throw new AssertionError();locked=false;} public boolean isLocked(){return locked;} }''',
'com/mdisdk/TaskManager.java': '''package com.mdisdk; public class TaskManager { public static android.content.Context applicationContext; public static Object displayManager=new Object(); public static final java.util.concurrent.ConcurrentHashMap<String,Object> activeTasks=new java.util.concurrent.ConcurrentHashMap<>(); public static int releases; public static void release(){if(!activeTasks.isEmpty())throw new AssertionError("broad release");releases++;applicationContext=null;displayManager=null;} }''',
'com/mdisdk/display/OmniDisplay.java': '''package com.mdisdk.display; public class OmniDisplay { public android.view.Display targetDisplay; public android.hardware.display.VirtualDisplay virtualDisplay; public Object renderPipeline; public OmniDisplay(int id){targetDisplay=new android.view.Display(id);virtualDisplay=new android.hardware.display.VirtualDisplay(targetDisplay);android.hardware.display.DisplayManager.INSTANCE.displays.put(id,targetDisplay);} }''',
'com/mdisdk/core/TaskContext.java': '''package com.mdisdk.core; public class TaskContext { final android.content.Context applicationContext=com.mdisdk.TaskManager.applicationContext; final com.mdisdk.display.OmniDisplay display; public TaskContext(int id){display=new com.mdisdk.display.OmniDisplay(id);} public android.view.Display getDisplay(){return display.targetDisplay;} public com.mdisdk.display.OmniDisplay getOmniForTest(){return display;} }''',
'com/mdisdk/core/DefaultTaskImpl.java': '''package com.mdisdk.core; public class DefaultTaskImpl { public final String name; public final int id; public final TaskContext taskContext; public boolean isFinishing, retainDisplay; public int finishes; public final java.util.Map<Object,Object> executionJobs=new java.util.HashMap<>(); public final java.util.concurrent.atomic.AtomicInteger activeExecutions=new java.util.concurrent.atomic.AtomicInteger(); public final w9.a finishMutex=new w9.a(); public DefaultTaskImpl(String n,int i){name=n;id=i;taskContext=new TaskContext(i);} public String getName(){return name;} public int getDisplayId(){return id;} public void finish(boolean ignore){finishes++;isFinishing=true;com.mdisdk.TaskManager.activeTasks.remove(name,this);if(!retainDisplay)android.hardware.display.DisplayManager.INSTANCE.displays.remove(id);} }''',
'com/stepos/taskrobot/llmapi/stepfun/MainDisplayWatcher.java': '''package com.stepos.taskrobot.llmapi.stepfun; public class MainDisplayWatcher { public final Object stateLock=new Object(); public final android.os.Handler handler=new android.os.Handler(); public com.mdisdk.core.DefaultTaskImpl watchTask; public String state="IDLE", observerRegId; public Runnable settleTimeout; public int resets; public void reset(){synchronized(stateLock){resets++; if(settleTimeout!=null)handler.removeCallbacks(settleTimeout);if(watchTask!=null)watchTask.finish(true);watchTask=null;observerRegId=null;settleTimeout=null;state="IDLE";}} }''',
'com/stepos/taskrobot/llmapi/stepfun/SystemSdkBridge.java': '''package com.stepos.taskrobot.llmapi.stepfun; public class SystemSdkBridge { public static android.content.Context appContext; public static final java.util.Map<String,Object> tasks=new java.util.HashMap<>(); public static com.mdisdk.core.DefaultTaskImpl queryTask; public static final w9.a queryTaskMutex=new w9.a(); public static final MainDisplayWatcher mainDisplayWatcher=new MainDisplayWatcher(); public static final android.os.Handler mainHandler=new android.os.Handler(); public static final Runnable queryTaskIdleRunnable=()->{if(queryTask!=null)queryTask.finish(true);queryTask=null;}; }''',
'stepx/NativeGuiTaskAdapter.java': '''package stepx; public class NativeGuiTaskAdapter { public static NativeGuiTaskAdapter hostedAdapter=new NativeGuiTaskAdapter(); public android.content.Context app; public boolean running=false,shutdownRequested=true; }''',
'stepx/CleanupHarness.java': '''package stepx;
import android.content.Context;
import android.hardware.display.DisplayManager;
import com.mdisdk.TaskManager;
import com.mdisdk.core.DefaultTaskImpl;
import com.stepos.taskrobot.llmapi.stepfun.SystemSdkBridge;
import com.stepos.taskrobot.llmapi.stepfun.MainDisplayWatcher;
import org.json.JSONObject;
public class CleanupHarness {
 static final String Q="__query_task__",W="__main_display_watcher__";
 static void check(boolean v){if(!v)throw new AssertionError();}
 static DefaultTaskImpl add(String key,int id){DefaultTaskImpl t=new DefaultTaskImpl(key,id);TaskManager.activeTasks.put(key,t);return t;}
 public static void main(String[] args)throws Exception{
  String mode=args[0]; Context host=new Context(); TaskManager.applicationContext=host;
  SystemSdkBridge.appContext=host; NativeGuiTaskAdapter.hostedAdapter.app=host;
  android.view.Display physical=new android.view.Display(0);physical.type=1;DisplayManager.INSTANCE.displays.put(0,physical);
  MainDisplayWatcher watcher=SystemSdkBridge.mainDisplayWatcher;
  DefaultTaskImpl watch=null,query=null;
  if(!mode.equals("empty")&&!mode.equals("query")){
   watch=add(W,90000001);watcher.watchTask=watch;watcher.state="WATCHING";
   watcher.observerRegId="local-observer";watcher.settleTimeout=()->{};watcher.handler.callbacks.add(watcher.settleTimeout);
  }
  if(mode.equals("both")||mode.equals("query")){
   query=add(Q,90000002);SystemSdkBridge.queryTask=query;
   SystemSdkBridge.mainHandler.callbacks.add(SystemSdkBridge.queryTaskIdleRunnable);
  }
  String reject=null;
  switch(mode){
   case "unknown":add("other-task",90000003);reject="task_scope_mismatch";break;
   case "replaced":watcher.watchTask=new DefaultTaskImpl(W,90000004);reject="task_scope_mismatch";break;
   case "wrong_owner":watch.taskContext.getOmniForTest().targetDisplay.owner=1001;reject="display_owner_mismatch";break;
   case "virtual_mismatch":watch.taskContext.getOmniForTest().virtualDisplay=new android.hardware.display.VirtualDisplay(new android.view.Display(90000008));reject="owned_virtual_display_mismatch";break;
   case "target_mismatch":watch.taskContext.getOmniForTest().targetDisplay=physical;reject="owned_virtual_display_mismatch";break;
   case "physical":TaskManager.activeTasks.clear();watch=new DefaultTaskImpl(W,0);TaskManager.activeTasks.put(W,watch);watcher.watchTask=watch;reject="virtual_display_required";break;
   case "busy":watch.activeExecutions.set(1);reject="executions_active";break;
   case "jobs":watch.executionJobs.put("owned",new Object());reject="task_not_idle";break;
   case "finishing":watch.isFinishing=true;reject="task_not_idle";break;
   case "native_active":SystemSdkBridge.tasks.put("active",new Object());reject="native_tasks_active";break;
   case "adapter_open":NativeGuiTaskAdapter.hostedAdapter.running=true;reject="adapter_not_closed";break;
   case "retain_display":watch.retainDisplay=true;break;
   case "mutex_busy":SystemSdkBridge.queryTaskMutex.locked=true;break;
  }
  if(reject!=null){
   boolean rejected=false;
   try{NativeGuiQueryCleanup.cleanup(host,7000);}catch(Exception expected){rejected=expected.getMessage().contains(reject);}
   check(rejected);check(TaskManager.releases==0);check(watcher.resets==0);check(watch.finishes==0);
   check(!TaskManager.activeTasks.isEmpty());System.out.println("guard-ok");return;
  }
  JSONObject before=NativeGuiTaskResources.registrySnapshot(host);check(before.optBoolean("ok"));
  // This getter intentionally returns the Android target Display. The real DEX
  // also has a same-name OmniDisplay-returning overload. Cleanup must not use it.
  if(watch!=null){
   check(watch.taskContext.getClass().getMethod("getDisplay").getReturnType()==android.view.Display.class);
   org.json.JSONArray tasks=(org.json.JSONArray)before.get("tasks");
   boolean found=false;
   for(int i=0;i<tasks.length();i++){
    JSONObject detail=(JSONObject)tasks.get(i);
    if(!"main_display_watcher".equals(detail.get("category")))continue;
    found=true;check(detail.optBoolean("has_target_display_accessor"));
    check(detail.optBoolean("omni_field_declared_type_matches"));check(detail.optBoolean("omni_field_value_type_matches"));
    check(detail.optBoolean("task_context_owns_host"));check(detail.optBoolean("target_matches_task"));check(detail.optBoolean("virtual_matches_task"));
    check(detail.get("target_display_id").equals(90000001));check(detail.get("virtual_display_id").equals(90000001));
   }
   check(found);
  }
  JSONObject result=NativeGuiQueryCleanup.cleanup(host,7000);
  if(mode.equals("mutex_busy")){check(!result.optBoolean("complete"));check(watcher.resets==0);check(TaskManager.releases==0);System.out.println("pending-ok");return;}
  if(mode.equals("retain_display")){
   check(!result.optBoolean("complete"));check(TaskManager.releases==0);check(watch.finishes==1);
   check(TaskManager.activeTasks.isEmpty());check(DisplayManager.INSTANCE.getDisplay(90000001)!=null);
   result=NativeGuiQueryCleanup.cleanup(host,7000);check(!result.optBoolean("complete"));check(watch.finishes==1);
   DisplayManager.INSTANCE.displays.remove(90000001);
   result=NativeGuiQueryCleanup.cleanup(host,7000);check(result.optBoolean("complete"));check(watch.finishes==1);
  }
  check(result.optBoolean("complete"));check(TaskManager.activeTasks.isEmpty());check(TaskManager.releases==1);
  check(DisplayManager.INSTANCE.getDisplay(0)!=null);
  if(watch!=null){check(watch.finishes==1);check(watcher.resets==1);check(watcher.observerRegId==null);check(watcher.settleTimeout==null);}
  if(query!=null)check(query.finishes==1);
  result=NativeGuiQueryCleanup.cleanup(host,7000);check(result.optBoolean("complete"));check(TaskManager.releases==1);
  check(NativeGuiTaskResources.registrySnapshot(host).optBoolean("ok"));System.out.println("cleanup-ok");
 }
}'''
}


@unittest.skipUnless((JAVA / 'javac').exists(), 'Local Android JDK unavailable')
class NativeHelperCleanupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory()
        root = Path(cls.directory.name)
        sources = []
        for name, contents in SOURCES.items():
            path = root / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_text(contents)
            sources.append(str(path))
        sources.extend(str(ROOT / 'desktop/gui-native' / name) for name in ('NativeGuiQueryCleanup.java', 'NativeGuiTaskResources.java'))
        cls.classes = root / 'classes'; cls.classes.mkdir()
        subprocess.run([str(JAVA / 'javac'), '--release', '11', '-d', str(cls.classes), *sources], check=True, capture_output=True, text=True)

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def run_case(self, case):
        return subprocess.run([str(JAVA / 'java'), '-cp', str(self.classes), 'stepx.CleanupHarness', case],
                              check=True, capture_output=True, text=True, timeout=10).stdout.strip()

    def test_known_helpers_use_original_callbacks_once_and_preserve_display_zero(self):
        for case in ('empty', 'watcher', 'query', 'both'):
            with self.subTest(case=case):
                self.assertEqual(self.run_case(case), 'cleanup-ok')

    def test_unknown_replaced_busy_or_unowned_resources_are_never_finished(self):
        for case in ('unknown', 'replaced', 'wrong_owner', 'virtual_mismatch', 'target_mismatch', 'physical', 'busy', 'jobs', 'finishing', 'native_active', 'adapter_open'):
            with self.subTest(case=case):
                self.assertEqual(self.run_case(case), 'guard-ok')

    def test_map_removal_is_insufficient_until_actual_virtual_display_disappears(self):
        self.assertEqual(self.run_case('retain_display'), 'cleanup-ok')

    def test_locked_query_creation_mutex_is_pending_without_mutation(self):
        self.assertEqual(self.run_case('mutex_busy'), 'pending-ok')


if __name__ == '__main__':
    unittest.main()
