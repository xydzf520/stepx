package com.stepx.companion.app;
import android.content.Context;
import android.security.keystore.*;
import android.util.Base64;
import org.json.*;
import java.security.KeyStore;
import javax.crypto.*;
import javax.crypto.spec.GCMParameterSpec;
import java.nio.charset.StandardCharsets;

/** Bounded, encrypted terminal summaries. Never stores API keys, screenshots or model traces. */
final class AgentHistory {
    private static final String ALIAS="stepx.gui.history.v1";
    private static String cachedData="",cachedIv="",cachedJson=null,lastObservedKey="";
    private static KeyStore keys()throws Exception{KeyStore k=KeyStore.getInstance("AndroidKeyStore");k.load(null);return k;}
    static synchronized JSONArray list(Context c)throws Exception{
        android.content.SharedPreferences p=c.getSharedPreferences("gui_history",0);String data=p.getString("data",""),iv=p.getString("iv","");if(data.isEmpty())return new JSONArray();
        if(cachedJson!=null&&data.equals(cachedData)&&iv.equals(cachedIv))return new JSONArray(cachedJson);
        Cipher cipher=Cipher.getInstance("AES/GCM/NoPadding");cipher.init(Cipher.DECRYPT_MODE,keys().getKey(ALIAS,null),new GCMParameterSpec(128,Base64.decode(iv,Base64.NO_WRAP)));
        String json=new String(cipher.doFinal(Base64.decode(data,Base64.NO_WRAP)),StandardCharsets.UTF_8);
        JSONArray arr=new JSONArray(json);
        cachedData=data;cachedIv=iv;cachedJson=json;
        return arr;
    }
    static synchronized void observe(Context c,JSONObject s){
        if(!s.optBoolean("terminal")||s.optString("task").isEmpty()||s.optLong("createdAt")<=c.getSharedPreferences("gui_history",0).getLong("clearedAt",0))return;
        try{
            JSONObject entry=new JSONObject();
            for(String field:new String[]{"task","goal","phase","progress","createdAt","actions","modelCalls","screenshots","result","model"})if(s.has(field))entry.put(field,s.opt(field));
            String entryKey=entry.toString();
            if(entryKey.equals(lastObservedKey))return;
            JSONArray old=list(c),next=new JSONArray();
            for(int i=0;i<old.length();i++)if(old.getJSONObject(i).optString("task").equals(entry.optString("task"))&&old.getJSONObject(i).toString().equals(entryKey)){lastObservedKey=entryKey;return;}
            next.put(entry);for(int i=0;i<old.length()&&next.length()<20;i++)if(!old.getJSONObject(i).optString("task").equals(entry.optString("task")))next.put(old.get(i));
            KeyStore k=keys();if(!k.containsAlias(ALIAS)){KeyGenerator g=KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES,"AndroidKeyStore");g.init(new KeyGenParameterSpec.Builder(ALIAS,KeyProperties.PURPOSE_ENCRYPT|KeyProperties.PURPOSE_DECRYPT).setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build());g.generateKey();}
            Cipher cipher=Cipher.getInstance("AES/GCM/NoPadding");cipher.init(Cipher.ENCRYPT_MODE,k.getKey(ALIAS,null));
            String nextJson=next.toString(),encData=Base64.encodeToString(cipher.doFinal(nextJson.getBytes(StandardCharsets.UTF_8)),Base64.NO_WRAP),encIv=Base64.encodeToString(cipher.getIV(),Base64.NO_WRAP);
            c.getSharedPreferences("gui_history",0).edit().putString("data",encData).putString("iv",encIv).commit();
            cachedData=encData;cachedIv=encIv;cachedJson=nextJson;lastObservedKey=entryKey;
        }catch(Exception ignored){/* History failure must not change execution or completion evidence. */}
    }
    static synchronized void clear(Context c){cachedData="";cachedIv="";cachedJson=null;lastObservedKey="";c.getSharedPreferences("gui_history",0).edit().clear().putLong("clearedAt",System.currentTimeMillis()).commit();}
}
