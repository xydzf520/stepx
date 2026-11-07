package com.stepx.companion.app;

import android.app.Service;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.os.*;
import android.util.Base64;
import android.util.Log;
import org.json.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.*;

/** Original StepOS Binder MCP transport. No socket, shell, or model credentials. */
public final class NativeMcpService extends Service {
    static final String ACTION="com.stepos.mcp.sdk.TOOLS_SERVICE";
    static final String DESCRIPTOR="com.stepos.mcp.sdk.IMCPBinderService";
    static final String CALLBACK="com.stepos.mcp.sdk.IMCPBinderCallback";
    static final String PERMISSION="com.stepos.mcp.sdk.permission.ACCESS_MCP_SERVICES";
    private final ConcurrentHashMap<String,Client> clients=new ConcurrentHashMap<>();
    private volatile String endedTask=""; private volatile int endedUid=-1;
    private final ExecutorService worker=new ThreadPoolExecutor(1,1,0L,TimeUnit.SECONDS,new ArrayBlockingQueue<>(16));
    private static final class Client { final int uid; final IBinder callback; volatile String task=""; volatile boolean connected=true; boolean initialized;
        Client(int uid,IBinder callback){this.uid=uid;this.callback=callback;}
    }
    private void authorize(){
        if(Binder.getCallingUid()!=android.os.Process.myUid() && checkCallingPermission(PERMISSION)!=PackageManager.PERMISSION_GRANTED)
            throw new SecurityException("Original MCP caller permission required");
    }
    private final Binder binder=new Binder(){
        @Override protected boolean onTransact(int code,Parcel data,Parcel reply,int flags)throws RemoteException {
            if(code==INTERFACE_TRANSACTION){if(reply!=null)reply.writeString(DESCRIPTOR);return true;}
            authorize();data.enforceInterface(DESCRIPTOR);
            int uid=Binder.getCallingUid();String session=data.readString();
            if(session==null||session.length()>256||session.isEmpty())throw new IllegalArgumentException("Invalid session");
            String key=uid+":"+session;
            if(code==1){
                IBinder callback=data.readStrongBinder();
                if(callback==null||clients.size()>=16||clients.containsKey(key))throw new IllegalArgumentException("Invalid registration");
                Client c=new Client(uid,callback);clients.put(key,c);callback.linkToDeath(()->remove(key,c),0);
                Log.i("StepXNativeMcp","client registered uid="+uid);return true;
            }
            Client c=clients.get(key);if(c==null)throw new SecurityException("Unregistered session");
            if(code==2){remove(key,c);return true;}
            if(code==3){String body=data.readString();if(body==null||body.length()>32768)throw new IllegalArgumentException("Invalid request");
                try{worker.execute(()->handle(c,body));}catch(RejectedExecutionException e){send(c,"{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32000,\"message\":\"Busy\"}}");}return true;}
            return super.onTransact(code,data,reply,flags);
        }
    };
    @Override public IBinder onBind(Intent intent){return ACTION.equals(intent.getAction())?binder:null;}
    private void remove(String key,Client c){if(clients.remove(key,c)){c.connected=false;stop(c);}}
    private void stop(Client c){
        String task=c.task;c.task="";
        if(!task.isEmpty()){
            new File(getExternalFilesDir(null),"mcp-screen-"+task+".jpg").delete();
            GuiAccessibilityService service=GuiAccessibilityService.instance;
            if(service!=null)service.main.post(()->{
                try{
                    CompletableFuture<JSONObject> answer=new CompletableFuture<>();
                    service.request(new JSONObject().put("op","stop").put("task",task),answer);
                    JSONObject result=answer.getNow(null);
                    if(result!=null&&result.optBoolean("ok")){endedUid=c.uid;endedTask=task;}
                }catch(Exception ignored){}
            });
        }
    }
    private void send(Client c,String body){if(!c.connected)return;Parcel out=Parcel.obtain();try{out.writeInterfaceToken(CALLBACK);out.writeString(body);c.callback.transact(1,out,null,IBinder.FLAG_ONEWAY);}catch(Exception e){stop(c);}finally{out.recycle();}}
    private JSONArray tools()throws Exception {
        int id=getResources().getIdentifier("stepx_mcp_tools","raw",getPackageName());
        try(InputStream in=getResources().openRawResource(id)){
            JSONArray definitions=new JSONArray(new String(in.readAllBytes(),StandardCharsets.UTF_8)),visible=new JSONArray();
            boolean neoSelected=NeoAgent.selected(this);
            for(int i=0;i<definitions.length();i++){
                JSONObject tool=definitions.getJSONObject(i);
                if(McpRoutePolicy.visible(tool.optString("name"),neoSelected))visible.put(tool);
            }
            return visible;
        }
    }
    private void handle(Client c,String body){if(!c.connected)return;Object id=JSONObject.NULL;try{
        JSONObject q=new JSONObject(body);id=q.opt("id");String method=q.getString("method");
        if(method.startsWith("notifications/"))return;
        Object result;
        switch(method){
            case "initialize":c.initialized=true;result=new JSONObject().put("protocolVersion",q.optJSONObject("params")==null?"2024-11-05":q.getJSONObject("params").optString("protocolVersion","2024-11-05"))
                .put("capabilities",new JSONObject().put("tools",new JSONObject().put("listChanged",false)))
                .put("serverInfo",new JSONObject().put("name","StepX伴侣").put("version",getPackageManager().getPackageInfo(getPackageName(),0).versionName));break;
            case "ping":result=new JSONObject();break;
            case "tools/list":result=new JSONObject().put("tools",tools());break;
            case "tools/call":
                if(!c.initialized)throw new IllegalStateException("Initialize first");
                JSONObject p=q.getJSONObject("params");String tool=p.getString("name");
                JSONObject callArgs=p.optJSONObject("arguments");if(callArgs==null)callArgs=new JSONObject();
                result=callTool(c,tool,callArgs);
                Log.i("StepXNativeMcp","tools/call tool="+tool+" op="+callArgs.optString("op")+" uid="+c.uid+" error="+((JSONObject)result).optBoolean("isError"));break;
            default:throw new IllegalArgumentException("Unknown method");
        }
        if(id!=null)send(c,new JSONObject().put("jsonrpc","2.0").put("id",id).put("result",result).toString());
    }catch(Exception e){try{send(c,new JSONObject().put("jsonrpc","2.0").put("id",id==null?JSONObject.NULL:id).put("error",new JSONObject().put("code",-32602).put("message",String.valueOf(e.getMessage()))).toString());}catch(Exception ignored){}}}
    private JSONObject callTool(Client c,String tool,JSONObject args)throws Exception{
        if(!McpRoutePolicy.isSupportedTool(tool))throw new IllegalArgumentException("Unknown tool");
        try{
            if(tool.equals("stepx_desktop_work"))return new JSONObject().put("isError",false).put("content",new JSONArray().put(new JSONObject().put("type","text").put("text",new DesktopWorkStore(this).call(c.uid,args).toString())));
            if(tool.equals("stepx_gui_agent"))return new JSONObject().put("isError",false).put("content",new JSONArray().put(new JSONObject().put("type","text").put("text",GuiDelegation.mcp(this,c.uid,args).toString())));
            if(tool.equals("stepx_expansion"))return new JSONObject().put("isError",false).put("content",new JSONArray().put(new JSONObject().put("type","text").put("text",callExpansion(args).toString())));
            return execute(c,args);
        }catch(Exception e){
            return new JSONObject().put("isError",true).put("content",new JSONArray().put(new JSONObject().put("type","text").put("text",String.valueOf(e.getMessage()))));
        }
    }
    private JSONObject callExpansion(JSONObject args)throws Exception{
        String op=args.optString("op","").trim();
        if(!McpRoutePolicy.isExpansionOperation(op))throw new IllegalArgumentException("Unknown expansion operation");
        switch(op){
            case "status":return ExpansionStore.status(this);
            case "list":{
                int limit=ExpansionRules.normalizeLimit(args.optInt("limit",20));
                String query=args.optString("query","").trim();
                JSONArray items=ExpansionStore.list(this,query,limit);
                return new JSONObject().put("ok",true).put("count",items.length()).put("items",items);
            }
            case "get":{
                String itemId=args.optString("item_id",args.optString("id","")).trim();
                return new JSONObject().put("ok",true).put("file",ExpansionStore.get(this,itemId));
            }
            case "request":{
                String itemId=args.optString("item_id",args.optString("id","")).trim();
                return ExpansionStore.requestRestore(this,itemId);
            }
            default:throw new IllegalArgumentException("Unknown expansion operation");
        }
    }
    private void requireLegacyOperation(String op){
        if(!McpRoutePolicy.legacyOperation(op,NeoAgent.selected(this)))
            throw new IllegalStateException("已启用内置 GUI Agent，请使用 stepx_gui_agent；旧逐步控制仅允许查询或停止既有任务");
    }
    private JSONObject execute(Client c,JSONObject args)throws Exception{
        if(!c.connected)throw new IllegalStateException("MCP session closed");
        String op=args.getString("op");
        if(!Arrays.asList("status","start","observe","action","pause","resume","stop").contains(op))throw new IllegalArgumentException("Unknown operation");
        requireLegacyOperation(op);
        if(op.equals("stop")&&c.task.isEmpty()&&c.uid==endedUid&&!endedTask.isEmpty()&&endedTask.equals(args.optString("task"))) {
            JSONObject stopped=new JSONObject().put("ok",true).put("task",endedTask).put("phase","stopped").put("alreadyStopped",true).put("detail","原会话已结束，未执行任何新动作");
            return new JSONObject().put("isError",false).put("content",new JSONArray().put(new JSONObject().put("type","text").put("text",stopped.toString())));
        }
        JSONObject request=new JSONObject(args.toString());
        if(op.equals("start")){
            String goal=args.getString("goal");if(goal.isBlank()||goal.length()>2000)throw new IllegalArgumentException("Invalid goal");
            request.put("task",UUID.randomUUID().toString().replace("-",""));
        }else if(!op.equals("status")){
            if(c.task.isEmpty()||!c.task.equals(args.optString("task")))throw new SecurityException("Task does not belong to this MCP session");
        }
        GuiAccessibilityService s=GuiAccessibilityService.instance;
        if(s==null)throw new IllegalStateException("请在 StepX伴侣启用看屏操作服务");
        CompletableFuture<JSONObject> answer=new CompletableFuture<>();s.main.post(()->{if(!answer.isDone()){try{if(!c.connected)throw new IllegalStateException("MCP session closed");requireLegacyOperation(op);if(!op.equals("status")&&!op.equals("start")&&!op.equals("stop"))s.task.renew(c.task,SystemClock.elapsedRealtime());s.request(request,answer);}catch(Exception e){answer.completeExceptionally(e);}}});
        JSONObject value;try{value=answer.get(8,TimeUnit.SECONDS);}catch(TimeoutException e){answer.cancel(false);throw e;}
        if(!value.optBoolean("ok"))throw new IllegalStateException(value.optString("error","Operation failed"));
        if(op.equals("start")){if(!c.task.isEmpty())new File(getExternalFilesDir(null),"mcp-screen-"+c.task+".jpg").delete();c.task=request.getString("task");if(!c.connected){stop(c);throw new IllegalStateException("MCP session closed");}}if(op.equals("stop")){endedUid=c.uid;endedTask=c.task;new File(getExternalFilesDir(null),"mcp-screen-"+c.task+".jpg").delete();c.task="";}
        value.put("executionModel","caller-plans-each-action");
        value.put("nextStepHint","start 只建立任务，不会自动执行目标。请由 Amoo 根据 observe 逐步调用 action；若尚未打开目标应用，可先用原厂 open_app_by_name 打开，再 observe。不要轮询等待任务自动完成。最终读图核验后 stop。");
        JSONArray content=new JSONArray();
        if(op.equals("observe")){
            File file=new File(getExternalFilesDir(null),"gui-screen.png");Bitmap image=BitmapFactory.decodeFile(file.getAbsolutePath());
            if(image==null)throw new IllegalStateException("Screenshot unavailable");
            int width=Math.min(480,image.getWidth());Bitmap scaled=Bitmap.createScaledBitmap(image,width,Math.max(1,image.getHeight()*width/image.getWidth()),true);
            ByteArrayOutputStream bytes=new ByteArrayOutputStream();scaled.compress(Bitmap.CompressFormat.JPEG,75,bytes);if(scaled!=image)scaled.recycle();image.recycle();
            if(bytes.size()>350000)throw new IllegalStateException("Screenshot exceeds IPC limit");
            File capture=new File(getExternalFilesDir(null),"mcp-screen-"+c.task+".jpg");
            try(FileOutputStream out=new FileOutputStream(capture)){out.write(bytes.toByteArray());}
            value.remove("screenshot");value.put("screenshotUri","content://"+McpScreenshotProvider.AUTHORITY+"/"+c.task+".jpg").put("captureTimeMs",System.currentTimeMillis());content.put(new JSONObject().put("type","image").put("mimeType","image/jpeg").put("data",Base64.encodeToString(bytes.toByteArray(),Base64.NO_WRAP)));
        }
        content.put(new JSONObject().put("type","text").put("text",value.toString()));
        return new JSONObject().put("isError",false).put("content",content);
    }
    @Override public void onDestroy(){for(Client c:clients.values()){c.connected=false;stop(c);}clients.clear();worker.shutdownNow();super.onDestroy();}
}
