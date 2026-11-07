package com.stepx.companion.neo;
import android.app.*;
import android.content.*;
import android.os.*;
import org.json.JSONObject;
/** Visible foreground lifetime and local stop control; no automatic task restart. */
public final class CompanionMdiService extends Service {
    private static volatile CompanionMdiService live;
    private PowerManager.WakeLock cpu;
    public void onCreate(){super.onCreate();live=this;getSystemService(NotificationManager.class).createNotificationChannel(new NotificationChannel("stepx-mdi","后台虚拟屏任务",NotificationManager.IMPORTANCE_LOW));startForeground(183,notification("正在准备后台任务",false));cpu=getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,"StepX:MDI");cpu.acquire(750000);}
    private Notification notification(String text,boolean paused){
        Intent open=getPackageManager().getLaunchIntentForPackage(getPackageName());
        Notification.Builder b=new Notification.Builder(this,"stepx-mdi").setSmallIcon(getApplicationInfo().icon).setContentTitle("StepX伴侣 · 后台虚拟屏").setContentText(text).setOnlyAlertOnce(true).setOngoing(true);
        if(open!=null)b.setContentIntent(PendingIntent.getActivity(this,181,open,PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT));
        for(String op:new String[]{paused?"resume":"pause","stop"}){PendingIntent p=PendingIntent.getService(this,op.hashCode(),new Intent(this,CompanionMdiService.class).setAction(op),PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT);b.addAction(new Notification.Action.Builder(null,op.equals("stop")?"停止":paused?"继续":"暂停",p).build());}
        return b.build();
    }
    static void changed(Context c,String text,boolean paused){CompanionMdiService s=live;if(s!=null)s.getSystemService(NotificationManager.class).notify(183,s.notification(text,paused));}
    public int onStartCommand(Intent i,int f,int id){CompanionMdiRuntime r=CompanionMdiRuntime.get(this);String op=i==null?null:i.getAction();if(op==null)r.begin();else new Thread(()->{try{r.request(new JSONObject().put("op",op).put("requestId",r.status().optString("requestId")).put("userConfirmed",true));}catch(Exception ignored){}}).start();return START_NOT_STICKY;}
    public void onDestroy(){CompanionMdiRuntime.get(this).serviceStopped();live=null;if(cpu!=null&&cpu.isHeld())cpu.release();super.onDestroy();}
    public IBinder onBind(Intent i){return null;}
}
