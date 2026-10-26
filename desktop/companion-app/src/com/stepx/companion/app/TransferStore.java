package com.stepx.companion.app;

import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.provider.OpenableColumns;
import org.json.JSONObject;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

/** Files move only over the already-authorized desktop connection; this app opens no server. */
final class TransferStore {
    static final long MAX_BYTES=256L*1024*1024;
    static final String AUTHORITY="com.stepx.companion.app.transfers";
    private static final ConcurrentHashMap<String,String> DIGEST_CACHE=new ConcurrentHashMap<>();
    private static volatile String cachedLinkPath="";
    private static volatile long cachedLinkMtime=-1L,cachedLinkLen=-1L;
    private static volatile JSONObject cachedLinkJson;
    private final Context context;
    static final class Link {
        final String id,name; final boolean online;
        Link(String id,String name,boolean online){this.id=id;this.name=name;this.online=online;}
    }
    static final class Entry {
        final String direction,id,name,sha256,status,computer,error;
        final long size,createdAt;
        Entry(String direction,JSONObject j,String status,String error){
            this.direction=direction;id=j.optString("id");name=j.optString("name");sha256=j.optString("sha256");size=j.optLong("size");createdAt=j.optLong("createdAt");this.status=status;this.error=error;
            computer=j.optString("computerName",direction.equals("inbox")?"已连接电脑":"电脑");
        }
        Uri uri(){return new Uri.Builder().scheme("content").authority(AUTHORITY).appendPath(direction).appendPath(id).build();}
    }
    TransferStore(Context c){context=c.getApplicationContext();}
    File root()throws IOException {File f=context.getExternalFilesDir(null);if(f==null)throw new IOException("手机存储暂不可用");if(!f.exists()&&!f.mkdirs())throw new IOException("无法创建传输目录");return f;}
    Link link(){try{
        File file=new File(root(),"desktop-link.json");
        long mtime=file.lastModified(),len=file.length();
        String path=file.getAbsolutePath();
        JSONObject j=cachedLinkJson;
        if(j==null||mtime<=0||mtime!=cachedLinkMtime||len!=cachedLinkLen||!path.equals(cachedLinkPath)){
            j=readJson(file);
            cachedLinkPath=path;cachedLinkMtime=mtime;cachedLinkLen=len;cachedLinkJson=j;
        }
        String id=j.optString("desktopId"),name=j.optString("computerName","电脑");long now=System.currentTimeMillis(),updated=j.optLong("updatedAtMs"),expires=j.optLong("expiresAtMs");
        boolean valid=j.optInt("protocol")==1&&uuid(id)&&j.optBoolean("connected")&&now>=updated-5000&&now-updated<=30000&&expires>=now&&expires<=updated+120000;
        return new Link(id,name,valid);
    }catch(Exception ignored){return new Link("","",false);}}
    static boolean uuid(String s){return s!=null&&s.matches("[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}");}
    static String safeName(String name){
        if(name==null)name="文件";
        name=name.replaceAll("[\\\\/\\p{Cntrl}]","_").trim();
        while(name.startsWith("."))name=name.substring(1);
        if(name.isEmpty())name="文件";
        if(name.equalsIgnoreCase("manifest.json")||name.equalsIgnoreCase("receipt.json"))name="file-"+name;
        if(name.toLowerCase(Locale.ROOT).endsWith(".json.tmp"))name=name+".file";
        int dot=name.lastIndexOf('.');String ext=dot>0&&name.length()-dot<=16?name.substring(dot):"",base=ext.isEmpty()?name:name.substring(0,dot);
        while((base+ext).getBytes(StandardCharsets.UTF_8).length>220&&!base.isEmpty())base=base.substring(0,base.offsetByCodePoints(base.length(),-1));
        name=base+ext;
        return name;
    }
    static JSONObject readJson(File f)throws Exception {
        if(!Files.isRegularFile(f.toPath(),LinkOption.NOFOLLOW_LINKS)||f.length()>32768)throw new IOException("传输信息不存在或无效");
        return new JSONObject(new String(Files.readAllBytes(f.toPath()),StandardCharsets.UTF_8));
    }
    private File directory(String direction,String id,boolean create)throws IOException {
        if(!direction.equals("inbox")&&!direction.equals("outbox"))throw new IOException("无效的传输方向");if(!uuid(id))throw new IOException("无效的传输记录");
        File base=root(),transfers=new File(base,"transfers"),side=new File(transfers,direction),dir=new File(side,id);
        for(File f:new File[]{transfers,side,dir}){if(Files.isSymbolicLink(f.toPath()))throw new IOException("传输目录无效");if(create&&!f.isDirectory()&&!f.mkdirs())throw new IOException("无法创建传输目录");}
        if(!dir.getCanonicalPath().startsWith(base.getCanonicalPath()+File.separator))throw new IOException("传输目录无效");return dir;
    }
    private JSONObject validatedManifest(String direction,String id)throws Exception {
        File dir=directory(direction,id,false);JSONObject j=readJson(new File(dir,"manifest.json"));String name=j.optString("name");
        if(j.optInt("protocol")!=1||!id.equals(j.optString("id"))||name.isEmpty()||!name.equals(safeName(name))||!j.optString("sha256").matches("[0-9a-f]{64}")||j.optLong("size",-1)<0||j.optLong("size")>MAX_BYTES)throw new IOException("文件传输信息无效");
        File file=new File(dir,name);if(!Files.isRegularFile(file.toPath(),LinkOption.NOFOLLOW_LINKS)||file.length()!=j.optLong("size"))throw new IOException("文件还没有接收完整");return j;
    }
    Entry get(String direction,String id)throws Exception {
        JSONObject j=validatedManifest(direction,id);String status=direction.equals("inbox")?"ready":"queued",error="";
        if(direction.equals("outbox")){File receipt=new File(directory(direction,id,false),"receipt.json");if(receipt.exists()){JSONObject r=readJson(receipt);if(r.optInt("protocol")==1&&id.equals(r.optString("id"))){
            if("received".equals(r.optString("status"))&&r.optLong("size",-1)==j.optLong("size")&&r.optString("sha256").equals(j.optString("sha256")))status="received";
            else if("error".equals(r.optString("status"))){status="error";error=r.optString("error","电脑接收失败");}
        }}}
        return new Entry(direction,j,status,error);
    }
    List<Entry> list()throws IOException {
        List<Entry> result=new ArrayList<>();
        for(String direction:new String[]{"inbox","outbox"}){File dir=new File(new File(root(),"transfers"),direction);if(Files.isSymbolicLink(dir.toPath()))continue;File[] files=dir.listFiles();if(files==null)continue;Arrays.sort(files,Comparator.comparingLong(File::lastModified).reversed());int examined=0;
            for(File f:files){if(++examined>300)break;if(!uuid(f.getName()))continue;try{result.add(get(direction,f.getName()));}catch(Exception ignored){}}
        }
        result.sort((a,b)->Long.compare(b.createdAt,a.createdAt));return result;
    }
    Entry enqueue(Uri uri,Link target)throws Exception {
        Link current=link();if(!target.online||!current.online||!current.id.equals(target.id))throw new IOException("电脑连接已变化，请重新选择文件发送");
        String name="文件";long reported=-1;
        try(Cursor cursor=context.getContentResolver().query(uri,new String[]{OpenableColumns.DISPLAY_NAME,OpenableColumns.SIZE},null,null,null)){if(cursor!=null&&cursor.moveToFirst()){int n=cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME),s=cursor.getColumnIndex(OpenableColumns.SIZE);if(n>=0&&!cursor.isNull(n))name=cursor.getString(n);if(s>=0&&!cursor.isNull(s))reported=cursor.getLong(s);}}
        if(reported>MAX_BYTES)throw new IOException("单个文件最多支持 256 MB");name=safeName(name);String id=UUID.randomUUID().toString();File dir=directory("outbox",id,true),file=new File(dir,name);
        try{
            MessageDigest digest=MessageDigest.getInstance("SHA-256");long size=0;byte[] buffer=new byte[65536];
            try(InputStream in=context.getContentResolver().openInputStream(uri);FileOutputStream out=new FileOutputStream(file)){if(in==null)throw new IOException("无法读取所选文件");int count;while((count=in.read(buffer))!=-1){size+=count;if(size>MAX_BYTES)throw new IOException("单个文件最多支持 256 MB");out.write(buffer,0,count);digest.update(buffer,0,count);}out.getFD().sync();}
            current=link();if(!current.online||!current.id.equals(target.id))throw new IOException("电脑连接已断开，请连接后重新发送");
            JSONObject manifest=new JSONObject().put("protocol",1).put("id",id).put("name",name).put("size",size).put("sha256",hex(digest.digest())).put("status","queued").put("createdAt",System.currentTimeMillis()).put("targetDesktopId",target.id).put("computerName",target.name);
            writeJson(new File(dir,"manifest.json"),manifest);return get("outbox",id);
        }catch(Exception e){deleteDirectory(dir);throw e;}
    }
    static void writeJson(File f,JSONObject j)throws Exception {File tmp=new File(f.getParentFile(),f.getName()+".tmp");Files.write(tmp.toPath(),j.toString().getBytes(StandardCharsets.UTF_8));Files.move(tmp.toPath(),f.toPath(),StandardCopyOption.ATOMIC_MOVE,StandardCopyOption.REPLACE_EXISTING);}
    private static String hex(byte[] data){StringBuilder s=new StringBuilder();for(byte b:data)s.append(String.format(Locale.ROOT,"%02x",b&255));return s.toString();}
    File verifiedFile(String direction,String id)throws Exception {
        Entry e=get(direction,id);File payload=new File(directory(direction,id,false),e.name);
        long len=payload.length(),mtime=payload.lastModified();
        String cacheKey=payload.getAbsolutePath()+":"+len+":"+mtime;
        String digestHex=DIGEST_CACHE.get(cacheKey);
        if(digestHex==null){
            MessageDigest d=MessageDigest.getInstance("SHA-256");byte[] b=new byte[65536];long size=0;
            try(InputStream in=new FileInputStream(payload)){int n;while((n=in.read(b))!=-1){size+=n;if(size>MAX_BYTES)throw new IOException("文件超过大小限制");d.update(b,0,n);}}
            if(size!=e.size)throw new IOException("文件校验未通过，请重新传输");
            digestHex=hex(d.digest());
            if(payload.length()==len&&payload.lastModified()==mtime){
                if(DIGEST_CACHE.size()>512)DIGEST_CACHE.clear();
                DIGEST_CACHE.put(cacheKey,digestHex);
            }
        }
        if(len!=e.size||!digestHex.equals(e.sha256))throw new IOException("文件校验未通过，请重新传输");
        return payload;
    }
    void save(Entry entry,Uri destination)throws Exception {File f=verifiedFile(entry.direction,entry.id);try(InputStream in=new FileInputStream(f);OutputStream out=context.getContentResolver().openOutputStream(destination,"wt")){if(out==null)throw new IOException("无法写入所选位置");byte[] buf=new byte[65536];int n;while((n=in.read(buf))!=-1)out.write(buf,0,n);}}
    void delete(Entry e)throws IOException {deleteDirectory(directory(e.direction,e.id,false));}
    private static void deleteDirectory(File dir)throws IOException {if(Files.isSymbolicLink(dir.toPath()))throw new IOException("无效目录");File[] contents=dir.listFiles();if(contents!=null)for(File f:contents){if(f.isDirectory()&&!Files.isSymbolicLink(f.toPath()))throw new IOException("传输目录中存在异常子目录");Files.deleteIfExists(f.toPath());}Files.deleteIfExists(dir.toPath());}
    static String mime(String name){int dot=name.lastIndexOf('.');String ext=dot<0?"":name.substring(dot+1).toLowerCase(Locale.ROOT);String mime=android.webkit.MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext);return mime==null?"application/octet-stream":mime;}
    static String size(long bytes){return bytes>=1024*1024?String.format(Locale.ROOT,"%.1f MB",bytes/1048576.0):bytes>=1024?String.format(Locale.ROOT,"%.1f KB",bytes/1024.0):bytes+" B";}
}
