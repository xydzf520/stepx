package com.stepx.companion.app;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.Arrays;
import org.json.*;
import android.util.AtomicFile;

/** app_process probe: uses an isolated copy, never mutates live conversations. */
public final class PushDatabaseDeviceProbe {
    private static void require(boolean value,String label){if(!value)throw new AssertionError(label);}
    private static void atomic(File target,String text)throws Exception{
        AtomicFile file=new AtomicFile(target);FileOutputStream out=null;
        try{out=file.startWrite();out.write(text.getBytes(StandardCharsets.UTF_8));file.finishWrite(out);}
        catch(Exception e){if(out!=null)file.failWrite(out);throw e;}
    }
    private static JSONObject times(long[] ns)throws Exception{
        Arrays.sort(ns);return new JSONObject().put("median_ms",ns[ns.length/2]/1e6).put("p95_ms",ns[Math.min(ns.length-1,ns.length*95/100)]/1e6).put("samples",ns.length);
    }
    public static void main(String[] args)throws Exception{
        File dir=new File(args[0]);require(dir.getPath().startsWith("/data/local/tmp/stepx-storage-"),"isolated path required");
        byte[] a=Files.readAllBytes(new File(dir,"im_threads.json").toPath()),b=Files.readAllBytes(new File(dir,"push_feed.json").toPath());
        JSONObject threads=new JSONObject(new String(a,StandardCharsets.UTF_8)),feed=new JSONObject(new String(b,StandardCharsets.UTF_8));
        long start=System.nanoTime();PushDatabase db=new PushDatabase(dir);JSONObject[] loaded=db.load(dir);long migration=System.nanoTime()-start;
        require(PushDatabase.equivalent(threads,loaded[0])&&PushDatabase.equivalent(feed,loaded[1]),"migration round trip");
        long[] oldTimes=new long[30],newTimes=new long[30];
        File oldDir=new File(dir,"baseline");oldDir.mkdirs();
        for(int i=0;i<30;i++){
            if(threads.getJSONArray("threads").length()>0){JSONArray m=threads.getJSONArray("threads").getJSONObject(0).getJSONArray("messages");if(m.length()>0)m.getJSONObject(0).put("storageProbeRevision",i);}
            long rev=threads.optLong("revision")+1;threads.put("revision",rev);feed.put("revision",rev);
            start=System.nanoTime();atomic(new File(oldDir,"im_threads.json"),threads.toString());atomic(new File(oldDir,"push_feed.json"),feed.toString());oldTimes[i]=System.nanoTime()-start;
            start=System.nanoTime();db.save(threads,feed,false);newTimes[i]=System.nanoTime()-start;
        }
        JSONObject changed=new JSONObject(threads.toString()).put("revision",999999999);
        try{db.save(changed,new JSONObject().put("notifications","invalid"),false);throw new AssertionError("invalid save accepted");}catch(JSONException expected){}
        require(PushDatabase.equivalent(threads,db.load(dir)[0]),"transaction rollback");
        db.close();db=new PushDatabase(dir);loaded=db.load(dir);
        require(PushDatabase.equivalent(threads,loaded[0])&&PushDatabase.equivalent(feed,loaded[1]),"reopen persisted snapshot");db.close();
        require(Arrays.equals(a,Files.readAllBytes(new File(dir,"im_threads.json").toPath()))&&Arrays.equals(b,Files.readAllBytes(new File(dir,"push_feed.json").toPath())),"legacy files unchanged");
        int count=0;JSONArray list=threads.getJSONArray("threads");for(int i=0;i<list.length();i++)count+=list.getJSONObject(i).getJSONArray("messages").length();
        JSONObject result=new JSONObject().put("threads",list.length()).put("messages",count).put("notifications",feed.getJSONArray("notifications").length()).put("migration_ms",migration/1e6).put("json_commit",times(oldTimes)).put("sqlite_commit",times(newTimes)).put("roundtrip",true).put("rollback",true).put("reopen",true).put("originals_unchanged",true);
        System.out.println(result.toString());
    }
}
