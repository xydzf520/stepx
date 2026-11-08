package stepx;

import android.content.*;
import android.content.pm.*;
import android.os.*;
import android.util.AtomicFile;
import java.io.*;
import java.lang.reflect.*;
import java.security.MessageDigest;
import java.util.*;
import org.json.JSONObject;

/** Firmware-scoped Google login registration; no account/token access or attestation changes. */
public final class GoogleLoginRuntime {
    private static final String GMS = "com.google.android.gms";
    private static final String PLAY = "com.android.vending";
    private static final String GSF = "com.google.android.gsf";
    private static final String[] PACKAGES = {GMS,PLAY,GSF};
    private static final String PLAY_SIGNER = "7ce83c1b71f3d572fed04c8d40c5cb10ff75e6d87d9df6fbd53f0468c2905053";
    private static final String SIGNER = "5f2391277b1dbd489000467e4c2fa6af802430080457dce2f618992e9dfb5402";
    private static final File RECEIPT = new File("/data/system/stepx-google-login-runtime.json");
    // Read-only coverage of the existing GmsProbe allowlist; host tests lock both lists.
    private static final String[][] REVIEWED_PERMISSIONS = {
        {
            "android.permission.ACCESS_BROADCAST_RESPONSE_STATS",
            "android.permission.ACCESS_CACHE_FILESYSTEM",
            "android.permission.ACCESS_CONTEXT_HUB",
            "android.permission.ACCESS_FPS_COUNTER",
            "android.permission.ACCESS_NETWORK_CONDITIONS",
            "android.permission.ACCESS_VIBRATOR_STATE",
            "android.permission.ACTIVITY_EMBEDDING",
            "android.permission.ALLOCATE_AGGRESSIVE",
            "android.permission.BACKUP",
            "android.permission.BLUETOOTH_PRIVILEGED",
            "android.permission.BROADCAST_CLOSE_SYSTEM_DIALOGS",
            "android.permission.CALL_AUDIO_INTERCEPTION",
            "android.permission.CALL_PRIVILEGED",
            "android.permission.CAPTURE_AUDIO_HOTWORD",
            "android.permission.CAPTURE_AUDIO_OUTPUT",
            "android.permission.CHANGE_COMPONENT_ENABLED_STATE",
            "android.permission.CHANGE_DEVICE_IDLE_TEMP_WHITELIST",
            "android.permission.CHECK_REMOTE_LOCKSCREEN",
            "android.permission.COMPANION_APPROVE_WIFI_CONNECTIONS",
            "android.permission.CONFIGURE_FACTORY_RESET_PROTECTION",
            "android.permission.CONNECTIVITY_USE_RESTRICTED_NETWORKS",
            "android.permission.CONTROL_DISPLAY_SATURATION",
            "android.permission.CONTROL_INCALL_EXPERIENCE",
            "android.permission.CONTROL_KEYGUARD_SECURE_NOTIFICATIONS",
            "android.permission.DISPATCH_PROVISIONING_MESSAGE",
            "android.permission.DOMAIN_VERIFICATION_AGENT",
            "android.permission.DUMP",
            "android.permission.EMERGENCY_INSTALL_PACKAGES",
            "android.permission.FORCE_STOP_PACKAGES",
            "android.permission.GET_APP_OPS_STATS",
            "android.permission.GET_ROLE_HOLDERS",
            "android.permission.INSTALL_LOCATION_TIME_ZONE_PROVIDER_SERVICE",
            "android.permission.INTENT_FILTER_VERIFICATION_AGENT",
            "android.permission.INTERACT_ACROSS_USERS",
            "android.permission.INVOKE_CARRIER_SETUP",
            "android.permission.LOCAL_MAC_ADDRESS",
            "android.permission.LOCATION_BYPASS",
            "android.permission.LOCATION_HARDWARE",
            "android.permission.LOCK_DEVICE",
            "android.permission.LOG_COMPAT_CHANGE",
            "android.permission.MANAGE_ADVANCED_PROTECTION_MODE",
            "android.permission.MANAGE_FACTORY_RESET_PROTECTION",
            "android.permission.MANAGE_GAME_ACTIVITY",
            "android.permission.MANAGE_GAME_MODE",
            "android.permission.MANAGE_INTRUSION_DETECTION_STATE",
            "android.permission.MANAGE_ROLLBACKS",
            "android.permission.MANAGE_SECURE_LOCK_DEVICE",
            "android.permission.MANAGE_SOUND_TRIGGER",
            "android.permission.MANAGE_SUBSCRIPTION_PLANS",
            "android.permission.MANAGE_TIME_AND_ZONE_DETECTION",
            "android.permission.MANAGE_USB",
            "android.permission.MANAGE_USERS",
            "android.permission.MANAGE_VOICE_KEYPHRASES",
            "android.permission.MANAGE_WIFI_INTERFACES",
            "android.permission.MANAGE_WIFI_NETWORK_SELECTION",
            "android.permission.MASTER_CLEAR",
            "android.permission.MEDIA_CONTENT_CONTROL",
            "android.permission.MODIFY_AUDIO_ROUTING",
            "android.permission.MODIFY_DAY_NIGHT_MODE",
            "android.permission.MODIFY_DEFAULT_AUDIO_EFFECTS",
            "android.permission.MODIFY_NETWORK_ACCOUNTING",
            "android.permission.MODIFY_PHONE_STATE",
            "android.permission.NOTIFY_PENDING_SYSTEM_UPDATE",
            "android.permission.OBSERVE_GRANT_REVOKE_PERMISSIONS",
            "android.permission.OBSERVE_ROLE_HOLDERS",
            "android.permission.OVERRIDE_WIFI_CONFIG",
            "android.permission.PACKAGE_USAGE_STATS",
            "android.permission.PREPARE_FACTORY_RESET",
            "android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE",
            "android.permission.PROVIDE_REMOTE_CREDENTIALS",
            "android.permission.PROVIDE_RESOLVER_RANKER_SERVICE",
            "android.permission.PROVIDE_TRUST_AGENT",
            "android.permission.READ_COMPAT_CHANGE_CONFIG",
            "android.permission.READ_DREAM_STATE",
            "android.permission.READ_DROPBOX_DATA",
            "android.permission.READ_INTRUSION_DETECTION_STATE",
            "android.permission.READ_LOGS",
            "android.permission.READ_NETWORK_USAGE_HISTORY",
            "android.permission.READ_OEM_UNLOCK_STATE",
            "android.permission.READ_PRIVILEGED_PHONE_STATE",
            "android.permission.READ_SYSTEM_GRAMMATICAL_GENDER",
            "android.permission.READ_WIFI_CREDENTIAL",
            "android.permission.REAL_GET_TASKS",
            "android.permission.REBOOT",
            "android.permission.RECEIVE_DATA_ACTIVITY_CHANGE",
            "android.permission.RECOVERY",
            "android.permission.RECOVER_KEYSTORE",
            "android.permission.REMOTE_DISPLAY_PROVIDER",
            "android.permission.RENOUNCE_PERMISSIONS",
            "android.permission.REQUEST_COMPANION_PROFILE_COMPUTER",
            "android.permission.REQUEST_COMPANION_SELF_MANAGED",
            "android.permission.RESET_PASSWORD",
            "android.permission.SATELLITE_COMMUNICATION",
            "android.permission.SCHEDULE_PRIORITIZED_ALARM",
            "android.permission.SCORE_NETWORKS",
            "android.permission.SEND_SAFETY_CENTER_UPDATE",
            "android.permission.SEND_SMS_NO_CONFIRMATION",
            "android.permission.SETUP_FSVERITY",
            "android.permission.SET_BIOMETRIC_DIALOG_ADVANCED",
            "android.permission.SET_LOW_POWER_STANDBY_PORTS",
            "android.permission.SET_TIME",
            "android.permission.SET_TIME_ZONE",
            "android.permission.START_ACTIVITIES_FROM_BACKGROUND",
            "android.permission.START_TASKS_FROM_RECENTS",
            "android.permission.STATUS_BAR",
            "android.permission.SUBSCRIBE_TO_KEYGUARD_LOCKED_STATE",
            "android.permission.SUBSTITUTE_NOTIFICATION_APP_NAME",
            "android.permission.SUBSTITUTE_SHARE_TARGET_APP_NAME_AND_ICON",
            "android.permission.TETHER_PRIVILEGED",
            "android.permission.THREAD_NETWORK_PRIVILEGED",
            "android.permission.TURN_SCREEN_ON",
            "android.permission.UPDATE_APP_OPS_STATS",
            "android.permission.UPDATE_DEVICE_STATS",
            "android.permission.UPDATE_FONTS",
            "android.permission.USER_ACTIVITY",
            "android.permission.UWB_PRIVILEGED",
            "android.permission.WRITE_EMBEDDED_SUBSCRIPTIONS",
            "android.permission.WRITE_GSERVICES",
            "android.permission.WRITE_SECURE_SETTINGS",
            "android.permission.WRITE_VERIFICATION_STATE_E2EE_CONTACT_KEYS"
        },
        {
            "android.permission.ALLOCATE_AGGRESSIVE",
            "android.permission.BACKUP",
            "android.permission.BATTERY_STATS",
            "android.permission.CHANGE_COMPONENT_ENABLED_STATE",
            "android.permission.CHANGE_DEVICE_IDLE_TEMP_WHITELIST",
            "android.permission.CHANGE_OVERLAY_PACKAGES",
            "android.permission.CLEAR_APP_CACHE",
            "android.permission.CONNECTIVITY_INTERNAL",
            "android.permission.DELETE_PACKAGES",
            "android.permission.DUMP",
            "android.permission.FORCE_STOP_PACKAGES",
            "android.permission.GET_ACCOUNTS_PRIVILEGED",
            "android.permission.GET_APP_OPS_STATS",
            "android.permission.INSTALL_PACKAGES",
            "android.permission.INTERACT_ACROSS_USERS",
            "android.permission.LOADER_USAGE_STATS",
            "android.permission.MANAGE_CLOUDSEARCH",
            "android.permission.MANAGE_ROLLBACKS",
            "android.permission.MANAGE_USERS",
            "android.permission.OVERRIDE_COMPAT_CHANGE_CONFIG_ON_RELEASE_BUILD",
            "android.permission.PACKAGE_USAGE_STATS",
            "android.permission.PACKAGE_VERIFICATION_AGENT",
            "android.permission.READ_PRIVILEGED_PHONE_STATE",
            "android.permission.READ_RUNTIME_PROFILES",
            "android.permission.READ_SYSTEM_GRAMMATICAL_GENDER",
            "android.permission.REAL_GET_TASKS",
            "android.permission.REBOOT",
            "android.permission.SEND_DEVICE_CUSTOMIZATION_READY",
            "android.permission.SEND_SAFETY_CENTER_UPDATE",
            "android.permission.SEND_SMS_NO_CONFIRMATION",
            "android.permission.SET_PREFERRED_APPLICATIONS",
            "android.permission.START_ACTIVITIES_FROM_BACKGROUND",
            "android.permission.STATUS_BAR",
            "android.permission.SUBSTITUTE_NOTIFICATION_APP_NAME",
            "android.permission.UPDATE_DEVICE_STATS",
            "android.permission.WRITE_SECURE_SETTINGS"
        }
    };
    private static final long[] RETRY_DELAYS = {15000,45000,120000};
    private static final String RECHECK_ACTION="com.stepx.companion.action.RECHECK_GOOGLE_RUNTIME";
    private static final String RUNTIME_REVISION="google-login-v2-recheck-1";
    private static long lastRecheckAt=-30000;
    private static long generation;
    private static boolean updatePending,grantAuthorized;
    private static Handler handler;
    private static BroadcastReceiver receiver;
    private static Context context;
    private static Runnable pending;
    private static volatile boolean stopped;
    private static final String REGISTRY="stepx.google.login.runtime.v1";
    private static final Set<Object> owned=Collections.newSetFromMap(new IdentityHashMap<Object,Boolean>());
    private static Object domainService, originalDomainProxy, installedDomainProxy;

    private static Object field(Object o, String name) throws Exception {
        for (Class<?> c=o.getClass();c!=null;c=c.getSuperclass()) {
            try { Field f=c.getDeclaredField(name);f.setAccessible(true);return f.get(o); }
            catch(NoSuchFieldException ignored) {}
        }
        throw new NoSuchFieldException(name);
    }
    private static String digest(byte[] bytes) throws Exception {
        StringBuilder s=new StringBuilder();
        for(byte b:MessageDigest.getInstance("SHA-256").digest(bytes))s.append(String.format("%02x",b&255));
        return s.toString();
    }
    private static final class Disabled extends Exception {}
    private static final class Changed extends Exception {}
    private static final class Unmanaged extends Exception {}
    private static final class CallBusy extends Exception {}
    private static final class IdentityFailure extends SecurityException {
        final String code,packageName,signer;
        IdentityFailure(String code,String packageName){this(code,packageName,"");}
        IdentityFailure(String code,String packageName,String signer){super(code);this.code=code;this.packageName=packageName;this.signer=signer;}
    }
    private static void firmware() {
        if(android.os.Process.myUid()!=1000 ||
           (!"StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys".equals(Build.FINGERPRINT)&&
            !"StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/12:user/release-keys".equals(Build.FINGERPRINT)))
            throw new IdentityFailure("unsupported_context","");
    }
    private static boolean trustedSigner(int index,SigningInfo signing,Signature[] current) throws Exception {
        if(current==null||current.length!=1)return false;
        String actual=digest(current[0].toByteArray());
        if((index==0?SIGNER:PLAY_SIGNER).equals(actual))return true;
        // Accept GSF key rotation only when the platform proves the previously
        // reviewed Play anchor and reviewed GMS current key in its signing lineage;
        // knowing either key alone is not sufficient to authorize a rotation.
        if(index!=2||!SIGNER.equals(actual)||!signing.hasPastSigningCertificates())return false;
        Signature[] history=signing.getSigningCertificateHistory();
        if(history==null||history.length<2||!actual.equals(digest(history[history.length-1].toByteArray())))return false;
        for(int i=0;i<history.length-1;i++)if(PLAY_SIGNER.equals(digest(history[i].toByteArray()))) {
            byte[] anchor=new byte[32];
            for(int n=0;n<anchor.length;n++)anchor[n]=(byte)Integer.parseInt(PLAY_SIGNER.substring(n*2,n*2+2),16);
            return context.getPackageManager().hasSigningCertificate(GSF,anchor,PackageManager.CERT_INPUT_SHA256);
        }
        return false;
    }
    private static PackageInfo[] trustedPackages() throws Exception {
        firmware();
        PackageInfo[] result=new PackageInfo[PACKAGES.length];
        PackageManager pm=context.getPackageManager();
        for(int i=0;i<PACKAGES.length;i++) {
            PackageInfo p=pm.getPackageInfo(PACKAGES[i],PackageManager.GET_PERMISSIONS|PackageManager.GET_SIGNING_CERTIFICATES);
            Signature[] signers=p.signingInfo==null?null:p.signingInfo.getApkContentsSigners();
            if(!trustedSigner(i,p.signingInfo,signers))throw new IdentityFailure("signer_changed",PACKAGES[i],
                signers!=null&&signers.length==1?digest(signers[0].toByteArray()):"");
            if(p.applicationInfo==null||p.applicationInfo.uid<10000||p.applicationInfo.uid>=100000)
                throw new IdentityFailure("uid_outside_user_zero",PACKAGES[i]);
            if(!p.applicationInfo.enabled||(p.applicationInfo.flags&ApplicationInfo.FLAG_SUSPENDED)!=0)throw new Disabled();
            if(p.getLongVersionCode()<=0||p.lastUpdateTime<=0)throw new Changed();
            result[i]=p;
        }
        return result;
    }
    private static String identity(PackageInfo[] packages) {
        StringBuilder key=new StringBuilder();
        for(PackageInfo p:packages)key.append(p.applicationInfo.uid).append(':').append(p.getLongVersionCode()).append(':').append(p.lastUpdateTime).append(';');
        return key.toString();
    }
    private static void versions(JSONObject r,PackageInfo[] packages) throws Exception {
        String[] names={"gms","play","gsf"};
        java.text.SimpleDateFormat local=new java.text.SimpleDateFormat("yyyy-MM-dd HH:mm:ss",Locale.US);
        java.text.SimpleDateFormat offset=new java.text.SimpleDateFormat("Z",Locale.US);
        for(int i=0;i<names.length;i++)r.put(names[i]+"_version",packages[i].getLongVersionCode())
            .put(names[i]+"_updated_at",packages[i].lastUpdateTime).put(names[i]+"_uid",packages[i].applicationInfo.uid)
            .put(names[i]+"_updated_local",local.format(new Date(packages[i].lastUpdateTime)))
            .put(names[i]+"_updated_offset",offset.format(new Date(packages[i].lastUpdateTime)));
        r.put("signatures_verified",true);
    }
    /** No grant is performed here: read every currently applicable pinned permission. */
    private static boolean permissions(JSONObject report,PackageInfo[] packages) throws Exception {
        PackageManager pm=context.getPackageManager();StringBuilder result=new StringBuilder();int checks=0;boolean all=true;
        for(int i=0;i<REVIEWED_PERMISSIONS.length;i++) {
            if(packages[i].requestedPermissions==null)throw new Changed();
            Set<String> requested=new HashSet<>(Arrays.asList(packages[i].requestedPermissions));
            for(String permission:REVIEWED_PERMISSIONS[i]) {
                if(!requested.contains(permission)){result.append("SKIP undeclared ").append(PACKAGES[i]).append(' ').append(permission).append('\n');continue;}
                PermissionInfo info;
                try{info=pm.getPermissionInfo(permission,0);}
                catch(PackageManager.NameNotFoundException missing){result.append("SKIP unavailable ").append(permission).append('\n');continue;}
                if(!"android".equals(info.packageName))throw new SecurityException("Permission owner changed");
                int granted=pm.checkPermission(permission,PACKAGES[i]);checks++;all &= granted==PackageManager.PERMISSION_GRANTED;
                result.append("check ").append(PACKAGES[i]).append(' ').append(permission).append('=').append(granted).append('\n');
            }
        }
        // The exact required provider permission prevents an empty set being called ready.
        all &= checks>0&&pm.checkPermission("android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE",GMS)==PackageManager.PERMISSION_GRANTED;
        report.put("permission_result",result.toString()).put("permission_checks",checks).put("permissions_verified",all);
        return all;
    }
    public static synchronized JSONObject install() throws Exception {
        try {
            firmware();
            Class<?> at=Class.forName("android.app.ActivityThread");
            context=(Context)at.getMethod("getSystemContext").invoke(at.getMethod("currentActivityThread").invoke(null));
            trustedPackages();
        } catch(IdentityFailure failure) {
            // An upgrade rejected before handler creation must not leave the
            // previous classloader's observer reporting an old "ready" receipt.
            if(android.os.Process.myUid()!=1000)throw failure;
            stopInstalled();
            JSONObject report=new JSONObject().put("status","blocked").put("error",failure.getClass().getSimpleName())
                .put("error_code",failure.code).put("identity_package",failure.packageName)
                .put("observed_signer_sha256",failure.signer)
                .put("update_monitor",false).put("recovery_pending",false).put("retry_after_ms",0)
                .put("signatures_verified",false).put("permissions_verified",false);
            writeReceipt(report);return report;
        } catch(PackageManager.NameNotFoundException absent) {
            return new JSONObject().put("status","not_installed");
        } catch(Disabled disabled) { return new JSONObject().put("status","disabled"); }
        boolean authorized=context.getPackageManager().checkPermission("android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE",GMS)==PackageManager.PERMISSION_GRANTED;
        if(handler==null) {
            stopInstalled();stopped=false;generation++;grantAuthorized=authorized;lastRecheckAt=-30000;
            HandlerThread t=new HandlerThread("StepX.GoogleLogin",android.os.Process.THREAD_PRIORITY_BACKGROUND);t.start();handler=new Handler(t.getLooper());
            receiver=new BroadcastReceiver(){public void onReceive(Context c,Intent i){
                if(i!=null&&RECHECK_ACTION.equals(i.getAction())){recheck(i);return;}
                if(i!=null&&Intent.ACTION_USER_UNLOCKED.equals(i.getAction())){schedule(false,1500);return;}
                if(i==null||i.getData()==null)return;
                String pkg=i.getData().getSchemeSpecificPart();
                if(!GMS.equals(pkg)&&!PLAY.equals(pkg)&&!GSF.equals(pkg))return;
                if(Intent.ACTION_PACKAGE_FULLY_REMOVED.equals(i.getAction()))stop();
                else if(Intent.ACTION_PACKAGE_REPLACED.equals(i.getAction()))schedule(true,8000);
            }};
            IntentFilter filter=new IntentFilter(Intent.ACTION_PACKAGE_REPLACED);filter.addAction(Intent.ACTION_PACKAGE_FULLY_REMOVED);filter.addDataScheme("package");
            try {
                context.registerReceiver(receiver,filter,null,handler,Context.RECEIVER_NOT_EXPORTED);
                // USER_UNLOCKED has no package URI and needs its own filter. It
                // never unlocks/wakes the device; it rechecks after Android is ready.
                context.registerReceiver(receiver,new IntentFilter(Intent.ACTION_USER_UNLOCKED),null,handler,Context.RECEIVER_NOT_EXPORTED);
                // Fixed, permission-protected maintenance only. DUMP is held by
                // shell/system and some trusted privileged apps; this receiver
                // cannot authorize grants or execute caller-supplied work.
                context.registerReceiver(receiver,new IntentFilter(RECHECK_ACTION),"android.permission.DUMP",handler,Context.RECEIVER_EXPORTED);
            }
            catch(Exception error){stop();throw error;}
            System.getProperties().put(REGISTRY,(Runnable)GoogleLoginRuntime::stop);
        }
        // Missing permission must not suppress the observer. It remains read-only
        // until the existing authorization gate has actually been seen granted;
        // an unverified on-disk receipt can never authorize this grant engine.
        grantAuthorized |= authorized;
        // Repeated desktop inspections cannot replace an update repair with a boot-only check.
        if(pending==null)schedule(false,1500);
        return new JSONObject().put("status",grantAuthorized?"scheduled":"needs_google_repair")
            .put("update_monitor",true).put("grant_authorized",grantAuthorized).put("recheck_supported",grantAuthorized)
            .put("runtime_revision",RUNTIME_REVISION);
    }
    private static synchronized void recheck(Intent intent) {
        if(stopped||handler==null||!grantAuthorized||pending!=null||!"android".equals(intent.getPackage())||
           intent.getData()!=null||(intent.getExtras()!=null&&!intent.getExtras().isEmpty()))return;
        long now=SystemClock.elapsedRealtime();
        if(now-lastRecheckAt<30000)return;
        lastRecheckAt=now;schedule(false,1500);
    }
    private static synchronized void schedule(boolean update,long delay) {
        if(stopped||handler==null)return;
        updatePending |= update;generation++;
        if(pending!=null)handler.removeCallbacks(pending);
        enqueue(updatePending,1,generation,delay);
    }
    private static void enqueue(boolean update,int attempt,long epoch,long delay) {
        pending=()->repair(update,attempt,epoch);handler.postDelayed(pending,delay);
    }
    private static synchronized void repair(boolean update,int attempt,long epoch) {
        if(stopped||epoch!=generation)return;
        pending=null;
        JSONObject r=new JSONObject();long token=Binder.clearCallingIdentity();boolean hardStop=false;
        try {
            r.put("attempt",attempt).put("reason",update?"package_updated":"boot").put("recovery_pending",false).put("retry_after_ms",0);
            PackageInfo[] packages=trustedPackages();String expected=identity(packages);versions(r,packages);
            // Validate the platform permission owners and user component state before
            // calling the existing grant engine. A denied grant is repairable; a
            // different permission owner or a user-disabled provider is not.
            boolean permissionsReady=permissions(r,packages);
            providerDeclaration();
            grantAuthorized |= context.getPackageManager().checkPermission("android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE",GMS)==PackageManager.PERMISSION_GRANTED;
            r.put("grant_authorized",grantAuthorized);
            if(!grantAuthorized)throw new Unmanaged();
            if(!permissionsReady||(packages[0].applicationInfo.flags&ApplicationInfo.FLAG_SYSTEM)==0) {
                // Recheck on the phone immediately before the grant engine can
                // restart Google processes: a call may begin after a host probe.
                android.telecom.TelecomManager calls=context.getSystemService(android.telecom.TelecomManager.class);
                if(calls==null||calls.isInCall())throw new CallBusy();
                // Reuse the existing pin-scoped engine on this background Android thread.
                // No native reinjection, process-wide bridge restart, or added permission.
                String grant=GmsProbe.run();
                if(stopped||epoch!=generation)return;
                if(!grant.startsWith("GRANT_RESULT\nSYSTEM_GMS=true\n"))throw new IllegalStateException("Google update grant failed");
            }
            if((context.getPackageManager().getPackageInfo(GMS,0).applicationInfo.flags&ApplicationInfo.FLAG_SYSTEM)==0)
                throw new IllegalStateException("Google system state read-back failed");
            if(!permissions(r,packages))throw new IllegalStateException("Google permission read-back failed");
            if(!expected.equals(identity(trustedPackages())))throw new Changed();
            registerProvider(r);
            registerDomainVerifier(r);
            if(stopped||epoch!=generation)return;
            // PackageManager may have replaced an APK while reflection registered its provider.
            if(!expected.equals(identity(trustedPackages()))||!permissions(r,packages))throw new Changed();
            if(!r.optBoolean("registered")||!r.optBoolean("domain_ready"))throw new IllegalStateException("Google provider read-back incomplete");
            r.put("status","ready").put("update_monitor",true);updatePending=false;
        } catch(Throwable error) {
            try {
                while(error instanceof InvocationTargetException&&((InvocationTargetException)error).getCause()!=null)error=error.getCause();
                hardStop=error instanceof SecurityException||error instanceof LinkageError||error instanceof ReflectiveOperationException;
                boolean retry=!hardStop&&!(error instanceof Disabled)&&!(error instanceof Unmanaged)&&attempt<=RETRY_DELAYS.length;
                long delay=retry?RETRY_DELAYS[attempt-1]:0;
                r.put("status",error instanceof Disabled?"disabled":error instanceof Unmanaged?"needs_google_repair":hardStop?"blocked":retry?"retrying":"needs_repair")
                    .put("error",error.getClass().getSimpleName()).put("permissions_verified",false)
                    .put("recovery_pending",retry).put("retry_after_ms",delay).put("update_monitor",!hardStop);
                if(retry&&!stopped&&epoch==generation)enqueue(update,attempt+1,epoch,delay);
                else updatePending=false;
            }catch(Exception ignored){}
        } finally {
            if(!stopped&&epoch==generation)writeReceipt(r);
            Binder.restoreCallingIdentity(token);
            if(hardStop)stop(false);
        }
    }
    private static void writeReceipt(JSONObject r) {
        try {
            r.put("recheck_supported",handler!=null&&!stopped&&grantAuthorized&&!"blocked".equals(r.optString("status")));
            r.put("runtime_revision",RUNTIME_REVISION);
            r.put("schema",2).put("system_server_pid",android.os.Process.myPid()).put("checked_at",System.currentTimeMillis());
            r.put("boot_id",new String(java.nio.file.Files.readAllBytes(new File("/proc/sys/kernel/random/boot_id").toPath()),"UTF-8").trim());
            AtomicFile file=new AtomicFile(RECEIPT);FileOutputStream out=null;
            try {out=file.startWrite();out.write(r.toString().getBytes("UTF-8"));file.finishWrite(out);android.system.Os.chmod(RECEIPT.getPath(),0600);}
            catch(Exception e){if(out!=null)file.failWrite(out);}
        }catch(Exception ignored){}
    }
    private static ServiceInfo providerDeclaration() throws Exception {
        PackageManager pm=context.getPackageManager();
        ComponentName component=new ComponentName(GMS,GMS+".auth.api.credentials.credman.service.GoogleIdService");
        int setting=pm.getComponentEnabledSetting(component);
        if(setting==PackageManager.COMPONENT_ENABLED_STATE_DISABLED ||
           setting==PackageManager.COMPONENT_ENABLED_STATE_DISABLED_USER ||
           setting==PackageManager.COMPONENT_ENABLED_STATE_DISABLED_UNTIL_USED)throw new Disabled();
        ServiceInfo info=pm.getServiceInfo(component,PackageManager.GET_META_DATA|PackageManager.MATCH_DISABLED_COMPONENTS);
        if(!info.enabled)throw new Disabled();
        if(!info.exported||!"android.permission.BIND_CREDENTIAL_PROVIDER_SERVICE".equals(info.permission))
            throw new SecurityException("Google provider declaration changed");
        return info;
    }
    @SuppressWarnings("unchecked")
    private static void registerProvider(JSONObject report) throws Exception {
        PackageManager pm=context.getPackageManager();
        if(pm.checkPermission("android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE",GMS)!=PackageManager.PERMISSION_GRANTED)
            throw new SecurityException("Missing Google provider permission");
        ComponentName component=new ComponentName(GMS,GMS+".auth.api.credentials.credman.service.GoogleIdService");
        ServiceInfo si=providerDeclaration();
        Object binder=Class.forName("android.os.ServiceManager").getMethod("getService",String.class).invoke(null,"credential");
        Object service=field(binder,"this$0"),lock=field(service,"mLock"),cache=field(service,"mSystemServicesCacheList");
        Class<?> factory=Class.forName("android.service.credentials.CredentialProviderInfoFactory");
        Object info=factory.getMethod("create",Context.class,ServiceInfo.class,boolean.class,boolean.class,boolean.class,boolean.class)
            .invoke(null,context,si,true,false,false,false);
        if(!Boolean.TRUE.equals(info.getClass().getMethod("isSystemProvider").invoke(info)))throw new SecurityException("Not a system credential provider");
        synchronized(lock) {
            Object old=cache.getClass().getMethod("get",int.class).invoke(cache,0);
            List<Object> next=new ArrayList<>();if(old instanceof List)next.addAll((List<?>)old);
            // Existing providers are left intact; Android removes stale entries on package update.
            for(Object item:next) {
                Object prior=field(item,"mInfo");
                if(component.equals(prior.getClass().getMethod("getComponentName").invoke(prior))) {
                    if(!Boolean.TRUE.equals(prior.getClass().getMethod("isSystemProvider").invoke(prior)))
                        throw new SecurityException("Unexpected existing credential provider");
                    report.put("provider",component.flattenToString()).put("registered",true);return;
                }
            }
            Class<?> impl=Class.forName("com.android.server.credentials.CredentialManagerServiceImpl");
            Constructor<?> ctor=impl.getDeclaredConstructor(service.getClass(),Object.class,int.class,info.getClass());ctor.setAccessible(true);
            if(stopped)return;
            Object entry=ctor.newInstance(service,lock,0,info);next.add(entry);owned.add(entry);
            cache.getClass().getMethod("put",int.class,Object.class).invoke(cache,0,next);
            Object stored=cache.getClass().getMethod("get",int.class).invoke(cache,0);
            if(!(stored instanceof List)||!((List<?>)stored).contains(entry))throw new IllegalStateException("Provider registration not retained");
            report.put("provider",component.flattenToString()).put("registered",true);
        }
    }
    public static void stopInstalled() {
        Object previous=System.getProperties().remove(REGISTRY);
        if(previous instanceof Runnable)((Runnable)previous).run();
    }
    private static void registerDomainVerifier(JSONObject report) throws Exception {
        Object binder=Class.forName("android.os.ServiceManager").getMethod("getService",String.class).invoke(null,"domain_verification");
        Object service=field(binder,"mService"),lock=field(service,"mLock");
        Class<?> api=Class.forName("com.android.server.pm.verify.domain.proxy.DomainVerificationProxy");
        synchronized(lock) {
            Object proxy=service.getClass().getMethod("getProxy").invoke(service);
            ComponentName current=(ComponentName)api.getMethod("getComponentName").invoke(proxy);
            if(current!=null){report.put("domain_ready",true).put("domain_provider",current.flattenToString());return;}
            if(!proxy.getClass().getName().endsWith(".DomainVerificationProxyUnavailable"))
                throw new SecurityException("Unrecognized domain verifier state");
            PackageManager pm=context.getPackageManager();
            if(pm.checkPermission("android.permission.DOMAIN_VERIFICATION_AGENT",GMS)!=PackageManager.PERMISSION_GRANTED)
                throw new SecurityException("Missing domain verifier permission");
            ComponentName target=new ComponentName(GMS,GMS+".chimera.GmsIntentOperationService$PersistentTrustedReceiver");
            boolean declared=false;
            for(ResolveInfo ri:pm.queryBroadcastReceivers(new Intent("android.intent.action.DOMAINS_NEED_VERIFICATION").setPackage(GMS),0))
                if(ri.activityInfo!=null&&target.equals(new ComponentName(ri.activityInfo.packageName,ri.activityInfo.name)))declared=true;
            if(!declared)throw new SecurityException("Google domain receiver changed");
            Class<?> connection=Class.forName("com.android.server.pm.verify.domain.proxy.DomainVerificationProxyV2$Connection");
            Object next=Class.forName("com.android.server.pm.verify.domain.proxy.DomainVerificationProxyV2")
                .getConstructor(Context.class,connection,ComponentName.class).newInstance(context,field(service,"mConnection"),target);
            if(stopped)return;
            service.getClass().getMethod("setProxy",api).invoke(service,next);
            domainService=service;originalDomainProxy=proxy;installedDomainProxy=next;
            if(service.getClass().getMethod("getProxy").invoke(service)!=next)
                throw new IllegalStateException("Domain verifier registration not retained");
            report.put("domain_ready",true).put("domain_provider",target.flattenToString());
        }
        // Android/Google still perform the normal HTTPS assetlinks verification.
        // No domain is approved here, and user link preferences remain unchanged.
    }
    private static synchronized void stop() { stop(true); }
    private static synchronized void stop(boolean receipt) {
        if(stopped)return;
        stopped=true;generation++;updatePending=false;grantAuthorized=false;
        if(receipt)try{writeReceipt(new JSONObject().put("status","stopped").put("recovery_pending",false).put("retry_after_ms",0).put("update_monitor",false).put("permissions_verified",false));}catch(Exception ignored){}
        if(handler!=null){handler.removeCallbacksAndMessages(null);handler.getLooper().quitSafely();handler=null;}
        if(receiver!=null){try{context.unregisterReceiver(receiver);}catch(Exception ignored){}receiver=null;}
        pending=null;
        try {
            Object binder=Class.forName("android.os.ServiceManager").getMethod("getService",String.class).invoke(null,"credential");
            Object service=field(binder,"this$0"),lock=field(service,"mLock"),cache=field(service,"mSystemServicesCacheList");
            synchronized(lock) {
                Object list=cache.getClass().getMethod("get",int.class).invoke(cache,0);
                if(list instanceof List)((List<?>)list).removeIf(owned::contains);
                owned.clear();
            }
        }catch(Exception ignored){}
        owned.clear();
        try {
            if(domainService!=null)synchronized(field(domainService,"mLock")) {
                if(domainService.getClass().getMethod("getProxy").invoke(domainService)==installedDomainProxy)
                    domainService.getClass().getMethod("setProxy",Class.forName("com.android.server.pm.verify.domain.proxy.DomainVerificationProxy")).invoke(domainService,originalDomainProxy);
            }
        }catch(Exception ignored){}
        domainService=null;originalDomainProxy=null;installedDomainProxy=null;
    }
}
