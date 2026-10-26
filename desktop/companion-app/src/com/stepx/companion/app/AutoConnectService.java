package com.stepx.companion.app;

import android.app.*;
import android.content.*;
import android.content.pm.PackageManager;
import android.net.*;
import android.os.*;
import android.provider.Settings;
import java.util.HashSet;

/** Rejoins Android's authenticated wireless transport. Never trusts a Wi-Fi network or an ADB key. */
public final class AutoConnectService extends Service {
    private static final String PREFS="computer_connection",CHANNEL="computer_connection";
    private final Handler main=new Handler(Looper.getMainLooper());
    private final HashSet<Network> networks=new HashSet<>();
    private ConnectivityManager manager;
    private boolean registered;
    private long permissionDeadline;
    private final Runnable retryPermission=()->{for(Network network:new HashSet<>(networks))enableOnNetwork(network);};
    private FileObserver linkObserver;
    private final Runnable refreshLink=()->showStatus(state(this));
    // Only the connection preference lives in device-protected storage. Conversations, keys
    // and files remain credential encrypted; a user's explicit pause survives upgrades.
    private static volatile SharedPreferences cachedPrefs;
    private static synchronized SharedPreferences prefs(Context c){
        if(cachedPrefs!=null)return cachedPrefs;
        Context device=c.createDeviceProtectedStorageContext();
        SharedPreferences stored=device.getSharedPreferences(PREFS,0);
        if(c.getSystemService(UserManager.class).isUserUnlocked()&&!stored.contains("migrated")){
            SharedPreferences old=c.getSharedPreferences(PREFS,0);
            SharedPreferences.Editor edit=stored.edit().putBoolean("migrated",true);
            if(!stored.contains("auto_connect")&&old.contains("auto_connect"))edit.putBoolean("auto_connect",old.getBoolean("auto_connect",false));
            edit.commit();
        }
        if(stored.contains("migrated"))cachedPrefs=stored;
        return stored;
    }
    static void rememberFirstConnection(Context c,boolean online){
        // A live receipt comes from the already-authorized desktop. Never override a user's pause.
        if(online&&!prefs(c).contains("auto_connect")&&c.checkSelfPermission("android.permission.WRITE_SECURE_SETTINGS")==PackageManager.PERMISSION_GRANTED)setEnabled(c,true);
    }
    public static boolean enabled(Context c){return prefs(c).getBoolean("auto_connect",false);}
    public static String state(Context c){return prefs(c).getString("state",enabled(c)?"等待同一 Wi-Fi 内的电脑":"自动连接已暂停");}
    public static void setEnabled(Context c,boolean on){
        prefs(c).edit().putBoolean("auto_connect",on).putString("state",on?"正在准备自动连接…":"自动连接已暂停").apply();
        if(on)resume(c);else c.stopService(new Intent(c,AutoConnectService.class));
    }
    public static void resume(Context c){if(enabled(c))try{c.startForegroundService(new Intent(c,AutoConnectService.class));}catch(Exception e){prefs(c).edit().putString("state","请打开伴侣一次，恢复后台自动连接").apply();}}
    private void status(String value){if(!value.equals(state(this)))prefs(this).edit().putString("state",value).apply();showStatus(value);}
    private void showStatus(String value){
        TransferStore.Link link=new TransferStore(this).link();
        getSystemService(NotificationManager.class).notify(104,notification(link.online?"已连接 · "+link.name:value));
        main.removeCallbacks(refreshLink);
        if(link.online)main.postDelayed(refreshLink,31000);
    }
    private Notification notification(String value){
        PendingIntent open=PendingIntent.getActivity(this,0,new Intent(this,MainActivity.class),PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        PendingIntent pause=PendingIntent.getService(this,1,new Intent(this,AutoConnectService.class).setAction("pause"),PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Builder(this,CHANNEL).setSmallIcon(getApplicationInfo().icon).setContentTitle("电脑自动连接").setContentText(value).setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true).addAction(new Notification.Action.Builder(null,"暂停",pause).build()).build();
    }
    @Override public void onCreate(){super.onCreate();getSystemService(NotificationManager.class).createNotificationChannel(new NotificationChannel(CHANNEL,"电脑自动连接",NotificationManager.IMPORTANCE_LOW));}
    private final ConnectivityManager.NetworkCallback callback=new ConnectivityManager.NetworkCallback(){
        @Override public void onAvailable(Network network){main.post(()->{if(networks.add(network))main.postDelayed(()->enableOnNetwork(network),1500);});}
        @Override public void onLost(Network network){main.post(()->{networks.remove(network);if(networks.isEmpty())status("等待 Wi-Fi · 已配对记录保留");});}
    };
    private void enableOnNetwork(Network network){
        if(!enabled(this)||!networks.contains(network))return;
        if(Settings.Global.getInt(getContentResolver(),"adb_wifi_enabled",0)!=1&&checkSelfPermission("android.permission.WRITE_SECURE_SETTINGS")!=PackageManager.PERMISSION_GRANTED){if(SystemClock.elapsedRealtime()<permissionDeadline){status("正在恢复开机连接权限…");main.removeCallbacks(retryPermission);main.postDelayed(retryPermission,5000);}else status("无线权限未恢复 · 请在连接管理查看详情");return;}
        try{
            // Android itself checks its remembered Wi-Fi networks. A new network still needs its system confirmation.
            if(Settings.Global.getInt(getContentResolver(),"adb_wifi_enabled",0)!=1)Settings.Global.putInt(getContentResolver(),"adb_wifi_enabled",1);
            main.postDelayed(()->{if(enabled(this)&&networks.contains(network))status(Settings.Global.getInt(getContentResolver(),"adb_wifi_enabled",0)==1?"等待已配对电脑 · 电脑打开后自动连接":"请在手机确认当前 Wi-Fi 的无线调试授权");},2000);
        }catch(Exception e){status("无线连接未就绪 · 请查看连接管理");}
    }
    @Override public int onStartCommand(Intent intent,int flags,int id){
        startForeground(104,notification(state(this)));
        if(intent!=null&&"pause".equals(intent.getAction())){setEnabled(this,false);return START_NOT_STICKY;}
        if(!enabled(this)){stopSelf();return START_NOT_STICKY;}
        if(!registered){permissionDeadline=SystemClock.elapsedRealtime()+180000;manager=getSystemService(ConnectivityManager.class);try{manager.registerNetworkCallback(new NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_WIFI).build(),callback);registered=true;}catch(Exception e){status("网络监听未启动 · 请重新开启自动连接");stopSelf();return START_NOT_STICKY;}}
        if(linkObserver==null&&getSystemService(UserManager.class).isUserUnlocked()){java.io.File dir=getExternalFilesDir(null);if(dir!=null){dir.mkdirs();linkObserver=new FileObserver(dir,FileObserver.CLOSE_WRITE|FileObserver.MOVED_TO){@Override public void onEvent(int event,String path){if("desktop-link.json".equals(path))main.post(refreshLink);}};linkObserver.startWatching();}}
        showStatus(state(this));
        return START_STICKY;
    }
    @Override public void onDestroy(){if(linkObserver!=null)linkObserver.stopWatching();main.removeCallbacksAndMessages(null);if(registered)try{manager.unregisterNetworkCallback(callback);}catch(Exception ignored){}super.onDestroy();}
    @Override public IBinder onBind(Intent intent){return null;}
}
