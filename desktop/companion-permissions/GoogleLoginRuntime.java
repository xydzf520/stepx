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
    private static final String SIGNER = "5f2391277b1dbd489000467e4c2fa6af802430080457dce2f618992e9dfb5402";
    private static final File RECEIPT = new File("/data/system/stepx-google-login-runtime.json");
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
    private static PackageInfo trustedGoogle() throws Exception {
        PackageInfo p=context.getPackageManager().getPackageInfo(GMS,PackageManager.GET_SIGNING_CERTIFICATES);
        if(p.signingInfo==null||p.signingInfo.getApkContentsSigners().length!=1||
           !SIGNER.equals(digest(p.signingInfo.getApkContentsSigners()[0].toByteArray()))||
           p.applicationInfo.uid<10000||p.applicationInfo.uid>=100000)
            throw new SecurityException("Google signing identity changed");
        return p;
    }
    public static synchronized JSONObject install() throws Exception {
        if(!"StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys".equals(Build.FINGERPRINT)&&!"StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/12:user/release-keys".equals(Build.FINGERPRINT))
            throw new SecurityException("Unsupported Google login firmware");
        Class<?> at=Class.forName("android.app.ActivityThread");
        context=(Context)at.getMethod("getSystemContext").invoke(at.getMethod("currentActivityThread").invoke(null));
        try { trustedGoogle(); } catch(PackageManager.NameNotFoundException absent) {
            return new JSONObject().put("status","not_installed");
        }
        // Activate only on an already adapted installation. No new Google grant on ordinary phones.
        if(context.getPackageManager().checkPermission("android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE",GMS)!=PackageManager.PERMISSION_GRANTED)
            return new JSONObject().put("status","needs_google_repair");
        if(handler==null) {
            stopInstalled();stopped=false;
            HandlerThread t=new HandlerThread("StepX.GoogleLogin",android.os.Process.THREAD_PRIORITY_BACKGROUND);t.start();handler=new Handler(t.getLooper());
            receiver=new BroadcastReceiver(){public void onReceive(Context c,Intent i){
                String pkg=i.getData()==null?"":i.getData().getSchemeSpecificPart();
                if(GMS.equals(pkg)||PLAY.equals(pkg))schedule(true,8000);
            }};
            IntentFilter filter=new IntentFilter(Intent.ACTION_PACKAGE_REPLACED);filter.addDataScheme("package");
            context.registerReceiver(receiver,filter,null,handler,Context.RECEIVER_NOT_EXPORTED);
            System.getProperties().put(REGISTRY,(Runnable)GoogleLoginRuntime::stop);
        }
        schedule(false,1500);
        return new JSONObject().put("status","scheduled").put("update_monitor",true);
    }
    private static synchronized void schedule(boolean update,long delay) {
        if(pending!=null)handler.removeCallbacks(pending);
        pending=()->repair(update);handler.postDelayed(pending,delay);
    }
    private static void repair(boolean update) {
        if(stopped)return;
        JSONObject r=new JSONObject();long token=Binder.clearCallingIdentity();
        try {
            PackageInfo p=trustedGoogle();
            if(!p.applicationInfo.enabled)throw new IllegalStateException("Google Play services disabled by user");
            r.put("gms_version",p.getLongVersionCode()).put("play_version",context.getPackageManager().getPackageInfo(PLAY,0).getLongVersionCode());
            if(update) {
                // The existing, signature-pinned permission allowlist is reused on a managed
                // Android thread. Package updates do not start another native injection.
                String grant=GmsProbe.run();int checks=0;
                if(!grant.startsWith("GRANT_RESULT\nSYSTEM_GMS=true\n"))throw new IllegalStateException("Google update grant failed");
                for(String line:grant.split("\n"))if(line.startsWith("check ")) {
                    if(!line.endsWith("=0"))throw new IllegalStateException("Google permission read-back failed");checks++;
                }
                if(checks<120)throw new IllegalStateException("Incomplete Google permission verification");
                r.put("permission_checks",checks);
            }
            registerProvider(r);
            try { registerDomainVerifier(r); }
            catch(Exception domainError){r.put("domain_ready",false).put("domain_error",domainError.getClass().getSimpleName());}
            r.put("status","ready").put("update_monitor",true).put("reason",update?"package_updated":"boot");
        } catch(Throwable error) {
            try {r.put("status","needs_repair").put("error",error.getClass().getSimpleName());}catch(Exception ignored){}
        } finally {
            try {
                r.put("schema",1).put("system_server_pid",android.os.Process.myPid()).put("checked_at",System.currentTimeMillis());
                r.put("boot_id",new String(java.nio.file.Files.readAllBytes(new File("/proc/sys/kernel/random/boot_id").toPath()),"UTF-8").trim());
                AtomicFile file=new AtomicFile(RECEIPT);FileOutputStream out=null;
                try {out=file.startWrite();out.write(r.toString().getBytes("UTF-8"));file.finishWrite(out);android.system.Os.chmod(RECEIPT.getPath(),0600);}
                catch(Exception e){if(out!=null)file.failWrite(out);}
            }catch(Exception ignored){}
            Binder.restoreCallingIdentity(token);
        }
    }
    @SuppressWarnings("unchecked")
    private static void registerProvider(JSONObject report) throws Exception {
        PackageManager pm=context.getPackageManager();
        if(pm.checkPermission("android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE",GMS)!=PackageManager.PERMISSION_GRANTED)
            throw new SecurityException("Missing Google provider permission");
        ComponentName component=new ComponentName(GMS,GMS+".auth.api.credentials.credman.service.GoogleIdService");
        ServiceInfo si=pm.getServiceInfo(component,PackageManager.GET_META_DATA);
        if(!si.enabled||!si.exported||!"android.permission.BIND_CREDENTIAL_PROVIDER_SERVICE".equals(si.permission))
            throw new SecurityException("Google provider declaration changed");
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
                    report.put("provider",component.flattenToString()).put("registered",true);return;
                }
            }
            Class<?> impl=Class.forName("com.android.server.credentials.CredentialManagerServiceImpl");
            Constructor<?> ctor=impl.getDeclaredConstructor(service.getClass(),Object.class,int.class,info.getClass());ctor.setAccessible(true);
            if(stopped)return;
            Object entry=ctor.newInstance(service,lock,0,info);next.add(entry);owned.add(entry);
            cache.getClass().getMethod("put",int.class,Object.class).invoke(cache,0,next);
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
            report.put("domain_ready",true).put("domain_provider",target.flattenToString());
        }
        // Android/Google still perform the normal HTTPS assetlinks verification.
        // No domain is approved here, and user link preferences remain unchanged.
    }
    private static void stop() {
        stopped=true;
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
        try {
            if(domainService!=null)synchronized(field(domainService,"mLock")) {
                if(domainService.getClass().getMethod("getProxy").invoke(domainService)==installedDomainProxy)
                    domainService.getClass().getMethod("setProxy",Class.forName("com.android.server.pm.verify.domain.proxy.DomainVerificationProxy")).invoke(domainService,originalDomainProxy);
            }
        }catch(Exception ignored){}
    }
}
