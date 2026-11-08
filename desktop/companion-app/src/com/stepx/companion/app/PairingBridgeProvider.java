package com.stepx.companion.app;

import android.content.*;
import android.database.Cursor;
import android.net.Uri;
import android.os.*;
import org.json.JSONObject;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;

/** DUMP-protected Binder donation; only the ADB shell can attach the scoped pairing service. */
public final class PairingBridgeProvider extends ContentProvider {
    private static final String DESCRIPTOR="com.stepx.companion.Pairing.v1";
    private static volatile IBinder service;
    private static volatile int servicePid,serviceUid;
    private static volatile String ownedPairingId="";
    private static volatile long pairingStarted;
    private static volatile boolean nativePortReady;
    private static volatile String lastPublishedStatus=null;
    private HandlerThread resultsThread;
    @Override public boolean onCreate(){
        resultsThread=new HandlerThread("StepXPairingResults");resultsThread.start();Handler results=new Handler(resultsThread.getLooper());
        getContext().registerReceiver(new BroadcastReceiver(){@Override public void onReceive(Context context,Intent intent){
            // Only a live round initiated in this app process may consume a non-sticky OS result.
            if(isInitialStickyBroadcast()||!"com.android.server.adb.WIRELESS_DEBUG_PAIRING_RESULT".equals(intent.getAction()))return;
            int result=intent.getIntExtra("status",-1);String round=ownedPairingId;long age=SystemClock.elapsedRealtime()-pairingStarted;
            if(round.isEmpty()||age<0||age>120000)return;
            // Android emits the new pairing port before its result. Old cancellation results
            // queued before the new port must not complete the replacement round.
            if(result==4&&intent.getIntExtra("adb_port",-1)>0){nativePortReady=true;return;}
            if(!nativePortReady||(result!=0&&result!=1&&result!=2))return;
            results.post(()->{try{
                if(!round.equals(ownedPairingId))return;
                JSONObject status=exchange(new JSONObject().put("op","pairing_status").put("nativeResult",result).put("pairingId",round));publish(context,status);
            }catch(Exception ignored){/* Cancelled, expired or replaced round: do not revive history. */}});
        }},new IntentFilter("com.android.server.adb.WIRELESS_DEBUG_PAIRING_RESULT"),"android.permission.MANAGE_DEBUGGING",new Handler(Looper.getMainLooper()),Context.RECEIVER_EXPORTED);
        return true;
    }
    @Override public Bundle call(String method,String arg,Bundle extras){
        int uid=Binder.getCallingUid(),pid=Binder.getCallingPid();
        // A diagnostic must observe a fresh Binder roundtrip, not pairing.json.
        // The manifest also requires DUMP. This branch cannot start/cancel pairing
        // or change wireless settings, and never discloses the pairing password.
        if(uid==2000&&pid>0&&"status".equals(method)&&arg==null&&(extras==null||extras.isEmpty())){
            long identity=Binder.clearCallingIdentity();
            try{
                JSONObject answer;
                try{answer=exchange(new JSONObject().put("op","pairing_status"));}
                catch(Exception unavailable){answer=new JSONObject().put("ok",false);}
                JSONObject proof=new JSONObject().put("protocol",1).put("ok",answer.optBoolean("ok"))
                    .put("service","stepx-pairing-v1").put("uid",android.os.Process.myUid())
                    .put("pid",answer.optInt("pid")).put("processUid",answer.optInt("processUid"))
                    .put("supported",answer.optBoolean("supported"));
                Bundle response=new Bundle();response.putString("result",android.util.Base64.encodeToString(
                    proof.toString().getBytes(StandardCharsets.UTF_8),android.util.Base64.NO_WRAP));return response;
            }catch(Exception error){throw new IllegalStateException("Unable to inspect pairing service",error);}
            finally{Binder.restoreCallingIdentity(identity);}
        }
        if((uid!=2000&&uid!=1000)||pid<=0||!"attach".equals(method)||arg!=null||extras==null||extras.getInt("version")!=1)throw new SecurityException("Untrusted pairing service");
        IBinder candidate=extras.getBinder("service");
        if(candidate==null)throw new IllegalArgumentException("Missing pairing service");
        long identity=Binder.clearCallingIdentity();
        try{
            if(!DESCRIPTOR.equals(candidate.getInterfaceDescriptor()))throw new SecurityException("Wrong pairing interface");
            JSONObject status=exchange(candidate,pid,uid,new JSONObject().put("op","pairing_status"));
            String statusKey=status==null?"":status.toString();
            boolean changed=false;
            synchronized(PairingBridgeProvider.class){
                if(service!=candidate){candidate.linkToDeath(()->invalidate(getContext(),candidate),0);service=candidate;servicePid=pid;serviceUid=uid;changed=true;}
                if(!statusKey.equals(lastPublishedStatus)){lastPublishedStatus=statusKey;changed=true;}
            }
            if(changed){
                publish(getContext(),status);
                AutoConnectService.resume(getContext());
            }
            Bundle answer=new Bundle();answer.putBoolean("accepted",true);return answer;
        }catch(Exception e){throw new IllegalStateException("Unable to verify scoped pairing service",e);}finally{Binder.restoreCallingIdentity(identity);}
    }
    static boolean available(){IBinder current=service;return current!=null&&current.isBinderAlive();}
    static JSONObject exchange(JSONObject request)throws Exception{
        IBinder current;int pid,uid;synchronized(PairingBridgeProvider.class){current=service;pid=servicePid;uid=serviceUid;}
        if(current==null||!current.isBinderAlive())throw new IOException("安全扫码服务尚未就绪，请在 desktop 小工具中安装或修复 StepX伴侣。");
        boolean begin="pairing_begin".equals(request.optString("op"));
        if(begin){nativePortReady=false;ownedPairingId=request.getString("pairingId");pairingStarted=SystemClock.elapsedRealtime();}
        try{
            JSONObject result=exchange(current,pid,uid,request);
            if(!result.optBoolean("pairingActive")&&ownedPairingId.equals(result.optString("pairingId")))ownedPairingId="";
            return result;
        }catch(Exception error){if(begin)ownedPairingId="";throw error;}
    }
    /** In-process adapter API; Binder donation is verified on every registration. */
    public static JSONObject mdi(JSONObject request)throws Exception {
        IBinder target; synchronized(PairingBridgeProvider.class){target=service;}
        if(target==null||!target.isBinderAlive())throw new IOException("MDI 系统服务未连接，请从电脑更新伴侣服务");
        Parcel data=Parcel.obtain(),reply=Parcel.obtain();
        try {
            data.writeInterfaceToken(DESCRIPTOR);data.writeString(request.toString());
            if(!target.transact(IBinder.FIRST_CALL_TRANSACTION+1,data,reply,0))throw new IOException("MDI 需要更新伴侣系统服务");
            reply.readException();String raw=reply.readString();
            if(raw==null||raw.length()>700000)throw new IOException("MDI 返回无效");
            JSONObject value=new JSONObject(raw);
            if(!value.optBoolean("ok"))throw new IOException(value.optString("error","MDI 执行失败"));
            return value;
        }finally{data.recycle();reply.recycle();}
    }
    private static JSONObject exchange(IBinder target,int pid,int uid,JSONObject request)throws Exception{
        Parcel data=Parcel.obtain(),reply=Parcel.obtain();
        try{
            data.writeInterfaceToken(DESCRIPTOR);data.writeString(request.toString());
            if(!target.transact(IBinder.FIRST_CALL_TRANSACTION,data,reply,0))throw new IOException("配对服务版本不兼容");
            reply.readException();String raw=reply.readString();if(raw==null||raw.length()>32768)throw new IOException("配对服务返回数据无效");
            JSONObject result=new JSONObject(raw);
            if(result.optInt("version")!=1||!"stepx-pairing-v1".equals(result.optString("service"))||result.optInt("processUid")!=uid||result.optInt("pid")!=pid||result.optInt("uid")!=android.os.Process.myUid())throw new SecurityException("配对服务身份校验失败");
            if(!result.optBoolean("ok"))throw new IOException(result.optString("error","安全配对未完成"));return result;
        }finally{data.recycle();reply.recycle();}
    }
    private static void invalidate(Context context,IBinder expected){
        synchronized(PairingBridgeProvider.class){if(service!=expected)return;service=null;servicePid=0;serviceUid=0;ownedPairingId="";lastPublishedStatus=null;}
        publish(context,null);
    }
    private static void publish(Context context,JSONObject status){try{
        JSONObject j=new JSONObject().put("protocol",1).put("pairingPermissionGranted",PairingController.permitted(context)).put("wirelessSettingsPermissionGranted",PairingController.settingsPermitted(context)).put("wirelessEnabled",status!=null&&status.optBoolean("wirelessEnabled")).put("pairingSupported",status!=null&&status.optBoolean("supported")).put("pairingBridgeReady",status!=null).put("pairingProcessUid",status==null?0:status.optInt("processUid")).put("pairingProcessPid",status==null?0:status.optInt("pid")).put("pairingState",status==null?"unavailable":status.optString("pairingState")).put("pairingMessage",status==null?"安全扫码服务已停止，请在 desktop 修复 StepX伴侣。":status.optString("pairingMessage")).put("updatedAtMs",System.currentTimeMillis());
        File directory=context.getExternalFilesDir(null);if(directory==null)return;directory.mkdirs();
        File temp=new File(directory,"pairing-provider.json.tmp");Files.write(temp.toPath(),j.toString().getBytes(StandardCharsets.UTF_8));Files.move(temp.toPath(),new File(directory,"pairing.json").toPath(),StandardCopyOption.REPLACE_EXISTING,StandardCopyOption.ATOMIC_MOVE);
    }catch(Exception ignored){}}
    @Override public Cursor query(Uri u,String[] p,String s,String[] a,String o){throw new UnsupportedOperationException();}
    @Override public String getType(Uri u){return null;}
    @Override public Uri insert(Uri u,ContentValues v){throw new UnsupportedOperationException();}
    @Override public int delete(Uri u,String s,String[] a){throw new UnsupportedOperationException();}
    @Override public int update(Uri u,ContentValues v,String s,String[] a){throw new UnsupportedOperationException();}
}
