package stepx;
import android.content.Context;
import android.app.KeyguardManager;
import android.graphics.*;
import android.os.SystemClock;
import android.util.Base64;
import org.json.*;
import java.io.*;
import java.util.*;
import java.util.concurrent.*;

/** No shell command API: one owned virtual display, bounded lifetime and fixed MDI calls. */
final class MdiBroker {
    private static String owner="";
    private static long touched;
    private static Context context;
    private static MdiTransport active;
    private static int displayId=-1;
    private static long probeUntil;
    private static final ScheduledExecutorService timer=Executors.newSingleThreadScheduledExecutor();
    static {timer.scheduleWithFixedDelay(()->{synchronized(MdiBroker.class){if(!owner.isEmpty()&&SystemClock.elapsedRealtime()-touched>120000)try{finish();}catch(Exception ignored){}}},15,15,TimeUnit.SECONDS);}
    static synchronized JSONObject handle(Context c,JSONObject q)throws Exception {
        if(android.os.Process.myUid()!=1000)throw new SecurityException("MDI 需要已适配的系统服务");
        context=c;String method=q.getString("method");JSONObject args=q.optJSONObject("params");if(args==null)args=new JSONObject();
        if(method.equals("probe")){
            if(active==null&&SystemClock.elapsedRealtime()>probeUntil)try(MdiTransport bus=new MdiTransport(context)){bus.probe();probeUntil=SystemClock.elapsedRealtime()+30000;}
            return new JSONObject().put("content",new JSONObject().put("ok",true).put("transportReady",true).put("busy",!owner.isEmpty()).put("serviceUid",android.os.Process.myUid()).put("transport","official-mcp-binder"));}
        if(method.equals("mcp_call")||method.equals("tools/call")){
            String toolName=args.optString("name",args.optString("tool","")).trim();
            JSONObject toolArgs=args.optJSONObject("arguments");
            if(toolArgs==null)toolArgs=args.optJSONObject("args");
            if(toolArgs==null){
                toolArgs=new JSONObject(args.toString());
                toolArgs.remove("name");
                toolArgs.remove("tool");
            }
            if(!MdiTransport.isAllowedTool(toolName)||MdiTransport.isMdiTool(toolName))throw new IllegalArgumentException("不支持的系统 MCP 工具："+toolName);
            android.content.ComponentName target=MdiTransport.resolveServiceComponent(toolName);
            try(MdiTransport bus=new MdiTransport(context,target)){
                Object content=bus.call(toolName,toolArgs);
                return new JSONObject().put("content",content).put("service",target.flattenToShortString());
            }
        }
        if(MdiTransport.isAllowedTool(method)&&!MdiTransport.isMdiTool(method)){
            android.content.ComponentName target=MdiTransport.resolveServiceComponent(method);
            try(MdiTransport bus=new MdiTransport(context,target)){
                Object content=bus.call(method,args);
                return new JSONObject().put("content",content).put("service",target.flattenToShortString());
            }
        }
        String name=args.getString("taskName");
        if(!name.matches("stepx-mdi-[a-f0-9-]{16,64}"))throw new SecurityException("MDI 任务标识无效");
        if(method.equals("lease")){if(!name.equals(owner))throw new SecurityException("MDI 任务已回收");touched=SystemClock.elapsedRealtime();return new JSONObject().put("content","renewed");}
        if(method.equals("mdi_task_finish")){
            if(owner.isEmpty())return new JSONObject().put("content","already released");
            if(!name.equals(owner))throw new SecurityException("不是当前 MDI 任务");finish();return new JSONObject().put("content","released");
        }
        if(((KeyguardManager)c.getSystemService(Context.KEYGUARD_SERVICE)).isKeyguardLocked())throw new SecurityException("请先手动解锁手机，MDI 不绕过锁屏");
        if(method.equals("mdi_task_create")){
            if(!owner.isEmpty())throw new IllegalStateException("MDI 已有任务，等待结束或自动回收");
            // Persist ownership before the call; a timeout may still create a native task.
            owner=name;touched=SystemClock.elapsedRealtime();args.put("reuse",false);
        }else if(!name.equals(owner))throw new SecurityException("MDI 任务不属于伴侣");
        touched=SystemClock.elapsedRealtime();
        if(active==null)active=new MdiTransport(context);
        {Object content=method.equals("mdi_capture")?capture():active.call(method,args);
            if(method.equals("mdi_task_create"))displayId=((JSONObject)content).getInt("displayId");
            if(method.equals("mdi_capture")){
                Bitmap b=(Bitmap)content;
                int width=b.getWidth(),height=b.getHeight();Bitmap scaled=Bitmap.createScaledBitmap(b,540,Math.max(1,height*540/width),true);
                ByteArrayOutputStream out=new ByteArrayOutputStream();scaled.compress(Bitmap.CompressFormat.JPEG,72,out);
                scaled.recycle();if(scaled!=b)b.recycle();if(out.size()>320000)throw new IOException("MDI 截图过大");
                content=new JSONObject().put("format","jpeg").put("base64",Base64.encodeToString(out.toByteArray(),Base64.NO_WRAP)).put("width",width).put("height",height);
            }
            return new JSONObject().put("content",content);
        }
    }
    // Capture only our owned display inside system_server. The original MCP serializes
    // full-resolution PNG as a Binder String and can exceed Android's transaction limit.
    private static Bitmap capture()throws Exception {
        if(displayId<=0)throw new IOException("MDI 虚拟屏尚未创建");
        Class<?> api=Class.forName("android.window.ScreenCapture");
        Class<?> args=Class.forName("android.window.ScreenCapture$CaptureArgs");
        Class<?> listenerType=Class.forName("android.window.ScreenCapture$ScreenCaptureListener");
        Object listener=api.getMethod("createSyncCaptureListener").invoke(null);
        Object wm=Class.forName("android.view.WindowManagerGlobal").getMethod("getWindowManagerService").invoke(null);
        Class.forName("android.view.IWindowManager").getMethod("captureDisplay",int.class,args,listenerType).invoke(wm,displayId,null,listener);
        Object buffer=Class.forName("android.window.ScreenCapture$SynchronousScreenCaptureListener").getMethod("getBuffer").invoke(listener);
        if(buffer==null)throw new IOException("MDI 截图未返回，请稍后重试");
        Bitmap hardware=null;
        try{hardware=(Bitmap)buffer.getClass().getMethod("asBitmap").invoke(buffer);
            if(hardware==null)throw new IOException("MDI 截图为空");return hardware.copy(Bitmap.Config.ARGB_8888,false);}
        finally{if(hardware!=null)hardware.recycle();((android.hardware.HardwareBuffer)buffer.getClass().getMethod("getHardwareBuffer").invoke(buffer)).close();}
    }
    static synchronized void shutdown(){timer.shutdownNow();try{if(!owner.isEmpty())finish();}catch(Exception ignored){}finally{if(active!=null)active.close();active=null;owner="";displayId=-1;}}
    private static void finish()throws Exception {
        if(active==null)active=new MdiTransport(context);
        JSONObject args=new JSONObject().put("taskName",owner).put("ignoreActions",true);
        try{active.call("mdi_task_finish",args);}catch(Exception interrupted){active.close();active=new MdiTransport(context);active.call("mdi_task_finish",args);}
        owner="";displayId=-1;active.close();active=null;
    }
}
