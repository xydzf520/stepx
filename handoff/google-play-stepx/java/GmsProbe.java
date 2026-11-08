package stepx;
import java.lang.reflect.*;
import java.util.*;
import java.security.MessageDigest;
import android.content.Context;
import android.content.pm.*;
public final class GmsProbe {
 static Object field(Object o,String n)throws Exception{Field f=o.getClass().getDeclaredField(n);f.setAccessible(true);return f.get(o);}
 static String digest(byte[] b)throws Exception {byte[] d=MessageDigest.getInstance("SHA-256").digest(b);StringBuilder s=new StringBuilder();for(byte v:d)s.append(String.format("%02x",v&255));return s.toString();}
 public static String run() {
  try {
   ClassLoader cl=Thread.currentThread().getContextClassLoader();
   Class<?> ls=Class.forName("com.android.server.LocalServices",false,cl);
   Class<?> pi=Class.forName("com.android.server.pm.permission.PermissionManagerServiceInterface",false,cl);
   Object permission=ls.getMethod("getService",Class.class).invoke(null,pi);
   Object service=field(permission,"service"),policy=field(permission,"policy"),lock=field(service,"stateLock"),persistence=field(service,"persistence"),allPolicy=field(service,"policy");
   Context context=(Context)service.getClass().getMethod("getContext").invoke(service);PackageManager pm=context.getPackageManager();
   String[] names={"com.google.android.gms","com.android.vending"};int[] uids=new int[names.length];String[][] perms={{"android.permission.ACCESS_BROADCAST_RESPONSE_STATS","android.permission.ACCESS_CACHE_FILESYSTEM","android.permission.ACCESS_CONTEXT_HUB","android.permission.ACCESS_FPS_COUNTER","android.permission.ACCESS_NETWORK_CONDITIONS","android.permission.ACCESS_VIBRATOR_STATE","android.permission.ACTIVITY_EMBEDDING","android.permission.ALLOCATE_AGGRESSIVE","android.permission.BACKUP","android.permission.BLUETOOTH_PRIVILEGED","android.permission.BROADCAST_CLOSE_SYSTEM_DIALOGS","android.permission.CALL_AUDIO_INTERCEPTION","android.permission.CALL_PRIVILEGED","android.permission.CAPTURE_AUDIO_HOTWORD","android.permission.CAPTURE_AUDIO_OUTPUT","android.permission.CHANGE_COMPONENT_ENABLED_STATE","android.permission.CHANGE_DEVICE_IDLE_TEMP_WHITELIST","android.permission.CHECK_REMOTE_LOCKSCREEN","android.permission.COMPANION_APPROVE_WIFI_CONNECTIONS","android.permission.CONFIGURE_FACTORY_RESET_PROTECTION","android.permission.CONNECTIVITY_USE_RESTRICTED_NETWORKS","android.permission.CONTROL_DISPLAY_SATURATION","android.permission.CONTROL_INCALL_EXPERIENCE","android.permission.CONTROL_KEYGUARD_SECURE_NOTIFICATIONS","android.permission.DISPATCH_PROVISIONING_MESSAGE","android.permission.DOMAIN_VERIFICATION_AGENT","android.permission.DUMP","android.permission.EMERGENCY_INSTALL_PACKAGES","android.permission.FORCE_STOP_PACKAGES","android.permission.GET_APP_OPS_STATS","android.permission.GET_ROLE_HOLDERS","android.permission.INSTALL_LOCATION_TIME_ZONE_PROVIDER_SERVICE","android.permission.INTENT_FILTER_VERIFICATION_AGENT","android.permission.INTERACT_ACROSS_USERS","android.permission.INVOKE_CARRIER_SETUP","android.permission.LOCAL_MAC_ADDRESS","android.permission.LOCATION_BYPASS","android.permission.LOCATION_HARDWARE","android.permission.LOCK_DEVICE","android.permission.LOG_COMPAT_CHANGE","android.permission.MANAGE_ADVANCED_PROTECTION_MODE","android.permission.MANAGE_FACTORY_RESET_PROTECTION","android.permission.MANAGE_GAME_ACTIVITY","android.permission.MANAGE_GAME_MODE","android.permission.MANAGE_INTRUSION_DETECTION_STATE","android.permission.MANAGE_ROLLBACKS","android.permission.MANAGE_SECURE_LOCK_DEVICE","android.permission.MANAGE_SOUND_TRIGGER","android.permission.MANAGE_SUBSCRIPTION_PLANS","android.permission.MANAGE_TIME_AND_ZONE_DETECTION","android.permission.MANAGE_USB","android.permission.MANAGE_USERS","android.permission.MANAGE_VOICE_KEYPHRASES","android.permission.MANAGE_WIFI_INTERFACES","android.permission.MANAGE_WIFI_NETWORK_SELECTION","android.permission.MASTER_CLEAR","android.permission.MEDIA_CONTENT_CONTROL","android.permission.MODIFY_AUDIO_ROUTING","android.permission.MODIFY_DAY_NIGHT_MODE","android.permission.MODIFY_DEFAULT_AUDIO_EFFECTS","android.permission.MODIFY_NETWORK_ACCOUNTING","android.permission.MODIFY_PHONE_STATE","android.permission.NOTIFY_PENDING_SYSTEM_UPDATE","android.permission.OBSERVE_GRANT_REVOKE_PERMISSIONS","android.permission.OBSERVE_ROLE_HOLDERS","android.permission.OVERRIDE_WIFI_CONFIG","android.permission.PACKAGE_USAGE_STATS","android.permission.PREPARE_FACTORY_RESET","android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE","android.permission.PROVIDE_REMOTE_CREDENTIALS","android.permission.PROVIDE_RESOLVER_RANKER_SERVICE","android.permission.PROVIDE_TRUST_AGENT","android.permission.READ_COMPAT_CHANGE_CONFIG","android.permission.READ_DREAM_STATE","android.permission.READ_DROPBOX_DATA","android.permission.READ_INTRUSION_DETECTION_STATE","android.permission.READ_LOGS","android.permission.READ_NETWORK_USAGE_HISTORY","android.permission.READ_OEM_UNLOCK_STATE","android.permission.READ_PRIVILEGED_PHONE_STATE","android.permission.READ_SYSTEM_GRAMMATICAL_GENDER","android.permission.READ_WIFI_CREDENTIAL","android.permission.REAL_GET_TASKS","android.permission.REBOOT","android.permission.RECEIVE_DATA_ACTIVITY_CHANGE","android.permission.RECOVERY","android.permission.RECOVER_KEYSTORE","android.permission.REMOTE_DISPLAY_PROVIDER","android.permission.RENOUNCE_PERMISSIONS","android.permission.REQUEST_COMPANION_PROFILE_COMPUTER","android.permission.REQUEST_COMPANION_SELF_MANAGED","android.permission.RESET_PASSWORD","android.permission.SATELLITE_COMMUNICATION","android.permission.SCHEDULE_PRIORITIZED_ALARM","android.permission.SCORE_NETWORKS","android.permission.SEND_SAFETY_CENTER_UPDATE","android.permission.SEND_SMS_NO_CONFIRMATION","android.permission.SETUP_FSVERITY","android.permission.SET_BIOMETRIC_DIALOG_ADVANCED","android.permission.SET_LOW_POWER_STANDBY_PORTS","android.permission.SET_TIME","android.permission.SET_TIME_ZONE","android.permission.START_ACTIVITIES_FROM_BACKGROUND","android.permission.START_TASKS_FROM_RECENTS","android.permission.STATUS_BAR","android.permission.SUBSCRIBE_TO_KEYGUARD_LOCKED_STATE","android.permission.SUBSTITUTE_NOTIFICATION_APP_NAME","android.permission.SUBSTITUTE_SHARE_TARGET_APP_NAME_AND_ICON","android.permission.TETHER_PRIVILEGED","android.permission.THREAD_NETWORK_PRIVILEGED","android.permission.TURN_SCREEN_ON","android.permission.UPDATE_APP_OPS_STATS","android.permission.UPDATE_DEVICE_STATS","android.permission.UPDATE_FONTS","android.permission.USER_ACTIVITY","android.permission.UWB_PRIVILEGED","android.permission.WRITE_EMBEDDED_SUBSCRIPTIONS","android.permission.WRITE_GSERVICES","android.permission.WRITE_SECURE_SETTINGS","android.permission.WRITE_VERIFICATION_STATE_E2EE_CONTACT_KEYS"},{"android.permission.ALLOCATE_AGGRESSIVE","android.permission.BACKUP","android.permission.BATTERY_STATS","android.permission.CHANGE_COMPONENT_ENABLED_STATE","android.permission.CHANGE_DEVICE_IDLE_TEMP_WHITELIST","android.permission.CHANGE_OVERLAY_PACKAGES","android.permission.CLEAR_APP_CACHE","android.permission.CONNECTIVITY_INTERNAL","android.permission.DELETE_PACKAGES","android.permission.DUMP","android.permission.FORCE_STOP_PACKAGES","android.permission.GET_ACCOUNTS_PRIVILEGED","android.permission.GET_APP_OPS_STATS","android.permission.INSTALL_PACKAGES","android.permission.INTERACT_ACROSS_USERS","android.permission.LOADER_USAGE_STATS","android.permission.MANAGE_CLOUDSEARCH","android.permission.MANAGE_ROLLBACKS","android.permission.MANAGE_USERS","android.permission.OVERRIDE_COMPAT_CHANGE_CONFIG_ON_RELEASE_BUILD","android.permission.PACKAGE_USAGE_STATS","android.permission.PACKAGE_VERIFICATION_AGENT","android.permission.READ_PRIVILEGED_PHONE_STATE","android.permission.READ_RUNTIME_PROFILES","android.permission.READ_SYSTEM_GRAMMATICAL_GENDER","android.permission.REAL_GET_TASKS","android.permission.REBOOT","android.permission.SEND_DEVICE_CUSTOMIZATION_READY","android.permission.SEND_SAFETY_CENTER_UPDATE","android.permission.SEND_SMS_NO_CONFIRMATION","android.permission.SET_PREFERRED_APPLICATIONS","android.permission.START_ACTIVITIES_FROM_BACKGROUND","android.permission.STATUS_BAR","android.permission.SUBSTITUTE_NOTIFICATION_APP_NAME","android.permission.UPDATE_DEVICE_STATS","android.permission.WRITE_SECURE_SETTINGS"}};PackageInfo[] infos=new PackageInfo[names.length];
   for(int i=0;i<names.length;i++){
    PackageInfo p=pm.getPackageInfo(names[i],PackageManager.GET_PERMISSIONS|PackageManager.GET_SIGNING_CERTIFICATES);
    infos[i]=p;
    String cert=digest(p.signingInfo.getApkContentsSigners()[0].toByteArray());String expected=i==0?"5f2391277b1dbd489000467e4c2fa6af802430080457dce2f618992e9dfb5402":"7ce83c1b71f3d572fed04c8d40c5cb10ff75e6d87d9df6fbd53f0468c2905053";
    if(!expected.equals(cert))throw new Exception("Certificate mismatch: "+names[i]);uids[i]=p.applicationInfo.uid;if(uids[i]<10000||uids[i]>=100000)throw new Exception("UID outside user 0");
   }
   Object binder=Class.forName("android.os.ServiceManager").getMethod("getService",String.class).invoke(null,"package");
   Object pms=field(binder,"this$0"),pmLock=field(pms,"mLock"),settings=field(pms,"mSettings"),packages=field(settings,"mPackages");
   String gmsSeInfo;
   synchronized(pmLock){
    Object ps=packages.getClass().getMethod("get",Object.class).invoke(packages,names[0]);
    Object parsed=ps.getClass().getMethod("getAndroidPackage").invoke(ps);
    parsed.getClass().getMethod("setSystem",boolean.class).invoke(parsed,true);
    Field bf=parsed.getClass().getDeclaredField("mBaseAppInfoFlags");bf.setAccessible(true);bf.setInt(parsed,bf.getInt(parsed)|ApplicationInfo.FLAG_SYSTEM);
    int flags=(Integer)ps.getClass().getMethod("getFlags").invoke(ps);
    if((flags&ApplicationInfo.FLAG_SYSTEM)!=0)throw new Exception("Unexpected persistent SYSTEM flag");
    parsed.getClass().getMethod("setPrivileged",boolean.class).invoke(parsed,true);
    Field pf=parsed.getClass().getDeclaredField("mBaseAppInfoPrivateFlags");pf.setAccessible(true);pf.setInt(parsed,pf.getInt(parsed)|8);
    int privateFlags=(Integer)ps.getClass().getMethod("getPrivateFlags").invoke(ps);
    if((privateFlags&8)!=0)throw new Exception("Unexpected persistent PRIVILEGED flag");
    Class<?> pc=Class.forName("com.android.server.pm.pkg.PackageState",false,cl),ap=Class.forName("com.android.server.pm.pkg.AndroidPackage",false,cl);
    String seinfo=(String)Class.forName("com.android.server.pm.SELinuxMMAC",false,cl).getMethod("getSeInfo",pc,ap,boolean.class,int.class).invoke(null,ps,parsed,true,infos[0].applicationInfo.targetSdkVersion);
    gmsSeInfo=seinfo;
    Object transientState=ps.getClass().getMethod("getTransientState").invoke(ps);
    transientState.getClass().getMethod("setSeInfo",String.class).invoke(transientState,seinfo);
    ps.getClass().getMethod("onChanged").invoke(ps);
    pms.getClass().getMethod("invalidatePackageInfoCache").invoke(null);
   }
   Object activity=Class.forName("android.app.ActivityManager").getMethod("getService").invoke(null);
   Method forceStop=activity.getClass().getMethod("forceStopPackage",String.class,int.class);
   forceStop.invoke(activity,names[0],0);forceStop.invoke(activity,names[1],0);
   boolean unlocked=context.getSystemService(android.os.UserManager.class).isUserUnlocked();
   for(String prefix:(unlocked?new String[]{"/data/user_de/0/","/data/user/0/"}:new String[]{"/data/user_de/0/"})) {
    android.system.StructStat st=android.system.Os.stat(prefix+names[0]);
    if(st.st_uid!=uids[0])throw new Exception("GMS data owner mismatch");
   }
   Class<?> argsClass=Class.forName("android.os.CreateAppDataArgs");Object create=argsClass.getConstructor().newInstance();
   argsClass.getField("packageName").set(create,names[0]);argsClass.getField("userId").setInt(create,0);
   argsClass.getField("flags").setInt(create,unlocked?3:1);argsClass.getField("appId").setInt(create,uids[0]);
   argsClass.getField("seInfo").set(create,gmsSeInfo);argsClass.getField("targetSdkVersion").setInt(create,infos[0].applicationInfo.targetSdkVersion);
   Object installer=field(pms,"mInstaller");Object createResult=installer.getClass().getMethod("createAppData",argsClass).invoke(installer,create);
   if(createResult.getClass().getField("exceptionCode").getInt(createResult)!=0)throw new Exception("GMS label preparation failed: "+createResult.getClass().getField("exceptionMessage").get(createResult));
   Class<?> ac=Class.forName("com.android.server.permission.access.AccessState",false,cl),mc=Class.forName("com.android.server.permission.access.MutableAccessState",false,cl),gs=Class.forName("com.android.server.permission.access.GetStateScope",false,cl),ms=Class.forName("com.android.server.permission.access.MutateStateScope",false,cl);
   Constructor<?> gsc=gs.getConstructor(ac),msc=ms.getConstructor(ac,mc);
   Method get=policy.getClass().getMethod("getPermissionFlags",gs,int.class,int.class,String.class),set=policy.getClass().getMethod("setPermissionFlags",ms,int.class,int.class,String.class,int.class),write=persistence.getClass().getMethod("write",ac),notify=allPolicy.getClass().getMethod("onStateMutated",gs);
   Field stateField=service.getClass().getDeclaredField("state");stateField.setAccessible(true);StringBuilder result=new StringBuilder();
   synchronized(lock){
    Object old=stateField.get(service),next=old.getClass().getMethod("toMutable").invoke(old),scope=msc.newInstance(old,next),readScope=gsc.newInstance(old);
    for(int i=0;i<names.length;i++)for(String perm:perms[i]){if(!Arrays.asList(infos[i].requestedPermissions).contains(perm)){result.append("SKIP undeclared "+names[i]+" "+perm+"\n");continue;}PermissionInfo info;try{info=pm.getPermissionInfo(perm,0);}catch(PackageManager.NameNotFoundException missing){result.append("SKIP unavailable "+perm+"\n");continue;}if(!"android".equals(info.packageName))throw new Exception("Permission owner mismatch");int before=(Integer)get.invoke(policy,readScope,uids[i],0,perm);int after=(before&~2)|4;set.invoke(policy,scope,uids[i],0,perm,after);result.append(names[i]+" uid="+uids[i]+" "+perm+" before="+before+" after="+after+"\n");}
    write.invoke(persistence,next);stateField.set(service,next);notify.invoke(allPolicy,gsc.newInstance(next));
   }
   for(int i=0;i<names.length;i++)for(String perm:perms[i])if(Arrays.asList(infos[i].requestedPermissions).contains(perm))try{pm.getPermissionInfo(perm,0);result.append("check "+names[i]+" "+perm+"="+pm.checkPermission(perm,names[i])+"\n");}catch(PackageManager.NameNotFoundException missing){}
   return "GRANT_RESULT\nSYSTEM_GMS="+((pm.getApplicationInfo(names[0],0).flags&ApplicationInfo.FLAG_SYSTEM)!=0)+"\n"+result;
  }catch(Throwable t){java.io.StringWriter s=new java.io.StringWriter();t.printStackTrace(new java.io.PrintWriter(s));return "ERROR "+s;}
 }
}
