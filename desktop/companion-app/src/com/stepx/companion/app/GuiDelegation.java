package com.stepx.companion.app;
import android.content.Context;
import org.json.*;
import java.io.*;
import java.nio.file.*;

/** Same phone-owned queue for native MCP, local phone UI and authorized desktop. */
final class GuiDelegation {
    static synchronized JSONObject desktop(Context c,JSONObject q)throws Exception {
        String op=q.getString("agentOp");
        // Home observes the phone queue without adopting scopes, probing MDI,
        // installing keys, renewing leases or writing execution history.
        if(op.equals("status_readonly"))return NeoAgent.selected(c)?NeoAgent.statusReadonly(c):PhoneGuiAgent.status(c);
        if(op.equals("neo_enable"))return NeoAgent.enable(c);
        if(op.equals("neo_execution_mode"))return NeoAgent.executionMode(c,q.getString("executionMode"));
        if(op.equals("neo_status"))return NeoAgent.info(c);
        if(op.equals("neo_disable"))return NeoAgent.disable(c);
        if(op.equals("neo_settings")){NeoAgent.openSettings(c);return new JSONObject().put("ok",true);}
        if(NeoAgent.selected(c)&&!op.equals("install_key")&&!op.equals("config_info"))return NeoAgent.request(c,android.os.Process.myUid(),new JSONObject(q.toString()).put("op",op),true);
        if(op.equals("config_info"))return (NeoAgent.selected(c)?NeoAgent.status(c):PhoneGuiAgent.status(c)).put("publicKey",GuiModelVault.publicKey());
        if(op.equals("install_key")){
            File f=new File(c.getExternalFilesDir(null),"gui-key.sealed");
            try{if(NeoAgent.selected(c))NeoAgent.stepPlanKeyTarget(c);if(!Files.isRegularFile(f.toPath(),LinkOption.NOFOLLOW_LINKS)||f.length()!=384)throw new IOException("密钥密文文件无效");GuiModelVault.importSealed(c,Files.readAllBytes(f.toPath()));}
            finally{f.delete();}
            return NeoAgent.selected(c)?NeoAgent.importStepPlanKey(c):PhoneGuiAgent.status(c);
        }
        return PhoneGuiAgent.request(c,android.os.Process.myUid(),new JSONObject(q.toString()).put("op",op),true);
    }
    static synchronized JSONObject mcp(Context c,int uid,JSONObject q)throws Exception {return NeoAgent.selected(c)?NeoAgent.request(c,uid,q,false):PhoneGuiAgent.request(c,uid,q,false);}
}
