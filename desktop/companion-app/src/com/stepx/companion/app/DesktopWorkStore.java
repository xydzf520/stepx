package com.stepx.companion.app;

import android.content.Context;
import android.os.Build;
import org.json.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;

/** Local MCP delegates only explicitly selected file copies to the paired desktop. */
final class DesktopWorkStore {
    private static final Object MUTATION = new Object();
    static final String FIRMWARE="StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys";
    static final String FIRMWARE_12="StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/12:user/release-keys";
    private final TransferStore transfers;
    DesktopWorkStore(Context context){transfers=new TransferStore(context);}
    private File root()throws Exception {
        File root=new File(transfers.root(),"desktop-work");
        if(Files.isSymbolicLink(root.toPath()))throw new IOException("任务目录不可用");
        if(!root.isDirectory()&&!root.mkdirs())throw new IOException("无法创建任务目录");
        return root;
    }
    private File directory(String id)throws Exception {
        if(!TransferStore.uuid(id))throw new IOException("任务标识无效");
        File dir=new File(root(),id);
        if(Files.isSymbolicLink(dir.toPath()))throw new IOException("任务目录不可用");
        return dir;
    }
    JSONArray files()throws Exception {
        JSONArray result=new JSONArray();String desktop=transfers.link().id;
        for(TransferStore.Entry entry:transfers.list()){
            if(!entry.direction.equals("outbox"))continue;
            JSONObject m=TransferStore.readJson(new File(transfers.root(),"transfers/outbox/"+entry.id+"/manifest.json"));
            if(m.optBoolean("workAllowed")&&desktop.equals(m.optString("targetDesktopId")))
                result.put(new JSONObject().put("id",entry.id).put("name",entry.name).put("size",entry.size));
            if(result.length()>=30)break;
        }
        return result;
    }
    JSONObject create(String id,String goal,JSONArray ids,int uid)throws Exception {
        synchronized(MUTATION){return createLocked(id,goal,ids,uid);}
    }
    private JSONObject createLocked(String id,String goal,JSONArray ids,int uid)throws Exception {
        if(!FIRMWARE.equals(Build.FINGERPRINT)&&!FIRMWARE_12.equals(Build.FINGERPRINT))throw new IOException("固件已变化，端侧 MCP 需要重新验证；文件互传仍可使用");
        TransferStore.Link link=transfers.link();
        if(goal==null||goal.trim().isEmpty()||goal.length()>4000||ids==null||ids.length()<1||ids.length()>10)throw new IOException("请填写任务并选择 1–10 个资料");
        File dir=directory(id),request=new File(dir,"request.json");
        if(request.exists()){
            JSONObject old=TransferStore.readJson(request);
            if(old.optInt("ownerUid")!=uid||!old.optString("goal").equals(goal)||!old.getJSONArray("files").toString().equals(ids.toString())||!old.optString("desktopId").equals(link.id))throw new IOException("任务标识已用于其他请求");
            return status(id,uid);
        }
        if(!link.online)throw new IOException("电脑离线，请打开 desktop 并连接同一 Wi-Fi；尚未提交任务");
        File[] existing=root().listFiles();if(existing!=null&&existing.length>=100)throw new IOException("任务记录已满，请先整理任务");
        HashSet<String> seen=new HashSet<>();
        for(int i=0;i<ids.length();i++){
            String file=ids.getString(i);if(!TransferStore.uuid(file)||!seen.add(file))throw new IOException("资料标识无效或重复");
            JSONObject manifest=TransferStore.readJson(new File(transfers.root(),"transfers/outbox/"+file+"/manifest.json"));
            if(!manifest.optBoolean("workAllowed")||!link.id.equals(manifest.optString("targetDesktopId")))throw new IOException("资料未授权给这台电脑处理，请在伴侣文件页重新选择");
            transfers.verifiedFile("outbox",file);
        }
        if(!dir.mkdir())throw new IOException("无法保存任务");
        TransferStore.writeJson(request,new JSONObject().put("protocol",1).put("id",id).put("goal",goal).put("files",ids).put("desktopId",link.id).put("ownerUid",uid).put("createdAt",System.currentTimeMillis()).put("firmware",Build.FINGERPRINT));
        return status(id,uid);
    }
    JSONObject status(String id,int uid)throws Exception {
        File dir=directory(id);JSONObject request=TransferStore.readJson(new File(dir,"request.json"));
        if(uid>=0&&request.optInt("ownerUid")!=uid)throw new SecurityException("任务属于另一个调用者");
        JSONObject result=new JSONObject().put("task",id).put("phase","queued").put("detail","等待电脑接收").put("goal",request.optString("goal"));
        File receipt=new File(dir,"receipt.json");
        if(receipt.isFile()){
            JSONObject r=TransferStore.readJson(receipt);
            if(!id.equals(r.optString("task"))||!request.optString("desktopId").equals(r.optString("desktopId")))throw new IOException("任务回执不匹配");
            result=r;result.put("goal",request.optString("goal"));
        }
        if(new File(dir,"cancel.json").isFile()&&!terminal(result.optString("phase")))
            result.put("phase","cancelling").put("detail","正在停止电脑任务，等待执行器确认；已生成文件保留");
        File retry=new File(dir,"retry.json");
        if(retry.isFile())result.put("retryTask",TransferStore.readJson(retry).optString("task"));
        TransferStore.Link link=transfers.link();
        String taskDesktop=request.optString("desktopId");
        result.put("desktopId",taskDesktop);
        result.put("createdAt",request.optLong("createdAt"));
        result.put("desktopOnline",DesktopWorkPolicy.sameDesktopOnline(taskDesktop,link.id,link.online));
        result.put("differentDesktop",link.online&&!taskDesktop.equals(link.id));
        result.put("instruction","queued/running 是处理中；completed 且有 delivered 文件才可宣称成果已到手机。离线保留任务，不要重复 start。可查询 status 或 cancel。");
        return result;
    }
    JSONObject cancel(String id,int uid)throws Exception {
        synchronized(MUTATION){
        JSONObject value=status(id,uid);
        if(terminal(value.optString("phase")))return value;
        TransferStore.writeJson(new File(directory(id),"cancel.json"),new JSONObject().put("task",id).put("requestedAt",System.currentTimeMillis()));
        return status(id,uid).put("cancelRequested",true);
        }
    }
    private static boolean terminal(String phase){return Arrays.asList("completed","failed","cancelled","interrupted").contains(phase);}
    JSONObject retry(String id,String requestId,int uid)throws Exception {
        synchronized(MUTATION){
            JSONObject old=status(id,uid);
            if(!Arrays.asList("failed","interrupted","cancelled").contains(old.optString("phase")))throw new IOException("仅失败或已停止的任务可以重试；正在处理的任务请继续等待");
            File dir=directory(id), marker=new File(dir,"retry.json");
            JSONObject original=TransferStore.readJson(new File(dir,"request.json"));
            if(marker.exists())requestId=TransferStore.readJson(marker).getString("task");
            if(!TransferStore.uuid(requestId)||id.equals(requestId))throw new IOException("请为重试提供新的 UUID");
            File target=directory(requestId);
            if(target.exists()&&!marker.exists())throw new IOException("重试标识已用于其他任务");
            if(!marker.exists())TransferStore.writeJson(marker,new JSONObject().put("task",requestId));
            JSONObject result=createLocked(requestId,original.getString("goal"),original.getJSONArray("files"),original.getInt("ownerUid"));
            // An explicit retry can reattempt previously failed source transfers immediately.
            JSONArray ids=original.getJSONArray("files");
            for(int i=0;i<ids.length();i++){
                File receipt=new File(transfers.root(),"transfers/outbox/"+ids.getString(i)+"/receipt.json");
                if(receipt.isFile()&&"error".equals(TransferStore.readJson(receipt).optString("status")))Files.delete(receipt.toPath());
            }
            return result.put("retryOf",id);
        }
    }
    JSONArray tasks()throws Exception {
        JSONArray items=new JSONArray();File[] dirs=root().listFiles();if(dirs==null)return items;
        Arrays.sort(dirs,Comparator.comparingLong(File::lastModified).reversed());
        for(File dir:dirs){if(items.length()>=20)break;try{items.put(status(dir.getName(),-1));}catch(Exception ignored){}}
        return items;
    }
    JSONObject call(int uid,JSONObject args)throws Exception {
        switch(args.getString("op")){
            case "files":return new JSONObject().put("files",files()).put("desktopOnline",transfers.link().online).put("hint","仅列出用户从伴侣的‘交给 Amoo 处理’选择过的资料；其他资料请先选择。");
            case "start":return create(args.getString("requestId"),args.getString("goal"),args.getJSONArray("fileIds"),uid);
            case "status":return status(args.getString("task"),uid);
            case "cancel":return cancel(args.getString("task"),uid);
            case "retry":return retry(args.getString("task"),args.getString("requestId"),uid);
            default:throw new IllegalArgumentException("不支持的电脑任务操作");
        }
    }
}
