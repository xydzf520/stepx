package com.stepx.companion.app;

import android.Manifest;
import android.app.*;
import android.content.*;
import android.content.pm.PackageManager;
import android.hardware.*;
import android.os.*;
import android.telephony.*;
import org.json.JSONObject;
import org.json.JSONArray;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.Arrays;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** User-started local runtime. It never starts at boot and never accepts arbitrary paths or code. */
public final class CompanionService extends Service implements SensorEventListener {
    private final Handler h=new Handler(Looper.getMainLooper());
    private final ExecutorService statusIo=Executors.newSingleThreadExecutor();
    private final long serviceCreatedUptimeMs=SystemClock.elapsedRealtime();
    private int serviceStartCount,lastStartId;
    private SensorManager sensors;
    private Sensor accel,gravity,gyro,rotation;
    private final float[] rotationMatrix=new float[9];
    private double[] backNormal;
    private final GameCore.FaceExit faceExit=new GameCore.FaceExit();
    private long rotationCount,bestScore,lastGameScore;
    private BackScreen screen;
    private PowerManager power;
    private PowerManager.WakeLock lock;
    private TelephonyManager phone;
    private PhoneStateListener phoneListener;
    private GameCore game;
    private final GameCore.TapDetector taps=new GameCore.TapDetector();
    private DisplayConfig config;
    private DisplayConfig resumeConfig;
    private long resumeElapsed,lastLockRenew,endedAt,lastSubmitted;
    private String resumeJobId="";
    private JSONObject lastTest;
    private String mode="stopped",requestId="",jobId="",error="",pause="",title="";
    private long lastTick,started,lastStatus,lastStatusWrite,lastFrame,accelCount,gravityCount,gyroCount,frameCount,tapTriggers;
    private String lastStatusKey="";
    private float gx,gy,gz=9.8f,gyroMagnitude;
    private double tilt,neutral;
    private boolean registered=false,motionRegistered=false,standby=false,synthetic=false,calling=false,restored=true,pausedOutput=false,commandReady=false;
    private volatile int cachedThermalStatus=0;
    private boolean cachedPowerSave=false;
    private long lastPowerSaveCheck=0;
    private int battery=100;
    private int[] previousFrame;
    private int brightness=5;
    private final BroadcastReceiver systemEvents=new BroadcastReceiver(){public void onReceive(Context c,Intent i){
        if(Intent.ACTION_BATTERY_CHANGED.equals(i.getAction())){battery=i.getIntExtra(BatteryManager.EXTRA_LEVEL,100)*100/Math.max(1,i.getIntExtra(BatteryManager.EXTRA_SCALE,100));}
        if(Intent.ACTION_USER_PRESENT.equals(i.getAction())&&game!=null&&!synthetic){pause="手机已解锁，三敲背面重新开始";finishGame(false);}
    }};
    private final PowerManager.OnThermalStatusChangedListener thermalListener=status->{cachedThermalStatus=status;if(status>=PowerManager.THERMAL_STATUS_SEVERE){pause="手机温度较高，已暂停背屏";stopOutput(false);}};
    private boolean isPowerSave(long now){if(now-lastPowerSaveCheck>=1000){cachedPowerSave=power.isPowerSaveMode();lastPowerSaveCheck=now;}return cachedPowerSave;}
    @Override public void onCreate(){super.onCreate();
        android.util.Log.i("StepXCompanion","create instance="+serviceCreatedUptimeMs);
        NotificationManager nm=getSystemService(NotificationManager.class);nm.createNotificationChannel(new NotificationChannel("backscreen","背屏与体感游戏",NotificationManager.IMPORTANCE_LOW));
        startForeground(17,notification("等待指令"));
        sensors=getSystemService(SensorManager.class);accel=sensors.getDefaultSensor(Sensor.TYPE_ACCELEROMETER);gravity=sensors.getDefaultSensor(Sensor.TYPE_GRAVITY);gyro=sensors.getDefaultSensor(Sensor.TYPE_GYROSCOPE);rotation=sensors.getDefaultSensor(Sensor.TYPE_GAME_ROTATION_VECTOR);
        bestScore=getSharedPreferences("game_scores",0).getLong("best",0);lastGameScore=getSharedPreferences("game_scores",0).getLong("last",0);
        power=getSystemService(PowerManager.class);lock=power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,"StepX:BackscreenGame");lock.setReferenceCounted(false);
        cachedThermalStatus=power.getCurrentThermalStatus();cachedPowerSave=power.isPowerSaveMode();lastPowerSaveCheck=SystemClock.elapsedRealtime();
        IntentFilter f=new IntentFilter(Intent.ACTION_BATTERY_CHANGED);f.addAction(Intent.ACTION_USER_PRESENT);registerReceiver(systemEvents,f,Context.RECEIVER_NOT_EXPORTED);power.addThermalStatusListener(thermalListener);
        if(checkSelfPermission(Manifest.permission.READ_PHONE_STATE)==PackageManager.PERMISSION_GRANTED){
            phone=getSystemService(TelephonyManager.class);phoneListener=new PhoneStateListener(){@Override public void onCallStateChanged(int state,String unused){calling=state!=TelephonyManager.CALL_STATE_IDLE;}};phone.listen(phoneListener,PhoneStateListener.LISTEN_CALL_STATE);
        }
        lastTick=SystemClock.elapsedRealtime();h.post(tick);
    }
    private Notification notification(String text){
        Intent stop=new Intent(this,CompanionService.class).putExtra("op","stop");
        PendingIntent pi=PendingIntent.getService(this,1,stop,PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        PendingIntent open=PendingIntent.getActivity(this,2,new Intent(this,MainActivity.class),PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Builder(this,"backscreen").setSmallIcon(android.R.drawable.ic_media_play).setContentTitle("StepX伴侣").setContentText(text).setContentIntent(open).setOngoing(true).addAction(new Notification.Action.Builder(null,"停止",pi).build()).build();
    }
    private String notificationText(){return mode.equals("standby")?"三敲背面开始游戏 · 待机不持唤醒锁":mode.equals("game")?"重力对齐 · 翻到手机正面结束":mode.equals("animation")?"背屏："+title:"已停止";}
    private void notifyState(){getSystemService(NotificationManager.class).notify(17,notification(notificationText()));}
    @Override public int onStartCommand(Intent intent,int flags,int startId){
        serviceStartCount++;lastStartId=startId;
        android.util.Log.i("StepXCompanion","start instance="+serviceCreatedUptimeMs+" startId="+startId+" mode="+mode);
        if(intent==null){stopSelf();return START_NOT_STICKY;}
        String op=intent.getStringExtra("op");requestId=intent.getStringExtra("request_id");if(requestId==null)requestId="local-"+System.currentTimeMillis();if(requestId.length()>100)requestId=requestId.substring(0,100);
        error="";commandReady=false;
        boolean preserveOutputOnFailure="apply".equals(op)&&!mode.equals("stopped");
        try{
            if(op==null)op="status";
            switch(op){
                case "status": if(getSharedPreferences("backscreen_restore",0).getBoolean("pending",false)&&screen==null)connect();break;
                case "stop": connectIfPending();standby=false;resumeConfig=null;stopOutput(true);mode="stopped";unregister();break;
                case "standby": connect();standby=true;if(game!=null)finishGame(false);if(!mode.equals("animation"))mode="standby";pause="";register(true);taps.reset();break;
                case "game": connect();standby=true;register(true);beginGame(false);break;
                case "test": if(!intent.getBooleanExtra("test_mode",false))throw new IOException("合成测试必须显式设置 test_mode");connect();register(true);standby=false;beginGame(true);break;
                case "apply": {
                    if(!"incoming.json".equals(intent.getStringExtra("config_name")))throw new IOException("仅接受 incoming.json");
                    File incoming=new File(getExternalFilesDir(null),"incoming.json");
                    if(!Files.isRegularFile(incoming.toPath(),java.nio.file.LinkOption.NOFOLLOW_LINKS)||incoming.length()>262144)throw new IOException("配置文件无效或超过256KiB");
                    byte[] raw=Files.readAllBytes(incoming.toPath());if(raw.length>262144)throw new IOException("配置文件过大");
                    DisplayConfig next=DisplayConfig.parse(new String(raw,StandardCharsets.UTF_8));
                    if(!next.jobId.isEmpty()&&!requestId.equals(next.jobId))throw new IOException("配置与本次请求不匹配");
                    // A rejected file must not interrupt an already valid applied display.
                    // Only failures after validation/starting the replacement need restoration.
                    preserveOutputOnFailure=false;
                    connect();register(standby);rememberGameScore();stopOutput(false);resumeConfig=null;config=next;game=null;synthetic=false;mode="animation";title=next.title;jobId=requestId;brightness=next.brightness;started=SystemClock.elapsedRealtime();pause="";frameCount=0;lastSubmitted=0;
                    atomic(new File(getFilesDir(),"accepted.json"),raw);screen.begin(brightness,false);restored=false;pausedOutput=false;previousFrame=null;h.removeCallbacks(tick);h.post(tick);break;
                }
                default:throw new IOException("不支持的操作");
            }
            // stopSelf(oldStartId) need not destroy this instance if another command
            // was already queued. Reassert FGS after an idle status/stop removed it.
            // This retains the manifest's specialUse type; it is not a freezer bypass.
            if(!mode.equals("stopped"))startForeground(17,notification(notificationText()));
            commandReady=true;
        }catch(Throwable e){error=message(e);if(!preserveOutputOnFailure){stopOutput(true);mode="stopped";standby=false;unregister();}}
        android.util.Log.i("StepXCompanion","ack instance="+serviceCreatedUptimeMs+" startId="+startId+" mode="+mode+" fgsType="+getForegroundServiceType());
        notifyState();writeStatus(true);if(mode.equals("stopped")){stopForeground(STOP_FOREGROUND_REMOVE);stopSelf(startId);}return START_NOT_STICKY;
    }
    private static String message(Throwable e){while(e.getCause()!=null)e=e.getCause();return e.getClass().getSimpleName()+": "+String.valueOf(e.getMessage());}
    private void connect()throws Exception{
        if(screen==null){screen=new BackScreen(this);if(screen.pending()){screen.restore(true);restored=true;}}
    }
    private void connectIfPending()throws Exception{if(screen==null&&getSharedPreferences("backscreen_restore",0).getBoolean("pending",false))connect();}
    private void register(boolean motion)throws Exception{
        if(registered&&motionRegistered==motion)return;unregister();
        if(gravity==null||(motion&&accel==null))throw new IOException("手机缺少加速度或重力传感器");
        if(!sensors.registerListener(this,gravity,motion?20000:40000,h)||(motion&&!sensors.registerListener(this,accel,10000,h))){sensors.unregisterListener(this);throw new IOException("无法订阅手机传感器");}
        if(motion&&gyro!=null)sensors.registerListener(this,gyro,20000,h);if(motion&&rotation!=null)sensors.registerListener(this,rotation,20000,h);registered=true;motionRegistered=motion;
    }
    private void unregister(){if(registered)sensors.unregisterListener(this);registered=false;}
    private void beginGame(boolean test)throws Exception{
        if(config!=null){resumeConfig=config;resumeElapsed=SystemClock.elapsedRealtime()-started;resumeJobId=jobId;}
        rememberGameScore();stopOutput(false);game=new GameCore(test);config=null;synthetic=test;mode="game";title="重力对齐";jobId=requestId;started=SystemClock.elapsedRealtime();neutral=tilt;pause="";taps.reset();frameCount=0;lastSubmitted=0;endedAt=0;faceExit.reset();
        screen.begin(brightness,true);restored=false;pausedOutput=false;previousFrame=null;lastLockRenew=0;notifyState();h.removeCallbacks(tick);h.post(tick);
    }
    private void finishGame(boolean clearStandby){
        boolean wasTest=synthetic;boolean passed=game!=null&&game.phase.equals("won");long score=game==null?0:game.score;rememberGameScore();
        if(clearStandby){standby=false;resumeConfig=null;}
        game=null;synthetic=false;stopOutput(resumeConfig==null);
        if(wasTest)try{lastTest=new JSONObject().put("synthetic",true).put("passed",passed).put("levels",passed?5:0).put("score",score).put("frames",frameCount).put("restored",restored).put("sensorInputWasSynthetic",true).put("realSensorEvents",accelCount);getSharedPreferences("test_result",0).edit().putString("last",lastTest.toString()).apply();}catch(Exception ignored){}
        if(resumeConfig!=null){config=resumeConfig;resumeConfig=null;brightness=config.brightness;title=config.title;jobId=resumeJobId;frameCount=0;lastSubmitted=0;mode="animation";started=SystemClock.elapsedRealtime()-resumeElapsed;try{screen.begin(brightness,false);restored=false;previousFrame=null;register(standby);}catch(Exception e){error=message(e);standby=false;stopOutput(true);mode="stopped";}}
        else {mode=standby?"standby":"stopped";title=standby?"三敲待机":"";}
        taps.reset();notifyState();if(mode.equals("stopped")){unregister();writeStatus(true);stopForeground(STOP_FOREGROUND_REMOVE);stopSelf();}
    }
    private void rememberGameScore(){
        if(game!=null&&!synthetic){lastGameScore=game.score;bestScore=Math.max(bestScore,game.score);getSharedPreferences("game_scores",0).edit().putLong("best",bestScore).putLong("last",lastGameScore).apply();}
    }
    private void stopOutput(boolean forget){
        if(lock!=null&&lock.isHeld())lock.release();
        if(screen!=null)try{screen.restore(forget);restored=true;}catch(Exception e){error=message(e);restored=false;}
        if(forget){rememberGameScore();game=null;config=null;previousFrame=null;}pausedOutput=false;
    }
    private final Runnable tick=new Runnable(){public void run(){
        long now=SystemClock.elapsedRealtime();double dt=Math.max(0,Math.min(.2,(now-lastTick)/1000.));lastTick=now;
        try{
            if(game!=null&&!synthetic&&!game.phase.equals("countdown")&&faceExit.sample(now,gz,backNormal)){pause="本局已结束，已恢复背屏";finishGame(false);}
            if(mode.equals("standby"))pause=gravityCount==0?"等待姿态传感器":gz>=7.5?"请拿起手机，将背屏朝向自己后敲三下":"";
            if(mode.equals("game")||mode.equals("animation")){
                String gate=calling?"来电或通话中":isPowerSave(now)?"系统省电模式":cachedThermalStatus>=PowerManager.THERMAL_STATUS_SEVERE?"手机温度较高":gravityCount==0?"等待姿态传感器":gz>=7.5?"请将背屏朝向自己":"";
                if(!gate.isEmpty()){
                    pause=gate;if(!pausedOutput){screen.restore(false);pausedOutput=true;restored=true;if(lock.isHeld())lock.release();}
                }else{
                    if(pausedOutput){screen.begin(brightness,game!=null);pausedOutput=false;restored=false;previousFrame=null;lastLockRenew=0;}
                    pause="";
                    boolean needsCpu=game!=null||(config!=null&&(config.frames==null||config.frames.length>1));
                    if(needsCpu&&(!lock.isHeld()||now-lastLockRenew>30000)){lock.acquire(60000L);lastLockRenew=now;}else if(!needsCpu&&lock.isHeld())lock.release();
                    if(game!=null){
                        if(game.phase.equals("countdown"))neutral+=.1*(tilt-neutral);game.update(dt,tilt-neutral);if(!synthetic&&game.score>bestScore){bestScore=game.score;getSharedPreferences("game_scores",0).edit().putLong("best",bestScore).apply();}if(now-lastFrame>=83)render(game.frame(8),now);
                        if(game.ended()){if(endedAt==0)endedAt=now;if(synthetic&&now-started>=7000)finishGame(false);}
                    }else if(config!=null){
                        if(config.durationSecs>0&&now-started>=config.durationSecs*1000L){stopOutput(true);mode="stopped";unregister();writeStatus(true);stopForeground(STOP_FOREGROUND_REMOVE);stopSelf();}
                        else if(now-lastFrame>=1000/config.fps)render(config.frame(now-started,battery),now);
                    }
                }
            }
        }catch(Throwable e){error=message(e);standby=false;stopOutput(true);mode="stopped";unregister();writeStatus(true);stopForeground(STOP_FOREGROUND_REMOVE);stopSelf();}
        writeStatus(false);h.postDelayed(this,("standby".equals(mode)||pausedOutput)?200:40);
    }};
    private void render(int[]f,long now)throws Exception{lastFrame=now;if(!Arrays.equals(previousFrame,f)){screen.push(f);previousFrame=f;frameCount++;lastSubmitted=now;}}
    @Override public void onSensorChanged(SensorEvent e){
        if(e.sensor.getType()==Sensor.TYPE_GRAVITY){gravityCount++;gx=e.values[0];gy=e.values[1];gz=e.values[2];
            // Firmware rotation=0 preserves row-major columns. In a back view, a
            // lowered right edge yields positive Android gravity X, so use +gx.
            tilt=GameCore.backViewTilt(gx,gy,gz);
        }else if(e.sensor.getType()==Sensor.TYPE_GAME_ROTATION_VECTOR){rotationCount++;SensorManager.getRotationMatrixFromVector(rotationMatrix,e.values);backNormal=new double[]{rotationMatrix[2],rotationMatrix[5],rotationMatrix[8]};}
        else if(e.sensor.getType()==Sensor.TYPE_GYROSCOPE){gyroCount++;gyroMagnitude=(float)Math.sqrt(e.values[0]*e.values[0]+e.values[1]*e.values[1]+e.values[2]*e.values[2]);}
        else if(e.sensor.getType()==Sensor.TYPE_ACCELEROMETER){accelCount++;boolean trigger=taps.sample(e.timestamp/1000000L,e.values[2]-gz,Math.hypot(e.values[0]-gx,e.values[1]-gy),gyroMagnitude,gravityCount>0&&gz<7.5);
            if(trigger&&!synthetic&&game==null){tapTriggers++;try{if(standby&&!calling&&!isPowerSave(SystemClock.elapsedRealtime()))beginGame(false);}catch(Exception failure){error=message(failure);finishGame(true);}}
        }
    }
    @Override public void onAccuracyChanged(Sensor s,int accuracy){}
    private void writeStatus(boolean force){
        long now=SystemClock.elapsedRealtime();if(!force&&now-lastStatus<250)return;lastStatus=now;
        String stateKey=requestId+"|"+jobId+"|"+commandReady+"|"+mode+"|"+(game!=null?game.phase:mode)+"|"+title+"|"+frameCount+"|"+error+"|"+pause+"|"+restored+"|"+synthetic+"|"+standby+"|"+taps.count+"|"+tapTriggers+"|"+taps.lastReason+"|"+taps.recent.size()+"|"+serviceStartCount+"|"+lastStartId+"|"+(gravityCount>0)+"|"+(gz>=7.5f)+"|"+(game==null?0:game.score)+"|"+bestScore+"|"+lastGameScore;
        if(!force&&stateKey.equals(lastStatusKey)&&now-lastStatusWrite<2000)return;
        lastStatusKey=stateKey;lastStatusWrite=now;
        try{
            JSONObject j=new JSONObject().put("protocol",1).put("app","StepX伴侣").put("version","0.1.2").put("uid",android.os.Process.myUid()).put("requestId",requestId).put("jobId",jobId).put("commandReady",commandReady).put("running",!mode.equals("stopped")).put("mode",mode).put("phase",game!=null?game.phase:mode).put("appliedTitle",title).put("frameCount",frameCount).put("lastFrameUptimeMs",lastSubmitted).put("lastError",error).put("pausedReason",pause).put("settingsRestored",restored).put("synthetic",synthetic).put("standbyEnabled",standby).put("wakeLockHeld",lock!=null&&lock.isHeld()).put("timestampMs",System.currentTimeMillis()).put("uptimeMs",now).put("tiltDegrees",tilt-neutral).put("gravityZ",gz).put("tapCount",taps.count).put("tapTriggers",tapTriggers).put("outputAccepted",frameCount>0&&!pausedOutput&&(mode.equals("game")||mode.equals("animation"))).put("outputVerified",false);
            j.put("serviceCreatedUptimeMs",serviceCreatedUptimeMs).put("serviceStartCount",serviceStartCount).put("lastStartId",lastStartId).put("foregroundServiceType",getForegroundServiceType());
            j.put("sensors",new JSONObject().put("accelerometer",accel!=null).put("gravity",gravity!=null).put("gyroscope",gyro!=null).put("accelEvents",accelCount).put("gravityEvents",gravityCount).put("gyroEvents",gyroCount).put("gameRotationVector",rotation!=null).put("rotationEvents",rotationCount).put("registered",registered).put("motionRegistered",motionRegistered));
            if(lastTest==null){String raw=getSharedPreferences("test_result",0).getString("last","");if(!raw.isEmpty())lastTest=new JSONObject(raw);}if(lastTest!=null)j.put("lastTest",lastTest);
            j.put("manageBackScreenGranted",checkSelfPermission("com.stepos.permission.MANAGE_BACK_SCREEN")==PackageManager.PERMISSION_GRANTED).put("permissionGranted",checkSelfPermission("com.stepos.permission.MANAGE_BACK_SCREEN")==PackageManager.PERMISSION_GRANTED).put("restorePending",getSharedPreferences("backscreen_restore",0).getBoolean("pending",false)).put("rawTiltDegrees",tilt);
            j.put("score",game==null?0:game.score).put("bestScore",bestScore).put("lastGameScore",lastGameScore).put("round",game==null?1:game.round).put("completedLevels",game==null?0:game.completedLevels).put("facingDot",faceExit.facingDot).put("faceExitArmed",faceExit.armed).put("lastKnockReason",taps.lastReason);
            String tapHint=game!=null?"左右倾斜对齐，翻到手机正面结束":gz>=7.5?"请拿起手机，让背屏朝向自己后敲三下":"连续敲三下背面，间隔不超过约半秒";
            j.put("tapHint",tapHint);
            JSONArray diagnostics=new JSONArray();for(GameCore.TapDetector.Impulse d:taps.recent)diagnostics.put(new JSONObject().put("uptimeMs",d.at).put("normalPeak",d.normal).put("lateralPeak",d.lateral).put("gyroPeak",d.gyro).put("reason",d.reason));j.put("tapDiagnostics",diagnostics);
            if(game!=null)j.put("level",Math.min(5,game.level)).put("barWidth",game.width).put("slotWidth",game.width).put("barX",game.x).put("slotX",game.target).put("barY",game.y).put("misses",game.misses).put("dropSeconds",game.dropSeconds());
            final File dir=getExternalFilesDir(null);
            if(dir!=null){
                final byte[] payload=j.toString().getBytes(StandardCharsets.UTF_8);
                statusIo.execute(()->{try{atomic(new File(dir,"status.json"),payload);}catch(Exception ignored){}});
            }
        }catch(Exception ignored){}
    }
    static void atomic(File file,byte[]data)throws Exception{File temp=new File(file.getParentFile(),file.getName()+".tmp");try(FileOutputStream out=new FileOutputStream(temp)){out.write(data);out.getFD().sync();}if(!temp.renameTo(file))throw new IOException("原子写入失败");}
    @Override public void onDestroy(){android.util.Log.i("StepXCompanion","destroy instance="+serviceCreatedUptimeMs+" mode="+mode);h.removeCallbacksAndMessages(null);unregister();stopOutput(true);mode="stopped";writeStatus(true);statusIo.shutdown();try{unregisterReceiver(systemEvents);}catch(Exception ignored){}power.removeThermalStatusListener(thermalListener);if(phone!=null&&phoneListener!=null)phone.listen(phoneListener,PhoneStateListener.LISTEN_NONE);super.onDestroy();}
    @Override public IBinder onBind(Intent i){return null;}
}
