package stepx;

import android.content.*;
import android.os.*;
import org.json.JSONObject;

/** Five fixed Android pairing operations; transport must authenticate the companion first. */
public final class ScopedPairing {
    private static final String DESCRIPTOR="android.debug.IAdbManager";
    private static final String PERMISSION="android.permission.MANAGE_DEBUGGING";
    private static final String PAIR_RESULT="com.android.server.adb.WIRELESS_DEBUG_PAIRING_RESULT";
    private static ScopedPairing instance;
    private final Context app;
    private final Handler main=new Handler(Looper.getMainLooper());
    private String id="",state="idle",message="可以开始安全扫码配对。";
    private long expires;
    private boolean active;
    private ScopedPairing(Context app){this.app=app;}
    public static synchronized void shutdown(){
        if(instance==null)return;ScopedPairing current=instance;instance=null;
        synchronized(current){try{current.cancelOwned();}catch(Exception ignored){}current.main.removeCallbacksAndMessages(null);}
    }
    static boolean isAllowedCaller(int uid,String pkg,int sdkInt){
        return sdkInt==36&&((uid==2000&&"com.android.shell".equals(pkg))
            ||(uid==1000&&("android".equals(pkg)||"com.stepos.agentbus".equals(pkg))));
    }
    public static JSONObject handle(Context app,JSONObject request)throws Exception{
        if(!isAllowedCaller(android.os.Process.myUid(),app.getPackageName(),Build.VERSION.SDK_INT))throw new SecurityException("不支持当前手机的安全扫码适配");
        String op=request.getString("op");
        if(!op.equals("pairing_status")&&!op.equals("pairing_enable")&&!op.equals("pairing_disable")&&!op.equals("pairing_begin")&&!op.equals("pairing_cancel"))throw new IllegalArgumentException("不支持的安全配对操作");
        ScopedPairing target;
        synchronized(ScopedPairing.class){if(instance==null)instance=new ScopedPairing(app);target=instance;}
        return target.execute(op,request);
    }
    private static IBinder service()throws Exception {
        IBinder b=(IBinder)Class.forName("android.os.ServiceManager").getMethod("getService",String.class).invoke(null,"adb");
        if(b==null||!DESCRIPTOR.equals(b.getInterfaceDescriptor()))throw new IllegalStateException("系统安全配对接口不可用");return b;
    }
    private static Parcel call(int code,String first,String second)throws Exception {
        // Exact Android 16 IAdbManager ABI, never supplied by the caller.
        if(code!=9&&code!=10&&code!=11&&code!=13)throw new SecurityException("不允许该配对操作");
        Parcel request=Parcel.obtain(),reply=Parcel.obtain();
        try{request.writeInterfaceToken(DESCRIPTOR);if(first!=null)request.writeString(first);if(second!=null)request.writeString(second);if(!service().transact(code,request,reply,0))throw new IllegalStateException("系统没有实现安全配对接口");reply.readException();return reply;}catch(Exception e){reply.recycle();throw e;}finally{request.recycle();}
    }
    private boolean supported()throws Exception {Parcel p=call(13,null,null);try{return p.readInt()!=0;}finally{p.recycle();}}
    private boolean enabled()throws Exception{return PairingProviders.wirelessEnabled();}
    private int port()throws Exception {if(!enabled())return -1;Parcel p=call(10,null,null);try{return p.readInt();}finally{p.recycle();}}
    private final Runnable timeout=()->{synchronized(this){if(active&&SystemClock.elapsedRealtime()>=expires){try{cancelOwned();}catch(Exception ignored){}state="error";message="二维码已过期，请在 desktop 刷新二维码。";}}};
    private void cancelOwned()throws Exception {if(active)call(11,null,null).recycle();active=false;expires=0;main.removeCallbacks(timeout);}
    private synchronized JSONObject execute(String op,JSONObject request)throws Exception {
        if(active&&SystemClock.elapsedRealtime()>=expires){cancelOwned();state="error";message="二维码已过期，请在 desktop 刷新二维码。";}
        if(request.has("nativeResult")){
            if(!op.equals("pairing_status")||!active||!id.equals(request.optString("pairingId")))throw new SecurityException("配对结果不属于当前请求");
            int result=request.getInt("nativeResult");
            if(result!=0&&result!=1&&result!=2)throw new IllegalArgumentException("无效的系统配对结果");
            active=false;expires=0;main.removeCallbacks(timeout);
            state=result==1?"paired":result==2?"cancelled":"error";
            message=result==1?"已与电脑安全配对，desktop 将自动连接。":result==2?"本次配对已取消。":"配对未成功，请检查同一 Wi-Fi 并刷新 desktop 二维码。";
        }
        if(op.equals("pairing_enable")){
            if(!supported())throw new IllegalStateException("此手机不支持 Android 安全二维码配对");
            // Only enables the setting. Android still asks the user to trust a new Wi-Fi network.
            PairingProviders.setWirelessEnabled(true);
            state="awaiting_network";message="正在准备无线连接；若系统询问是否信任当前 Wi-Fi，请在手机确认。";
        }else if(op.equals("pairing_disable")){
            cancelOwned();PairingProviders.setWirelessEnabled(false);state="idle";message="无线连接已关闭，USB 与已配对记录不受影响。";
        }else if(op.equals("pairing_begin")){
            String next=request.getString("pairingId"),name=request.getString("service"),password=request.getString("password");
            if(!next.matches("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")||!name.matches("studio-stepx-[0-9a-f]{16}")||!password.matches("[A-Za-z0-9]{20}"))throw new IllegalArgumentException("不是有效的 StepX 连接二维码");
            if(!supported()||!enabled()||port()<=0)throw new IllegalStateException("请先开启无线连接并完成系统 Wi-Fi 确认");
            cancelOwned();id=next;call(9,name,password).recycle();active=true;expires=SystemClock.elapsedRealtime()+120000;state="pairing";message="已识别 StepX 二维码，等待电脑完成安全配对…";main.postDelayed(timeout,120000);
        }else if(op.equals("pairing_cancel")){
            String requested=request.getString("pairingId");if(active&&!id.equals(requested))throw new SecurityException("该配对不属于当前请求");
            if(active&&id.equals(requested)){cancelOwned();state="cancelled";message="本次配对已取消。";}
        }
        boolean enabled=enabled();int port=port();
        if(state.equals("awaiting_network")&&enabled&&port>0){state="idle";message="无线连接已就绪，可以扫描 desktop 二维码。";}
        return new JSONObject().put("supported",supported()).put("wirelessEnabled",enabled).put("port",port).put("pairingActive",active).put("pairingState",state).put("pairingMessage",message).put("pairingId",id);
    }
}
