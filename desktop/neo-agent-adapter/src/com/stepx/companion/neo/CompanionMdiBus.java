package com.stepx.companion.neo;
import android.content.Context;
import org.json.JSONObject;
/** Calls only the signed companion's scoped Binder service, not a public socket. */
final class CompanionMdiBus implements AutoCloseable {
    Object call(String method,JSONObject args)throws Exception {
        Class<?> bridge=Class.forName("com.stepx.companion.app.PairingBridgeProvider");
        try {
            JSONObject r=(JSONObject)bridge.getMethod("mdi",JSONObject.class).invoke(null,new JSONObject().put("method",method).put("params",args));
            if(!r.optBoolean("ok"))throw new java.io.IOException(r.optString("error","MDI 未就绪"));
            return r.opt("content");
        }catch(java.lang.reflect.InvocationTargetException e){throw new java.io.IOException(e.getCause().getMessage());}
    }
    public void close(){}
    static JSONObject probe(){try(CompanionMdiBus bus=new CompanionMdiBus()){return (JSONObject)bus.call("probe",new JSONObject());}
        catch(Exception e){return CompanionBridgeProvider.failure("mdi_unavailable",e.getMessage());}}
}
