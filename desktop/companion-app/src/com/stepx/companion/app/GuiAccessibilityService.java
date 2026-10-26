package com.stepx.companion.app;

import android.accessibilityservice.AccessibilityService;
import android.accessibilityservice.GestureDescription;
import android.app.KeyguardManager;
import android.graphics.*;
import android.os.*;
import android.view.accessibility.*;
import org.json.*;
import java.io.*;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Bounded screenshot/action executor shared by native MCP and independent planners. */
public final class GuiAccessibilityService extends AccessibilityService {
    static volatile GuiAccessibilityService instance;
    static volatile long lastUserTouchRealtimeMs;
    static volatile long lastSyntheticActionRealtimeMs;
    private volatile JSONArray observedNodes;
    private volatile String observedWindow = "";
    private final ExecutorService screenshotExecutor = Executors.newSingleThreadExecutor();
    final GuiTaskState task = new GuiTaskState();
    final Handler main = new Handler(Looper.getMainLooper());
    private final Runnable watchdog = new Runnable() { public void run() {
        task.expire(SystemClock.elapsedRealtime());
        main.postDelayed(this, task.active() ? 1000L : 10000L);
    }};
    private void scheduleWatchdog() {
        main.removeCallbacks(watchdog);
        main.postDelayed(watchdog, task.active() ? 1000L : 10000L);
    }
    @Override protected void onServiceConnected() { instance=this; scheduleWatchdog(); }
    @Override public void onAccessibilityEvent(AccessibilityEvent event) {
        if (event == null) return;
        int type = event.getEventType();
        if (type == AccessibilityEvent.TYPE_TOUCH_INTERACTION_START
                || type == AccessibilityEvent.TYPE_GESTURE_DETECTION_START
                || type == AccessibilityEvent.TYPE_VIEW_CLICKED
                || type == AccessibilityEvent.TYPE_VIEW_SCROLLED) {
            long now = SystemClock.elapsedRealtime();
            if (now - lastSyntheticActionRealtimeMs > 1200L && !task.active()) {
                lastUserTouchRealtimeMs = now;
            }
        }
    }
    public static void markSyntheticAction() {
        lastSyntheticActionRealtimeMs = SystemClock.elapsedRealtime();
    }
    public static void recordUserTouch(long realtimeMs) {
        lastUserTouchRealtimeMs = realtimeMs;
    }
    public static boolean isHumanTouchingRecently(long windowMs) {
        return isHumanTouchingRecently(SystemClock.elapsedRealtime(), windowMs);
    }
    public static boolean isHumanTouchingRecently(long nowRealtimeMs, long windowMs) {
        long last = lastUserTouchRealtimeMs;
        return last > 0L && nowRealtimeMs >= last && (nowRealtimeMs - last) < windowMs;
    }
    @Override public void onInterrupt() { if(task.active()) task.stop(task.id,"系统中断了操作服务"); }
    @Override public void onDestroy() { if(instance==this)instance=null; main.removeCallbacksAndMessages(null); screenshotExecutor.shutdownNow(); super.onDestroy(); }
    JSONObject status() throws JSONException {
        task.expire(SystemClock.elapsedRealtime());
        return new JSONObject().put("ok",true).put("ready",true).put("task",task.id)
            .put("phase",task.phase).put("goal",task.goal).put("detail",task.detail)
            .put("epoch",task.epoch).put("actions",task.actions).put("userPaused",task.userPaused).put("userStopped",task.userStopped)
            .put("lastUserTouchRealtimeMs",lastUserTouchRealtimeMs).put("humanTouching",isHumanTouchingRecently(10_000L));
    }
    void unlocked() {
        if(getSystemService(KeyguardManager.class).isKeyguardLocked() || !getSystemService(PowerManager.class).isInteractive())
            throw new IllegalStateException("请手动解锁并点亮手机屏幕");
    }
    void request(JSONObject q, CompletableFuture<JSONObject> answer) {
        try {
            long now=SystemClock.elapsedRealtime(); String op=q.getString("op"), id=q.optString("task");
            if(op.equals("status")){answer.complete(status());return;}
            if(op.equals("user_pause")){task.userPause(id,now);PhoneGuiAgent.controlChanged("pause");answer.complete(status());return;}
            if(op.equals("user_resume")){unlocked();task.userContinue(id,now);scheduleWatchdog();PhoneGuiAgent.controlChanged("resume");answer.complete(status());return;}
            if(op.equals("user_stop")){task.userStop(id);scheduleWatchdog();PhoneGuiAgent.controlChanged("stop");new File(getExternalFilesDir(null),"gui-screen.png").delete();File[] captures=getExternalFilesDir(null).listFiles((dir,name)->name.matches("mcp-screen-[a-f0-9]{32}\\.jpg"));if(captures!=null)for(File file:captures)file.delete();answer.complete(status());return;}
            if(op.equals("start")){unlocked();task.start(id,q.getString("goal"),now);scheduleWatchdog();answer.complete(status());return;}
            if(op.equals("pause")){task.pause(id,now);scheduleWatchdog();answer.complete(status());return;}
            if(op.equals("resume")){unlocked();task.resume(id,now);scheduleWatchdog();answer.complete(status());return;}
            if(op.equals("stop")){task.stop(id,"已停止操作");scheduleWatchdog();new File(getExternalFilesDir(null),"gui-screen.png").delete();answer.complete(status());return;}
            if(op.equals("heartbeat")){task.renew(id,now);answer.complete(status());return;}
            task.require(id,now); unlocked();
            if(!task.phase.equals("running"))throw new IllegalStateException("任务已暂停");
            if(op.equals("observe")){observe(answer);return;}
            if(!op.equals("action"))throw new IllegalArgumentException("未知操作");
            JSONObject a=q.getJSONObject("action"); String kind=a.getString("type");
            if(!java.util.Arrays.asList("click","swipe","long_press","input","back","home","launch").contains(kind))throw new IllegalArgumentException("不支持的动作");
            JSONArray liveNodes=new JSONArray();collect(getRootInActiveWindow(),liveNodes,0);
            // Launch/home do not target a screen element. Clock/widgets changing must not
            // invalidate them; task/epoch, unlock and human stop checks still apply below.
            boolean targetsCurrentScreen=!kind.equals("launch")&&!kind.equals("home");
            if(targetsCurrentScreen&&!q.optString("treeHash").equals(Integer.toHexString(liveNodes.toString().hashCode()))) {
                android.util.DisplayMetrics metrics=getResources().getDisplayMetrics();
                boolean stableTarget=false;
                if((kind.equals("click")||kind.equals("long_press")) && observedNodes!=null && windowIdentity().equals(observedWindow)) {
                    int x=(int)coord(a,"x",metrics.widthPixels), y=(int)coord(a,"y",metrics.heightPixels);
                    String before=hitTarget(observedNodes,x,y), after=hitTarget(liveNodes,x,y);
                    stableTarget=!before.isEmpty() && before.equals(after);
                }
                if(!stableTarget)throw new IllegalStateException("界面内容已改变，请重新观察");
            }
            task.action(id,q.getLong("epoch"),now);
            if(kind.equals("launch")){
                String pkg=a.getString("package");
                if(!pkg.matches("[a-zA-Z][a-zA-Z0-9_]*(\\.[a-zA-Z][a-zA-Z0-9_]*)+"))throw new IllegalArgumentException("应用包名无效");
                android.content.Intent intent=getPackageManager().getLaunchIntentForPackage(pkg);
                if(intent==null)throw new IllegalStateException("应用未安装或没有启动入口，请从桌面打开");
                intent.addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK);startActivity(intent);answer.complete(status());return;
            }
            if(kind.equals("input")){
                String text=a.getString("text"); if(text.length()>4000)throw new IllegalArgumentException("输入过长");
                AccessibilityNodeInfo root=getRootInActiveWindow();
                AccessibilityNodeInfo focus=root==null?null:root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT);
                if(focus==null || !focus.isEditable() || focus.isPassword())throw new IllegalStateException("当前没有可编辑的普通输入框，请先点击输入框");
                Bundle args=new Bundle();args.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,text);
                if(!focus.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT,args))throw new IllegalStateException("输入框未接受文字");
                answer.complete(status());return;
            }
            if(kind.equals("back")||kind.equals("home")){
                if(!performGlobalAction(kind.equals("back")?GLOBAL_ACTION_BACK:GLOBAL_ACTION_HOME))throw new IllegalStateException("系统未执行导航");
                answer.complete(status());return;
            }
            android.util.DisplayMetrics dm=getResources().getDisplayMetrics();
            Path path=new Path();
            if(kind.equals("swipe")){
                path.moveTo(coord(a,"x1",dm.widthPixels),coord(a,"y1",dm.heightPixels));
                path.lineTo(coord(a,"x2",dm.widthPixels),coord(a,"y2",dm.heightPixels));
            }else path.moveTo(coord(a,"x",dm.widthPixels),coord(a,"y",dm.heightPixels));
            long duration=kind.equals("long_press")?650:kind.equals("swipe")?450:70;
            GestureDescription gesture=new GestureDescription.Builder().addStroke(new GestureDescription.StrokeDescription(path,0,duration)).build();
            if(!dispatchGesture(gesture,new GestureResultCallback(){
                @Override public void onCompleted(GestureDescription g){try{answer.complete(status());}catch(Exception e){answer.completeExceptionally(e);}}
                @Override public void onCancelled(GestureDescription g){answer.completeExceptionally(new IOException("手势被取消，请重新观察屏幕"));}
            },main))throw new IllegalStateException("系统未接受手势");
        }catch(Exception error){answer.completeExceptionally(error);}
    }
    private static float coord(JSONObject a,String key,int size)throws JSONException {
        double n=a.getDouble(key);if(!Double.isFinite(n)||n<0||n>1000)throw new IllegalArgumentException("坐标无效");return (float)(n*(size-1)/1000);
    }
    private void observe(CompletableFuture<JSONObject> answer) throws Exception {
        final String id=task.id;final long epoch=task.epoch;
        takeScreenshot(0,screenshotExecutor,new TakeScreenshotCallback(){
            @Override public void onSuccess(ScreenshotResult result){
                try(android.hardware.HardwareBuffer buffer=result.getHardwareBuffer()){
                    task.require(id,SystemClock.elapsedRealtime());
                    if(task.epoch!=epoch||!task.phase.equals("running"))throw new IllegalStateException("任务状态已改变");
                    unlocked(); Bitmap hardware=Bitmap.wrapHardwareBuffer(buffer,result.getColorSpace());
                    if(hardware==null)throw new IOException("截图为空");
                    Bitmap bitmap=hardware.copy(Bitmap.Config.ARGB_8888,false);hardware.recycle();
                    File file=new File(getExternalFilesDir(null),"gui-screen.png");
                    try(FileOutputStream out=new FileOutputStream(file)){bitmap.compress(Bitmap.CompressFormat.PNG,100,out);}
                    JSONObject reply=status().put("screenshot",file.getAbsolutePath()).put("width",bitmap.getWidth()).put("height",bitmap.getHeight());bitmap.recycle();
                    JSONArray nodes=new JSONArray();collect(getRootInActiveWindow(),nodes,0);observedNodes=nodes;observedWindow=windowIdentity();reply.put("nodes",nodes).put("treeHash",Integer.toHexString(nodes.toString().hashCode()));answer.complete(reply);
                }catch(Exception e){answer.completeExceptionally(e);}
            }
            @Override public void onFailure(int code){answer.completeExceptionally(new IOException("截图失败："+code));}
        });
    }
    private String windowIdentity() {
        AccessibilityNodeInfo root=getRootInActiveWindow();
        return root==null?"":String.valueOf(root.getPackageName())+":"+root.getWindowId();
    }
    // Ignore unrelated animations only when the same labelled target remains under the pointer.
    private static String hitTarget(JSONArray nodes,int x,int y)throws JSONException {
        String found="";long smallest=Long.MAX_VALUE;
        for(int i=0;i<nodes.length();i++) {
            JSONObject node=nodes.getJSONObject(i);JSONArray b=node.getJSONArray("bounds");
            int l=b.getInt(0),t=b.getInt(1),r=b.getInt(2),bottom=b.getInt(3);
            if(x<l||x>=r||y<t||y>=bottom)continue;
            if(node.optString("text").isEmpty()&&node.optString("description").isEmpty())continue;
            long area=(long)(r-l)*(bottom-t);
            if(area<=smallest){smallest=area;found=node.toString();}
        }
        return found;
    }
    private void collect(AccessibilityNodeInfo node,JSONArray out,int depth)throws JSONException {
        if(node==null||depth>40||out.length()>=350)return;
        if(node.isVisibleToUser()){
            Rect bounds=new Rect();node.getBoundsInScreen(bounds);
            JSONObject item=new JSONObject().put("bounds",new JSONArray(new int[]{bounds.left,bounds.top,bounds.right,bounds.bottom}))
                .put("text",node.isPassword()?"":String.valueOf(node.getText()==null?"":node.getText()))
                .put("description",String.valueOf(node.getContentDescription()==null?"":node.getContentDescription()))
                .put("editable",node.isEditable()).put("password",node.isPassword()).put("clickable",node.isClickable());
            out.put(item);
        }
        for(int i=0;i<node.getChildCount()&&out.length()<350;i++)collect(node.getChild(i),out,depth+1);
    }
}
