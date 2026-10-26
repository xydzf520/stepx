package com.stepx.companion.app;

import android.content.Context;
import android.util.Base64;
import org.json.*;
import java.io.*;
import java.nio.ByteBuffer;
import java.nio.charset.*;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.util.*;

/** Private, offline HTML library. Uploads commit only after a complete SHA-256 check. */
final class StudioLocal {
    static final int MAX_BYTES=2*1024*1024, CHUNK_BYTES=12*1024;
    private static final long EXPIRY=30*60*1000;
    private static final char[] HEX="0123456789abcdef".toCharArray();
    static boolean validId(String id){return id!=null&&id.matches("local-[a-f0-9]{32}");}
    static String origin(String id){if(!validId(id))throw new IllegalArgumentException("作品编号无效");return "https://"+id+".local.stepx.invalid";}
    static boolean isDocument(String id,String raw){return (origin(id)+"/index.html").equals(raw==null?"":raw.split("#",2)[0]);}
    static String hash(byte[] data)throws Exception {byte[] b=MessageDigest.getInstance("SHA-256").digest(data);char[] s=new char[b.length*2];for(int i=0;i<b.length;i++){int v=b[i]&255;s[i*2]=HEX[v>>>4];s[i*2+1]=HEX[v&15];}return new String(s);}
    private static File root(Context c)throws IOException {File f=new File(c.getFilesDir(),"studio-local");if(!f.isDirectory()&&!f.mkdirs())throw new IOException("无法创建作品目录");return f;}
    private static File dir(Context c,String id)throws IOException {if(!validId(id))throw new IOException("作品编号无效");return new File(root(c),id);}
    private static File stage(Context c,String upload)throws IOException {if(!validId(upload))throw new IOException("上传编号无效");return new File(root(c),"upload-"+upload);}
    private static JSONObject readJson(File file)throws Exception {if(!file.isFile()||file.length()>16384)throw new IOException("作品记录无效");return new JSONObject(new String(Files.readAllBytes(file.toPath()),StandardCharsets.UTF_8));}
    private static void write(File file,byte[] bytes)throws IOException {try(FileOutputStream o=new FileOutputStream(file)){o.write(bytes);o.getFD().sync();}}
    private static void remove(File d){File[] fs=d.listFiles();if(fs!=null)for(File f:fs)if(f.isFile())f.delete();d.delete();}
    private static void clean(Context c)throws IOException {File[] fs=root(c).listFiles();if(fs!=null)for(File f:fs)if(f.getName().startsWith("upload-")&&System.currentTimeMillis()-f.lastModified()>EXPIRY)remove(f);}
    static synchronized JSONObject begin(Context c,JSONObject q)throws Exception {
        clean(c);String name=q.getString("name").trim(),sha=q.getString("sha256");int size=q.getInt("size");
        if(name.isEmpty()||name.codePointCount(0,name.length())>60||name.matches("(?s).*[\\p{Cntrl}].*")||!sha.matches("[a-f0-9]{64}")||size<1||size>MAX_BYTES)throw new IOException("作品名称或文件大小无效（最大 2 MB）");
        File[] files=root(c).listFiles();int count=0,uploads=0;if(files!=null)for(File f:files){if(validId(f.getName()))count++;else if(f.getName().startsWith("upload-"))uploads++;}
        if(count>=40||uploads>=3)throw new IOException("作品库或上传队列已满，请先整理作品");
        String id="local-"+UUID.randomUUID().toString().replace("-","");File d=stage(c,id);if(!d.mkdir())throw new IOException("无法开始上传");
        JSONObject meta=new JSONObject().put("id",id).put("name",name).put("size",size).put("sha256",sha).put("createdAt",System.currentTimeMillis()).put("offline",true);
        write(new File(d,"meta.json"),meta.toString().getBytes(StandardCharsets.UTF_8));write(new File(d,"body.part"),new byte[0]);
        return new JSONObject().put("ok",true).put("upload",id).put("offset",0);
    }
    static synchronized JSONObject chunk(Context c,JSONObject q)throws Exception {
        File d=stage(c,q.getString("upload"));JSONObject meta=readJson(new File(d,"meta.json"));File body=new File(d,"body.part");long offset=q.getLong("offset");
        if(System.currentTimeMillis()-meta.getLong("createdAt")>EXPIRY)throw new IOException("上传已过期，请重新发送");
        byte[] bytes=Base64.decode(q.getString("data"),Base64.DEFAULT);
        if(offset!=body.length()||bytes.length==0||bytes.length>CHUNK_BYTES||offset+bytes.length>meta.getInt("size"))throw new IOException("上传分片顺序或大小无效");
        try(FileOutputStream out=new FileOutputStream(body,true)){out.write(bytes);}
        return new JSONObject().put("ok",true).put("upload",q.getString("upload")).put("offset",body.length());
    }
    private static boolean containsIgnoreCase(String src,String target){int len=target.length(),max=src.length()-len;for(int i=0;i<=max;i++)if(src.regionMatches(true,i,target,0,len))return true;return false;}
    static synchronized JSONObject commit(Context c,String upload)throws Exception {
        File d=stage(c,upload);JSONObject meta=readJson(new File(d,"meta.json"));File body=new File(d,"body.part");
        if(body.length()!=meta.getInt("size")||body.length()>MAX_BYTES)throw new IOException("文件尚未完整上传");
        byte[] bytes=Files.readAllBytes(body.toPath());if(!hash(bytes).equals(meta.getString("sha256")))throw new IOException("文件校验失败，请重新发送");
        String html=StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes)).toString();
        if(!containsIgnoreCase(html,"<html")||!containsIgnoreCase(html,"</html>"))throw new IOException("请发送完整 HTML 作品");
        try(FileOutputStream o=new FileOutputStream(body,true)){o.getFD().sync();}
        if(!body.renameTo(new File(d,"index.html"))||!d.renameTo(dir(c,upload)))throw new IOException("保存作品失败");
        return new JSONObject(meta.toString()).put("ok",true);
    }
    static synchronized JSONObject abort(Context c,String id)throws Exception {remove(stage(c,id));return new JSONObject().put("ok",true);}
    static synchronized JSONObject item(Context c,String id)throws Exception {
        File d=dir(c,id);JSONObject meta=readJson(new File(d,"meta.json"));File html=new File(d,"index.html");if(!id.equals(meta.getString("id"))||html.length()!=meta.getInt("size")||html.length()>MAX_BYTES)throw new IOException("作品文件不完整");return meta;
    }
    static synchronized byte[] html(Context c,String id)throws Exception {JSONObject meta=item(c,id);byte[] b=Files.readAllBytes(new File(dir(c,id),"index.html").toPath());if(!hash(b).equals(meta.getString("sha256")))throw new IOException("作品文件校验失败");return b;}
    static synchronized JSONArray list(Context c)throws Exception {
        clean(c);ArrayList<JSONObject> items=new ArrayList<>();File[] fs=root(c).listFiles();if(fs!=null)for(File f:fs)if(validId(f.getName()))try{items.add(item(c,f.getName()));}catch(Exception ignored){}
        items.sort((a,b)->Long.compare(b.optLong("createdAt"),a.optLong("createdAt")));JSONArray out=new JSONArray();for(JSONObject item:items)out.put(item);return out;
    }
    static synchronized JSONObject delete(Context c,String id)throws Exception {item(c,id);remove(dir(c,id));new android.os.Handler(android.os.Looper.getMainLooper()).post(()->android.webkit.WebStorage.getInstance().deleteOrigin(origin(id)));return new JSONObject().put("ok",true);}
    static JSONObject status(Context c)throws Exception {android.content.SharedPreferences p=c.getSharedPreferences("studio-local-state",0);return new JSONObject().put("ok",true).put("lastOpen",p.getString("lastOpen","")).put("openToken",p.getString("openToken","")).put("readyAt",p.getLong("readyAt",0)).put("title",p.getString("title","")).put("errorCount",p.getInt("errorCount",0)).put("lastError",p.getString("lastError",""));}
}
