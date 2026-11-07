package stepx;

import android.content.*;
import android.os.*;
import org.json.JSONObject;

/** Restores an already authorized installation only after its own package replacement. */
final class CompanionUpdateRuntime {
    private static final String PACKAGE="com.stepx.companion.app";
    private static final String REGISTRY="stepx.companion.update.runtime.v1";
    private final Context context;
    private final String signer,baseline;
    private final int uid;
    private final HandlerThread thread;
    private final Handler handler;
    private boolean stopped,registered;
    private final Runnable repair=this::repair;
    private final BroadcastReceiver receiver=new BroadcastReceiver(){
        @Override public void onReceive(Context c,Intent intent){
            if(intent==null||intent.getData()==null||!PACKAGE.equals(intent.getData().getSchemeSpecificPart()))return;
            if(Intent.ACTION_PACKAGE_FULLY_REMOVED.equals(intent.getAction()))stop();
            else if(Intent.ACTION_PACKAGE_REPLACED.equals(intent.getAction()))schedule();
        }
    };

    private CompanionUpdateRuntime(Context context,String signer,int uid,String baseline){
        this.context=context;this.signer=signer;this.uid=uid;this.baseline=baseline;
        thread=new HandlerThread("StepX.CompanionUpdate",android.os.Process.THREAD_PRIORITY_BACKGROUND);
        thread.start();handler=new Handler(thread.getLooper());
    }

    static JSONObject install(String signer,int uid,String baseline)throws Exception{
        if(android.os.Process.myUid()!=1000)throw new SecurityException("System update context required");
        Class<?> at=Class.forName("android.app.ActivityThread");
        Context context=(Context)at.getMethod("getSystemContext").invoke(at.getMethod("currentActivityThread").invoke(null));
        // runBoot has just verified the identity and all four grants. Preserve its
        // exact durable baseline; a later unexpected flag or signer is not repaired.
        stopInstalled();
        CompanionUpdateRuntime runtime=new CompanionUpdateRuntime(context,signer,uid,baseline);
        try{
            IntentFilter filter=new IntentFilter(Intent.ACTION_PACKAGE_REPLACED);
            filter.addAction(Intent.ACTION_PACKAGE_FULLY_REMOVED);filter.addDataScheme("package");
            context.registerReceiver(runtime.receiver,filter,null,runtime.handler,Context.RECEIVER_NOT_EXPORTED);
            runtime.registered=true;
            System.getProperties().put(REGISTRY,(Runnable)runtime::stop);
        }catch(Exception failure){runtime.stop();throw failure;}
        return new JSONObject().put("update_monitor",true).put("package",PACKAGE).put("uid",uid);
    }

    static void stopInstalled(){
        Object previous=System.getProperties().remove(REGISTRY);
        if(previous instanceof Runnable)((Runnable)previous).run();
    }

    private synchronized void schedule(){
        if(stopped)return;
        handler.removeCallbacks(repair);
        handler.postDelayed(repair,2000L);
    }

    private synchronized void repair(){
        if(stopped)return;
        long identity=Binder.clearCallingIdentity();
        try{
            if(!context.getPackageManager().getApplicationInfo(PACKAGE,0).enabled)return;
            // The existing fixed grant validates signer, UID, declaration, owner
            // and baseline before any mutation. It cannot grant any other package.
            JSONObject result=new JSONObject(CompanionGrant.restoreAfterUpdate(signer,baseline,uid));
            if(!"success".equals(result.optString("status"))||!result.optBoolean("all_granted"))
                throw new SecurityException(result.optString("error","Update grant read-back failed"));
            // Replacement kills the app process, and its old pairing loop may have
            // exited on the temporary revocation. Reattach the same scoped Binder.
            PairingDaemon.installSystem(signer,uid);
            android.util.Log.i("StepXCompanionUpdate","Companion update permissions restored");
        }catch(Exception error){
            android.util.Log.w("StepXCompanionUpdate","Companion update needs repair: "+error.getMessage());
        }finally{Binder.restoreCallingIdentity(identity);}
    }

    private synchronized void stop(){
        if(stopped)return;
        stopped=true;handler.removeCallbacksAndMessages(null);
        if(registered){context.unregisterReceiver(receiver);registered=false;}
        thread.quitSafely();
    }
}
