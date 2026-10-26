package com.stepx.companion.app;

import android.app.*;
import android.content.*;
import android.content.pm.*;
import android.os.*;
import android.provider.Settings;
import org.json.JSONObject;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.util.HashSet;
import java.util.Set;

/** Fixed, permission-protected native CloudWake requests during real desktop task leases. */
public final class AmooConnectionService extends Service {
    private static final String NATIVE_PACKAGE="com.stepos.agentbus";
    private static final String NATIVE_PATH="/system/priv-app/AgentBus/AgentBus.apk";
    private static final String NATIVE_SHA="50a1bbd6af20d1cceff335e2336cff04e08d3718cf4cad581a86954c354f2a48";
    private static final String FINGERPRINT="StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys";
    private static final String MESSAGE_PERMISSION="com.stepos.cloud.push.permission.MESSAGE";
    private static final String[] PERMISSIONS={"com.stepos.permission.MANAGE_BACK_SCREEN","android.permission.MANAGE_DEBUGGING","android.permission.WRITE_SECURE_SETTINGS","android.permission.CAMERA","android.permission.POST_NOTIFICATIONS","android.permission.READ_PHONE_STATE","android.permission.WAKE_LOCK","android.permission.FOREGROUND_SERVICE","android.permission.FOREGROUND_SERVICE_SPECIAL_USE","android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE","android.permission.INTERNET","android.permission.ACCESS_NETWORK_STATE","android.permission.ACCESS_WIFI_STATE","android.permission.CHANGE_NETWORK_STATE",MESSAGE_PERMISSION};
    static boolean permissionsGranted(Context context){for(String permission:PERMISSIONS)if(context.checkSelfPermission(permission)!=PackageManager.PERMISSION_GRANTED)return false;return true;}
    private final Handler main=new Handler(Looper.getMainLooper());
    private final AmooLeaseState leases=new AmooLeaseState();
    private final Set<String> oneShot=new HashSet<>(),oneShotDone=new HashSet<>();
    private HandlerThread workerThread;
    private Handler worker;
    private PowerManager power;
    private PowerManager.WakeLock cpu;
    private SharedPreferences preferences;
    private boolean supported,registered;
    private volatile boolean closed;
    private volatile int latestStart;
    private String lastError="",requestId="",observedLease="",lastNotifiedText="",lastWrittenState="",lastWrittenMessage="";
    private long observedAt=0,lastWriteMs=0;
    private int bootCount,lastLeaseCount=-1;
    private final BroadcastReceiver screen=new BroadcastReceiver(){@Override public void onReceive(Context c,Intent i){
        if(worker==null)return;
        final String action=i.getAction();final long receivedAt=SystemClock.elapsedRealtime();
        // SCREEN_ON/OFF are protected system broadcasts, received by our
        // non-exported observer. Preserve a real ON edge even when rapid ON/OFF
        // transitions both finish before the worker reads current power state.
        worker.post(()->{if(Intent.ACTION_SCREEN_ON.equals(action))leases.screen(true,receivedAt);
            else if(Intent.ACTION_SCREEN_OFF.equals(action))leases.screen(false,receivedAt);tick();});
    }};
    private final Runnable pulse=()->tick();
    @Override public void onCreate(){
        super.onCreate();
        power=getSystemService(PowerManager.class);cpu=power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,"StepX:amoo-task");cpu.setReferenceCounted(false);
        preferences=getSharedPreferences("amoo_connection",0);
        bootCount=Settings.Global.getInt(getContentResolver(),"boot_count",-1);
        if(bootCount>=0&&preferences.getInt("wake_boot",-2)==bootCount)leases.restoreWake(preferences.getLong("wake_elapsed",-1),SystemClock.elapsedRealtime());
        leases.screen(power.isInteractive(),SystemClock.elapsedRealtime());
        getSystemService(NotificationManager.class).createNotificationChannel(new NotificationChannel("amoo_connection","电脑后台协作",NotificationManager.IMPORTANCE_LOW));
        lastNotifiedText="正在检查任务连接…";
        startForeground(18,notification(lastNotifiedText));
        workerThread=new HandlerThread("StepXAmooConnection");workerThread.start();worker=new Handler(workerThread.getLooper());
        IntentFilter filter=new IntentFilter(Intent.ACTION_SCREEN_OFF);filter.addAction(Intent.ACTION_SCREEN_ON);
        registerReceiver(screen,filter,Context.RECEIVER_NOT_EXPORTED);registered=true;
    }
    private Notification notification(String text){
        PendingIntent open=PendingIntent.getActivity(this,182,new Intent(this,MainActivity.class),PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        PendingIntent stop=PendingIntent.getForegroundService(this,183,new Intent(this,AmooConnectionService.class).putExtra("op","disable").putExtra("request_id","notification-stop"),PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Builder(this,"amoo_connection").setSmallIcon(android.R.drawable.stat_notify_sync).setContentTitle("StepX伴侣 · 后台协作").setContentText(text).setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true).addAction(new Notification.Action.Builder(null,"停止后台协作",stop).build()).build();
    }
    @Override public int onStartCommand(Intent intent,int flags,int startId){
        latestStart=startId;
        if(lastNotifiedText.isEmpty())lastNotifiedText="正在检查任务连接…";
        startForeground(18,notification(lastNotifiedText));
        final String id=bounded(intent==null?null:intent.getStringExtra("request_id"),"local-"+System.currentTimeMillis());
        final String op=intent==null?"status":bounded(intent.getStringExtra("op"),"status");
        final String leaseId=intent==null?"":bounded(intent.getStringExtra("lease_id"),"");
        worker.post(()->handle(op,id,leaseId));
        return START_NOT_STICKY;
    }
    private static String bounded(String value,String fallback){if(value==null)return fallback;return value.substring(0,Math.min(value.length(),100));}
    private boolean enabled(){return preferences.getBoolean("enabled",true);}
    private void verify() throws Exception {
        if(Build.VERSION.SDK_INT!=36||!FINGERPRINT.equals(Build.FINGERPRINT))throw new SecurityException("当前手机固件尚未完成后台协作适配");
        ApplicationInfo app=getPackageManager().getApplicationInfo(NATIVE_PACKAGE,0);
        if(app.uid!=1000||!NATIVE_PATH.equals(app.sourceDir))throw new SecurityException("Amoo 原生服务身份发生变化");
        if(!supported){
            MessageDigest digest=MessageDigest.getInstance("SHA-256");try(InputStream in=new FileInputStream(NATIVE_PATH)){byte[] b=new byte[65536];int n;while((n=in.read(b))>0)digest.update(b,0,n);}
            StringBuilder hash=new StringBuilder();for(byte b:digest.digest())hash.append(String.format(java.util.Locale.ROOT,"%02x",b&255));
            if(!NATIVE_SHA.equals(hash.toString()))throw new SecurityException("Amoo 原生服务版本已变化，请先更新适配");
            ActivityInfo receiver=getPackageManager().getReceiverInfo(new ComponentName(NATIVE_PACKAGE,NATIVE_PACKAGE+".push.PushReceiver"),0);
            if(!receiver.exported||!receiver.enabled||!MESSAGE_PERMISSION.equals(receiver.permission))throw new SecurityException("Amoo 受保护后台入口不匹配");
        }
        for(String permission:PERMISSIONS)if(checkSelfPermission(permission)!=PackageManager.PERMISSION_GRANTED)throw new SecurityException("后台协作缺少安装权限："+permission);
        supported=true;
    }
    private void requireLeaseId(String id){if(!id.matches("[a-zA-Z0-9-]{16,90}"))throw new IllegalArgumentException("任务租约标识无效");}
    private void handle(String op,String id,String leaseId){
        requestId=id;lastError="";long now=SystemClock.elapsedRealtime();
        try{
            if(op.equals("disable")){
                preferences.edit().putBoolean("enabled",false).commit();leases.clear();oneShot.clear();oneShotDone.clear();observedLease="";observedAt=0;
            }else if(op.equals("release")){
                requireLeaseId(leaseId);leases.release(leaseId);oneShot.remove(leaseId);oneShotDone.remove(leaseId);if(observedLease.equals(leaseId)){observedLease="";observedAt=0;}
            }else{
                verify();
                if(op.equals("enable"))preferences.edit().putBoolean("enabled",true).commit();
                else if(!op.equals("status")){
                    requireLeaseId(leaseId);
                    if(!enabled())throw new IllegalStateException("后台协作已由用户关闭；请在 StepX伴侣设置中重新开启");
                    if(op.equals("lease")||op.equals("wakeonce")){leases.acquire(leaseId,now);if(op.equals("wakeonce"))oneShot.add(leaseId);}
                    else if(op.equals("renew"))leases.renew(leaseId,now);
                    else if(op.equals("mark-connected")){
                        if(!leases.has(leaseId,now))throw new IllegalStateException("连接回执对应的租约已过期");
                        observedLease=leaseId;observedAt=now;
                    }else throw new IllegalArgumentException("不支持的后台操作");
                }
            }
        }catch(Exception e){lastError=message(e);if(e instanceof SecurityException){supported=false;leases.clear();}}
        tick();
    }
    private boolean mayRequest(long now){
        oneShot.removeIf(id->!leases.has(id,now));oneShotDone.retainAll(oneShot);
        return leases.count(now)>oneShot.size()||oneShotDone.size()<oneShot.size();
    }
    private void tick(){
        if(closed)return;
        worker.removeCallbacks(pulse);long now=SystemClock.elapsedRealtime();
        if(!enabled())leases.clear();
        boolean active=leases.active(now);
        if(active){
            try{
                verify();
                cpu.acquire(Math.min(AmooLeaseState.TTL_MS+5_000,Math.max(1,leases.expires(now)-now+5_000)));
                long delay=leases.wakeDelay(power.isInteractive(),now);
                if(!closed&&delay==0&&mayRequest(now)){
                    Intent wake=new Intent("com.stepos.push.intent.MESSAGE").setComponent(new ComponentName(NATIVE_PACKAGE,NATIVE_PACKAGE+".push.PushReceiver"));
                    wake.addCategory(NATIVE_PACKAGE);wake.putExtra("method","message");wake.putExtra("message",CloudWakeMessage.create());
                    wake.putExtra("extra_app_push_task_Id","");wake.putExtra("extra_app_push_seq_Id","");
                    sendBroadcast(wake);leases.woke(now);oneShotDone.addAll(oneShot);
                    preferences.edit().putLong("wake_elapsed",now).putInt("wake_boot",bootCount).commit();
                    // Delivery is only a request. Neither network state nor task success is inferred.
                }
            }catch(Exception e){lastError=message(e);if(e instanceof SecurityException)supported=false;leases.clear();active=false;}
        }else leases.screen(power.isInteractive(),now);
        if(!active&&cpu.isHeld())cpu.release();
        long grace=supported?leases.idleGraceRemaining(enabled(),power.isInteractive(),now):0;
        String msg=statusMessage(now);
        String curState=(active?"connected":(grace>0?"grace":"idle"))+":"+enabled()+":"+supported+":"+power.isInteractive()+":"+requestId;
        int curLeaseCount=leases.count(now);
        if(!curState.equals(lastWrittenState)||!msg.equals(lastWrittenMessage)||curLeaseCount!=lastLeaseCount||lastWriteMs==0||now-lastWriteMs>=15_000L){
            writeStatus(now,msg,curState,curLeaseCount);
        }
        if(active||grace>0){
            if(!msg.equals(lastNotifiedText)){
                NotificationManager nm=getSystemService(NotificationManager.class);
                if(nm!=null){nm.notify(18,notification(msg));lastNotifiedText=msg;}
            }
            // Idle grace observes only the existing native window. CPU lock is
            // already released above; no lease, wake request or deadline is added.
            worker.postDelayed(pulse,active?1000:Math.min(1000,grace));
        }else{int start=latestStart;main.post(()->{if(latestStart==start){stopForeground(STOP_FOREGROUND_REMOVE);stopSelf(start);}});}
    }
    private String statusMessage(long now){
        if(!lastError.isEmpty())return lastError;
        if(!enabled())return "后台协作已停止续期；原生连接窗口最多 3 分钟后结束。";
        if(!leases.active(now))return leases.idleGraceRemaining(enabled(),power.isInteractive(),now)>0?
            "任务已结束，等待下一次连接。":"已允许后台协作，等待电脑任务。";
        if(observedAt>0&&now-observedAt<60_000&&leases.has(observedLease,now))return "后台通道已确认可达，任务以实际回执为准。";
        long delay=leases.wakeDelay(power.isInteractive(),now);
        if(!power.isInteractive()&&leases.nativeWindowRemaining(now)==0&&delay>0)return "正在等待熄屏稳定，约 "+((delay+999)/1000)+" 秒后连接。";
        return power.isInteractive()?"任务连接中，等待 Amoo 实际回执。":"已请求 Amoo 后台连接，等待实际任务回执。";
    }
    private void writeStatus(long now,String msg,String curState,int curLeaseCount){try{
        leases.screen(power.isInteractive(),now);boolean active=enabled()&&leases.active(now);
        boolean observed=active&&observedAt>0&&observedAt<=now&&now-observedAt<60_000&&leases.has(observedLease,now);
        JSONObject state=new JSONObject().put("protocol",2).put("requestId",requestId).put("request_id",requestId)
            .put("supported",supported).put("enabled",enabled()).put("active",active).put("bridgeReady",false)
            .put("leaseCount",curLeaseCount).put("leaseExpiresElapsedMs",leases.expires(now))
            .put("uptimeMs",now).put("screenInteractive",power.isInteractive()).put("wakeLockHeld",cpu.isHeld())
            .put("wakeRequested",leases.lastWake()>=0&&leases.nativeWindowRemaining(now)>0).put("wakeRequestedElapsedMs",leases.lastWake()).put("lastWakeRequestElapsedMs",leases.lastRequest())
            .put("nextWakeInMs",active&&mayRequest(now)?leases.wakeDelay(power.isInteractive(),now):-1)
            .put("nativeWindowRemainingMs",leases.nativeWindowRemaining(now))
            .put("idleGraceRemainingMs",supported?leases.idleGraceRemaining(enabled(),power.isInteractive(),now):0).put("observedConnected",observed)
            .put("observedConnectedElapsedMs",observedAt).put("observedConnectionSource",observed?"agentbus_handshake":"")
            .put("lastError",lastError).put("bridge",new JSONObject()).put("message",msg);
        File dir=getExternalFilesDir(null);if(dir==null)throw new IOException("伴侣状态目录不可用");dir.mkdirs();
        File temporary=new File(dir,"amoo-connection.json.tmp"),result=new File(dir,"amoo-connection.json");
        Files.write(temporary.toPath(),state.toString().getBytes(StandardCharsets.UTF_8));
        Files.move(temporary.toPath(),result.toPath(),StandardCopyOption.REPLACE_EXISTING,StandardCopyOption.ATOMIC_MOVE);
        lastWrittenState=curState;lastWrittenMessage=msg;lastLeaseCount=curLeaseCount;lastWriteMs=now;
    }catch(Exception e){lastError=message(e);android.util.Log.e("StepXAmoo","Cannot write task state: "+e.getClass().getSimpleName());}}
    private static String message(Exception e){String text=e.getMessage();return text==null?e.getClass().getSimpleName():text;}
    @Override public void onDestroy(){closed=true;if(registered)unregisterReceiver(screen);if(worker!=null)worker.removeCallbacksAndMessages(null);if(cpu!=null&&cpu.isHeld())cpu.release();if(workerThread!=null)workerThread.quitSafely();super.onDestroy();}
    @Override public IBinder onBind(Intent intent){return null;}
}
