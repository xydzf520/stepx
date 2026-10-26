package com.stepx.companion.app;

import android.content.*;
import android.database.*;
import android.database.sqlite.*;
import android.graphics.*;
import android.media.ThumbnailUtils;
import android.os.Environment;
import android.util.AtomicFile;
import android.util.Size;
import org.json.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;

/** A verified offload journal. Original files are removed only by commit(), never by scanning. */
final class ExpansionStore {
    static final String AUTH="com.stepx.companion.app.expansion", DOCS=AUTH+".documents";
    static final long MAX_FILE=32L*1024*1024*1024, RESERVE=128L*1024*1024;
    private static final Object COMMIT_LOCK=new Object(),HOST_LOCK=new Object();
    private static final char[] HEX="0123456789abcdef".toCharArray();
    private static final Map<String,String> DIGEST_CACHE=Collections.synchronizedMap(new LinkedHashMap<String,String>(128,0.75f,true){@Override protected boolean removeEldestEntry(Map.Entry<String,String> eldest){return size()>512;}});
    private static Db shared;
    private static volatile boolean opInitialized=false;
    private static volatile long policyGeneration=0;
    private static volatile long cachedPolicyMtime=-1L,cachedPolicyLen=-1L;
    private static volatile String cachedPolicyRaw=null;
    private static volatile long cachedHostMtime=-1L,cachedHostLen=-1L;
    private static volatile String cachedHostRaw=null;
    private static volatile long cachedInspectAtMs=0L;
    private static volatile String cachedInspectRaw=null;
    private final Context context;
    private final SQLiteDatabase db;
    private final File sharedRoot=Environment.getExternalStorageDirectory();
    static final class Db extends SQLiteOpenHelper {
        Db(Context c){super(c,"expansion.db",null,1);}
        public void onCreate(SQLiteDatabase d){d.execSQL("CREATE TABLE files (id TEXT PRIMARY KEY, path TEXT, name TEXT, kind TEXT, state TEXT, desktop TEXT, size INTEGER, modified INTEGER, updated INTEGER, body TEXT)");d.execSQL("CREATE INDEX by_path ON files(path)");d.execSQL("CREATE TABLE requests (id TEXT PRIMARY KEY, mode TEXT, state TEXT, error TEXT, updated INTEGER)");}
        public void onUpgrade(SQLiteDatabase d,int old,int next){throw new IllegalStateException("不支持的扩容记录版本");}
    }
    ExpansionStore(Context c){context=c.getApplicationContext();synchronized(ExpansionStore.class){if(!opInitialized){opInitialized=true;try{android.provider.Settings.Secure.putInt(context.getContentResolver(),context.getPackageName()+"_op_100",4);}catch(Exception ignored){}}if(shared==null)shared=new Db(context);db=shared.getWritableDatabase();}}
    static boolean id(String value){return value!=null&&value.matches("[a-f0-9]{32}");}
    private static String uid(){return UUID.randomUUID().toString().replace("-","");}
    private File privateRoot()throws IOException{File dir=new File(context.getFilesDir(),"expansion");if(!dir.isDirectory()&&!dir.mkdirs())throw new IOException("无法创建扩容记录");return dir;}
    File incoming(String id)throws IOException{if(!id(id))throw new IOException("无效文件编号");File root=context.getExternalFilesDir("expansion-incoming");if(root==null)throw new IOException("手机存储不可用");File dir=new File(root,id);if(Files.isSymbolicLink(dir.toPath())||(!dir.isDirectory()&&!dir.mkdirs()))throw new IOException("接收目录不可用");return new File(dir,"original.part");}
    File cached(String id)throws IOException{if(!id(id))throw new IOException("无效文件编号");File root=context.getExternalCacheDir();if(root==null)throw new IOException("手机缓存不可用");File dir=new File(root,"expansion");if(!dir.isDirectory()&&!dir.mkdirs())throw new IOException("无法创建缓存");return new File(dir,id);}
    File thumbnail(String id)throws IOException{if(!id(id))throw new IOException("无效文件编号");return new File(privateRoot(),id+".jpg");}
    private void notifyFiles(){context.getContentResolver().notifyChange(android.provider.DocumentsContract.buildChildDocumentsUri(DOCS,"root"),null);}
    static void writeAtomic(File path,JSONObject value)throws Exception{AtomicFile a=new AtomicFile(path);FileOutputStream out=null;try{out=a.startWrite();out.write(value.toString().getBytes(StandardCharsets.UTF_8));a.finishWrite(out);}catch(Exception e){if(out!=null)a.failWrite(out);throw e;}}
    private JSONObject read(File path)throws Exception{if(!path.isFile()||path.length()>65536)throw new IOException("记录无效");return new JSONObject(new String(Files.readAllBytes(path.toPath()),StandardCharsets.UTF_8));}
    synchronized JSONObject policy()throws Exception{
        File f=new File(privateRoot(),"policy.json");
        if(f.isFile()){
            long mtime=f.lastModified(),len=f.length();
            String raw=cachedPolicyRaw;
            if(raw!=null&&mtime>0&&mtime==cachedPolicyMtime&&len==cachedPolicyLen)return new JSONObject(raw);
            if(len>65536)throw new IOException("记录无效");
            raw=new String(Files.readAllBytes(f.toPath()),StandardCharsets.UTF_8);
            JSONObject parsed=new JSONObject(raw);
            cachedPolicyMtime=mtime;cachedPolicyLen=len;cachedPolicyRaw=raw;
            return parsed;
        }
        return new JSONObject().put("enabled",false).put("photos",true).put("videos",false).put("documents",false).put("retentionDays",7).put("epoch",uid()).put("desktopId","");
    }
    JSONObject host()throws Exception{
        File f=new File(privateRoot(),"host.json");
        if(!f.isFile())return new JSONObject();
        long mtime=f.lastModified(),len=f.length();
        String raw=cachedHostRaw;
        if(raw!=null&&mtime>0&&mtime==cachedHostMtime&&len==cachedHostLen)return new JSONObject(raw);
        if(len>65536)throw new IOException("记录无效");
        raw=new String(Files.readAllBytes(f.toPath()),StandardCharsets.UTF_8);
        JSONObject parsed=new JSONObject(raw);
        cachedHostMtime=mtime;cachedHostLen=len;cachedHostRaw=raw;
        return parsed;
    }
    boolean online(JSONObject h){TransferStore.Link l=new TransferStore(context).link();return l.online&&l.id.equals(h.optString("desktopId"))&&System.currentTimeMillis()-h.optLong("seenAt")<35000;}
    JSONObject offer(JSONObject q)throws Exception{synchronized(HOST_LOCK){
        TransferStore.Link link=new TransferStore(context).link();if(!link.online||!link.id.equals(q.optString("desktopId")))throw new IOException("请先连接这台电脑");
        JSONObject h=new JSONObject().put("desktopId",link.id).put("name",link.name).put("folder",q.optString("folder")).put("ready",q.optBoolean("ready")).put("seenAt",System.currentTimeMillis());File hf=new File(privateRoot(),"host.json");writeAtomic(hf,h);cachedHostMtime=hf.lastModified();cachedHostLen=hf.length();cachedHostRaw=h.toString();return status();
    }}
    JSONObject configure(JSONObject q)throws Exception{policyGeneration++;synchronized(COMMIT_LOCK){
        JSONObject old=policy(),h=host();boolean enable=q.optBoolean("enabled");
        String target=q.optString("desktopId",old.optString("desktopId"));
        int days=q.optInt("retentionDays",7);if(days!=0&&days!=7&&days!=30)throw new IOException("保留时间无效");
        if(enable&&(!Environment.isExternalStorageManager()||!online(h)||!h.optBoolean("ready")||!target.equals(h.optString("desktopId"))))throw new IOException("请连接电脑、设置保存文件夹并授予文件管理权限");
        if(enable&&!q.optBoolean("photos")&&!q.optBoolean("videos")&&!q.optBoolean("documents"))throw new IOException("请至少选择一类文件");
        JSONObject p=new JSONObject().put("enabled",enable).put("photos",q.optBoolean("photos")).put("videos",q.optBoolean("videos")).put("documents",q.optBoolean("documents")).put("retentionDays",days).put("desktopId",target).put("epoch",uid());
        recoverStaged();File pf=new File(privateRoot(),"policy.json");writeAtomic(pf,p);cachedPolicyMtime=pf.lastModified();cachedPolicyLen=pf.length();cachedPolicyRaw=p.toString();cachedInspectRaw=null;cachedInspectAtMs=0L;return status();
    }}
    private void clearStaleFailed(){try(Cursor c=db.rawQuery("SELECT id,body FROM files WHERE state='failed'",null)){while(c.moveToNext())try{JSONObject old=new JSONObject(c.getString(1));File f=source(old.getString("path"));if(!f.isFile()||f.length()!=old.optLong("size")||f.lastModified()!=old.optLong("modified")||!digest(f).equals(old.optString("sha256"))){if(db.delete("files","id=?",new String[]{c.getString(0)})>0){cachedInspectRaw=null;cachedInspectAtMs=0L;}}}catch(Exception ignored){}}}
    JSONObject status()throws Exception{
        clearStaleFailed();
        JSONObject h=host(),result=new JSONObject().put("ok",true).put("policy",policy()).put("host",h).put("online",online(h)).put("permission",Environment.isExternalStorageManager()).put("availableBytes",sharedRoot.getUsableSpace());
        Map<String,Long> cacheSizes=new HashMap<>(),thumbSizes=new HashMap<>();
        File extCache=context.getExternalCacheDir();
        if(extCache!=null){File cacheDir=new File(extCache,"expansion");File[] cfs=cacheDir.listFiles();if(cfs!=null)for(File cf:cfs)cacheSizes.put(cf.getName(),cf.length());}
        File[] pfs=privateRoot().listFiles();if(pfs!=null)for(File pf:pfs)thumbSizes.put(pf.getName(),pf.length());
        long count=0,released=0,archived=0;try(Cursor c=db.rawQuery("SELECT body FROM files WHERE state='archived'",null)){while(c.moveToNext()){JSONObject j=new JSONObject(c.getString(0));count++;archived+=j.optLong("size");if(j.optBoolean("cleanupVerified")){String fid=j.getString("id");Long cLen=cacheSizes.get(fid);if(cLen==null)cLen=cacheSizes.get(fid+".bin");Long tLen=thumbSizes.get(fid+".jpg");released+=Math.max(0,j.optLong("size")-(cLen==null?0L:cLen)-(tLen==null?0L:tLen)-j.optLong("galleryBytes"));}}}result.put("count",count).put("releasedBytes",released).put("archivedBytes",archived);
        try(Cursor c=db.rawQuery("SELECT COUNT(*) FROM files WHERE state='failed'",null)){c.moveToFirst();result.put("failed",c.getLong(0));}
        return result.put("items",list("all","date",true,0,20,"archived"));
    }
    JSONObject get(String id)throws Exception{if(!id(id))throw new IOException("无效文件编号");try(Cursor c=db.query("files",new String[]{"body"},"id=?",new String[]{id},null,null,null)){if(!c.moveToFirst())throw new IOException("文件记录不存在");return new JSONObject(c.getString(0));}}
    void put(JSONObject j)throws Exception{ContentValues v=new ContentValues();for(String k:new String[]{"id","path","name","kind","state","desktop"})v.put(k,j.getString(k));v.put("size",j.getLong("size"));v.put("modified",j.getLong("modified"));v.put("updated",System.currentTimeMillis());v.put("body",j.toString());db.insertWithOnConflict("files",null,v,SQLiteDatabase.CONFLICT_REPLACE);cachedInspectRaw=null;cachedInspectAtMs=0L;notifyFiles();}
    JSONArray list(String kind,String sort,boolean descending,int offset,int limit,String state)throws Exception{
        String order=sort.equals("name")?"name COLLATE NOCASE":sort.equals("size")?"size":"modified";order+=(descending?" DESC":" ASC")+", id ASC";
        ArrayList<String> args=new ArrayList<>();String where="state!='forgotten'";if(!kind.equals("all")){where+=" AND kind=?";args.add(kind);}if(!state.equals("all")){where+=" AND state=?";args.add(state);}
        JSONArray result=new JSONArray();try(Cursor c=db.query("files",new String[]{"body"},where,args.toArray(new String[0]),null,null,order,Math.max(0,offset)+","+Math.min(100,Math.max(1,limit)))){while(c.moveToNext())result.put(new JSONObject(c.getString(0)));}return result;
    }
    static String kind(String relative){return ExpansionRules.kind(relative);}
    private File source(String relative)throws Exception{
        if(relative.startsWith("/")||relative.contains("\\")||relative.contains("\u0000")||relative.length()>4096||kind(relative).isEmpty())throw new IOException("文件不在已支持的迁移目录");
        File f=sharedRoot;for(String part:relative.split("/")){if(part.isEmpty()||part.equals(".")||part.equals("..")||part.startsWith("."))throw new IOException("文件路径无效");f=new File(f,part);if(Files.isSymbolicLink(f.toPath()))throw new IOException("不迁移符号链接");}
        if(!f.getCanonicalPath().startsWith(sharedRoot.getCanonicalPath()+File.separator))throw new IOException("文件超出共享目录");return f;
    }
    private JSONObject active(String desktop)throws Exception{JSONObject p=policy(),h=host();if(!p.optBoolean("enabled")||!desktop.equals(p.optString("desktopId"))||!online(h)||!h.optBoolean("ready")||!desktop.equals(h.optString("desktopId")))throw new IOException("自动迁移已暂停或电脑连接已变化");if(!Environment.isExternalStorageManager())throw new IOException("请开启文件管理权限");return p;}
    private boolean eligible(File f,String relative,JSONObject p){long age=System.currentTimeMillis()-f.lastModified();return f.isFile()&&f.length()>0&&f.length()<=MAX_FILE&&p.optBoolean(kind(relative))&&System.currentTimeMillis()>=ExpansionRules.readyAt(f.lastModified(),p.optInt("retentionDays",7));}
    JSONArray scan(String desktop)throws Exception{
        JSONObject p=active(desktop);JSONArray out=new JSONArray();Set<String> excluded=new HashSet<>();
        try(Cursor c=db.rawQuery("SELECT path,body FROM files WHERE state IN ('archived','restored','forgotten')",null)){while(c.moveToNext()){excluded.add(c.getString(0));JSONObject j=new JSONObject(c.getString(1));if(j.has("restoredPath"))excluded.add(j.getString("restoredPath"));}}
        // Resume a crash between shortcut creation and original removal even if the original has been renamed.
        try(Cursor c=db.rawQuery("SELECT body FROM files WHERE state='releasing' AND desktop=? LIMIT 20",new String[]{desktop})){while(c.moveToNext())out.put(new JSONObject(c.getString(0)).put("resume",true));}
        ArrayDeque<File> queue=new ArrayDeque<>();for(String dir:new String[]{"DCIM","Documents","Download"})queue.add(new File(sharedRoot,dir));int visited=0;
        while(!queue.isEmpty()&&visited++<20000&&out.length()<40){File f=queue.removeFirst();if(Files.isSymbolicLink(f.toPath())||f.getName().startsWith("."))continue;if(f.isDirectory()){File[] children=f.listFiles();if(children!=null)Collections.addAll(queue,children);continue;}String relative=sharedRoot.toPath().relativize(f.toPath()).toString();if(excluded.contains(relative)||!eligible(f,relative,p))continue;
            out.put(new JSONObject().put("path",relative).put("name",f.getName()).put("kind",kind(relative)).put("size",f.length()).put("modified",f.lastModified()));
        }return out;
    }
    static String digest(File f)throws Exception{long len=f.length(),mod=f.lastModified();String key=f.getAbsolutePath()+"|"+len+"|"+mod,cached=DIGEST_CACHE.get(key);if(cached!=null)return cached;MessageDigest h=MessageDigest.getInstance("SHA-256");try(InputStream in=new FileInputStream(f)){byte[] b=new byte[1024*1024];int n;while((n=in.read(b))!=-1)h.update(b,0,n);}byte[] raw=h.digest();char[] hex=new char[raw.length*2];for(int i=0;i<raw.length;i++){int v=raw[i]&255;hex[i*2]=HEX[v>>>4];hex[i*2+1]=HEX[v&15];}String res=new String(hex);if(f.length()==len&&f.lastModified()==mod)DIGEST_CACHE.put(key,res);return res;}
    /** Read-only draft preview. Never configures policy, prepares or removes a file. */
    JSONObject inspect(JSONObject draft)throws Exception{
        if(!Environment.isExternalStorageManager())throw new IOException("请先开启手机文件管理权限");
        if(draft==null){
            String cached=cachedInspectRaw;
            if(cached!=null&&System.currentTimeMillis()-cachedInspectAtMs<3000L)return new JSONObject(cached);
        }
        JSONObject p=draft==null?policy():draft;
        int days=p.optInt("retentionDays",7);if(days!=0&&days!=7&&days!=30)throw new IOException("保留时间无效");
        Set<String> excluded=new HashSet<>();
        try(Cursor c=db.rawQuery("SELECT path,body FROM files WHERE state IN ('archived','restored','forgotten')",null)){
            while(c.moveToNext()){excluded.add(c.getString(0));JSONObject j=new JSONObject(c.getString(1));if(j.has("restoredPath"))excluded.add(j.getString("restoredPath"));}
        }
        long now=System.currentTimeMillis(),bytes=0,next=0;int visited=0,selected=0,eligible=0,retained=0,writing=0,skipped=0,invalid=0;
        JSONObject counts=new JSONObject().put("photos",0).put("videos",0).put("documents",0);JSONArray examples=new JSONArray();
        ArrayDeque<File> queue=new ArrayDeque<>();for(String dir:new String[]{"DCIM","Documents","Download"})queue.add(new File(sharedRoot,dir));
        while(!queue.isEmpty()&&visited++<20000){
            File f=queue.removeFirst();if(Files.isSymbolicLink(f.toPath())||f.getName().startsWith("."))continue;
            if(f.isDirectory()){File[] children=f.listFiles();if(children!=null)Collections.addAll(queue,children);continue;}
            if(!f.isFile())continue;
            String relative=sharedRoot.toPath().relativize(f.toPath()).toString(),kind=kind(relative);if(kind.isEmpty())continue;
            counts.put(kind,counts.optInt(kind)+1);if(!p.optBoolean(kind))continue;selected++;
            if(excluded.contains(relative)){skipped++;continue;}
            if(f.length()==0||f.length()>MAX_FILE){invalid++;continue;}
            long ready=ExpansionRules.readyAt(f.lastModified(),days);
            if(now<ready){if(now-f.lastModified()<120000L)writing++;else retained++;if(next==0||ready<next)next=ready;continue;}
            eligible++;bytes+=f.length();
            if(examples.length()<5)examples.put(new JSONObject().put("name",f.getName()).put("kind",kind).put("size",f.length()));
        }
        JSONObject res=new JSONObject().put("checkedAt",now).put("counts",counts).put("selectedCount",selected).put("candidateCount",eligible).put("candidateBytes",bytes)
            .put("retainedCount",retained).put("writingCount",writing).put("excludedCount",skipped).put("invalidCount",invalid).put("nextEligibleAt",next).put("partial",!queue.isEmpty()).put("examples",examples);
        if(draft==null){cachedInspectRaw=res.toString();cachedInspectAtMs=now;}
        return res;
    }
    JSONObject prepare(JSONObject q)throws Exception{
        String desktop=q.getString("desktopId"),path=q.getString("path");JSONObject p=active(desktop);File f=source(path);if(!eligible(f,path,p))throw new IOException("文件正在写入、未到保留时间或类型未勾选");long size=f.length(),mtime=f.lastModified();
        String sha=digest(f);if(f.length()!=size||f.lastModified()!=mtime)throw new IOException("文件发生变化，本次跳过");
        JSONObject j=null;try(Cursor c=db.rawQuery("SELECT body FROM files WHERE path=? AND desktop=? AND state IN ('prepared','failed','verified') ORDER BY updated DESC LIMIT 1",new String[]{path,desktop})){if(c.moveToFirst())j=new JSONObject(c.getString(0));}
        if(j==null)j=new JSONObject().put("id",uid()).put("token",uid());
        db.delete("files","path=? AND state IN ('prepared','failed','verified') AND id!=?",new String[]{path,j.getString("id")});
        clearStaleFailed();
        j.put("path",path).put("name",f.getName()).put("kind",kind(path)).put("state","prepared").put("desktop",desktop).put("desktopName",host().optString("name","保存原件的电脑")).put("size",size).put("modified",mtime).put("sha256",sha).put("epoch",p.getString("epoch")).put("error","");
        put(j);createThumbnail(j,f);return j;
    }
    private void createThumbnail(JSONObject j,File f){try{File target=thumbnail(j.getString("id"));if(target.isFile())return;Bitmap bitmap=null;if(j.getString("kind").equals("photos")){BitmapFactory.Options o=new BitmapFactory.Options();o.inJustDecodeBounds=true;BitmapFactory.decodeFile(f.getPath(),o);o.inSampleSize=1;while(Math.max(o.outWidth,o.outHeight)/o.inSampleSize>480)o.inSampleSize*=2;o.inJustDecodeBounds=false;bitmap=BitmapFactory.decodeFile(f.getPath(),o);}else if(j.getString("kind").equals("videos"))bitmap=ThumbnailUtils.createVideoThumbnail(f,new Size(320,320),null);if(bitmap!=null){try(FileOutputStream out=new FileOutputStream(target)){bitmap.compress(Bitmap.CompressFormat.JPEG,75,out);out.getFD().sync();}bitmap.recycle();}}catch(Exception ignored){/* The document shortcut remains valid without a thumbnail. */}}
    JSONObject commit(JSONObject q)throws Exception{synchronized(COMMIT_LOCK){
        long generation=policyGeneration;JSONObject j=get(q.getString("id"));String desktop=q.getString("desktopId");JSONObject p=active(desktop);
        if(!desktop.equals(j.getString("desktop"))||!q.getString("token").equals(j.getString("token"))||!q.getString("sha256").equals(j.getString("sha256"))||q.getLong("size")!=j.getLong("size"))throw new IOException("电脑校验回执与手机文件不一致");
        if(j.getString("state").equals("archived"))return j;
        if(!p.getString("epoch").equals(j.getString("epoch"))||!p.optBoolean(j.getString("kind")))throw new IOException("迁移设置已变化，保留手机原文件");
        File f=source(j.getString("path")),moved=new File(f.getParentFile(),".stepx-migrate-"+j.getString("id")),shortcut=new File(f.getParentFile(),f.getName()+".stepxlink");
        try{
            if(f.exists()&&moved.exists())throw new IOException("原位置已有另一个文件，请先恢复迁移记录");
            File current=moved.exists()?moved:f;
            if(current.exists()&&(current.length()!=j.getLong("size")||current.lastModified()!=j.getLong("modified")||!digest(current).equals(j.getString("sha256"))))throw new IOException("原文件已变化，不会清理手机文件");
            if(!current.exists()&&!j.getString("state").equals("releasing"))throw new IOException("原文件不存在，无法确认迁移");
            if(shortcut.exists()&&!read(shortcut).optString("id").equals(j.getString("id")))throw new IOException("同名快捷方式已存在，保留原文件");
            writeAtomic(shortcut,new JSONObject().put("type","stepx-storage-shortcut").put("version",1).put("id",j.getString("id")).put("desktopId",desktop).put("name",j.getString("name")));
            j.put("state","releasing");put(j); // durable journal precedes original deletion
            if(f.exists())Files.move(f.toPath(),moved.toPath());
            if(moved.exists()){
                if(moved.length()!=j.getLong("size")||!digest(moved).equals(j.getString("sha256")))throw new IOException("清理前再次校验失败，手机原文件保留");
                ExpansionGallery.publish(context,this,j);
                if((j.optString("kind").equals("photos")||j.optString("kind").equals("videos"))&&!j.optString("galleryError").isEmpty())throw new IOException("相册预览尚未就绪，保留手机原件："+j.optString("galleryError"));
                if(generation!=policyGeneration)throw new IOException("迁移已暂停，保留手机原件");
                j.put("cleanupPath",moved.getPath());put(j);return j;
            }
            j.put("state","archived").put("archivedAt",System.currentTimeMillis()).put("cleanupVerified",true).put("error","");put(j);
            android.media.MediaScannerConnection.scanFile(context,new String[]{f.getPath()},null,null);ExpansionGallery.publish(context,this,j);return j;
        }catch(Exception e){if(moved.exists()&&!f.exists())try{Files.move(moved.toPath(),f.toPath());}catch(Exception ignored){}if(f.exists())try{ExpansionGallery.remove(context,j);}catch(Exception ignored){}j.put("state","failed").put("error",e.getMessage());put(j);throw e;}
    }}
    // ADB's authorized file channel performs the final unlink. Vendor app recycle bins
    // intercept ordinary app deletion and do not actually release storage.
    JSONObject finalizeCleanup(JSONObject q)throws Exception{synchronized(COMMIT_LOCK){
        JSONObject j=get(q.getString("id"));if(!q.getString("desktopId").equals(j.getString("desktop"))||!q.getString("token").equals(j.getString("token")))throw new IOException("清理回执无效");
        if(j.optString("state").equals("archived")&&j.optBoolean("cleanupVerified"))return j;
        if(!j.optString("state").equals("releasing"))throw new IOException("文件尚未进入清理阶段");
        File original=source(j.getString("path")),staged=new File(original.getParentFile(),".stepx-migrate-"+j.getString("id"));
        if(staged.exists()||original.exists())throw new IOException("手机原件仍保留，尚未释放空间");
        j.put("state","archived").put("archivedAt",System.currentTimeMillis()).put("cleanupVerified",true).put("error","");put(j);android.media.MediaScannerConnection.scanFile(context,new String[]{original.getPath()},null,null);ExpansionGallery.publish(context,this,j);return j;
    }}
    private void recoverStaged()throws Exception {
        JSONArray pending=list("all","date",true,0,100,"releasing");for(int i=0;i<pending.length();i++){JSONObject j=pending.getJSONObject(i);File f=source(j.getString("path")),m=new File(f.getParentFile(),".stepx-migrate-"+j.getString("id"));
            if(m.exists()){if(f.exists())throw new IOException("迁移文件待恢复：原位置已有文件");Files.move(m.toPath(),f.toPath());ExpansionGallery.remove(context,j);j.put("state","prepared").put("error","已暂停，手机原件保留");}
            else if(!f.exists())j.put("state","archived").put("cleanupVerified",true).put("archivedAt",System.currentTimeMillis());
            else j.put("state","prepared");put(j);ExpansionGallery.publish(context,this,j);
        }
    }
    JSONObject fail(String id,String message)throws Exception{JSONObject j=get(id);if(!j.optString("state").equals("archived")&&!j.optString("state").equals("restored")&&!j.optString("state").equals("releasing")){j.put("state","failed").put("error",message);put(j);}return j;}
    JSONObject request(String id,String mode)throws Exception{
        if(!mode.equals("view")&&!mode.equals("restore"))throw new IOException("无效操作");JSONObject j=get(id);if(!j.optString("state").equals("archived")&&!j.optString("state").equals("restored"))throw new IOException("文件还没有完成迁移");
        if((mode.equals("view")||j.optString("state").equals("restored"))&&available(j)!=null)return new JSONObject().put("state","ready");
        JSONObject h=host();if(!online(h)||!h.optString("desktopId").equals(j.getString("desktop")))throw new IOException("原电脑离线，请连接保存此文件的电脑后重试");
        if(sharedRoot.getUsableSpace()<j.getLong("size")*(mode.equals("restore")?2:1)+RESERVE)throw new IOException("手机空间不足，请先清理缓存或在电脑查看");
        ContentValues v=new ContentValues();v.put("id",id);v.put("mode",mode);v.put("state","queued");v.put("error","");v.put("updated",System.currentTimeMillis());db.insertWithOnConflict("requests",null,v,SQLiteDatabase.CONFLICT_REPLACE);return new JSONObject().put("state","queued");
    }
    JSONObject requestState(String id)throws Exception{try(Cursor c=db.query("requests",new String[]{"mode","state","error"},"id=?",new String[]{id},null,null,null)){if(!c.moveToFirst())return new JSONObject();return new JSONObject().put("mode",c.getString(0)).put("state",c.getString(1)).put("error",c.getString(2));}}
    JSONArray requests(String desktop)throws Exception{JSONArray out=new JSONArray();try(Cursor c=db.rawQuery("SELECT r.id,r.mode FROM requests r JOIN files f ON r.id=f.id WHERE r.state='queued' AND f.desktop=? ORDER BY r.updated LIMIT 5",new String[]{desktop})){while(c.moveToNext()){JSONObject j=get(c.getString(0));out.put(j.put("requestMode",c.getString(1)).put("incoming",incoming(c.getString(0)).getPath()));}}return out;}
    void requestError(String id,String error){ContentValues v=new ContentValues();v.put("state","error");v.put("error",error);db.update("requests",v,"id=?",new String[]{id});}
    JSONObject deliver(JSONObject q)throws Exception{
        JSONObject j=get(q.getString("id"));String id=j.getString("id");if(!j.getString("desktop").equals(q.getString("desktopId")))throw new IOException("电脑不匹配");JSONObject r=requestState(id);if(!r.optString("state").equals("queued"))throw new IOException("文件请求已取消");
        File staged=incoming(id);if(!staged.isFile()||staged.length()!=j.getLong("size")||!digest(staged).equals(j.getString("sha256")))throw new IOException("取回文件校验失败");
        File target=cached(id);Files.move(staged.toPath(),target.toPath(),StandardCopyOption.REPLACE_EXISTING);
        if(r.optString("mode").equals("restore")){
            File original=source(j.getString("path"));if(!original.getParentFile().isDirectory()&&!original.getParentFile().mkdirs())throw new IOException("原文件夹不可用");
            boolean complete=false;File restored=original;if(!restored.createNewFile()){
                if(restored.length()==j.getLong("size")&&digest(restored).equals(j.getString("sha256"))){complete=true;j.put("state","restored").put("restoredPath",j.getString("path"));put(j);}
                else {int dot=original.getName().lastIndexOf('.');String n=original.getName();restored=new File(original.getParentFile(),(dot>0?n.substring(0,dot):n)+" (恢复-"+id.substring(0,6)+")"+(dot>0?n.substring(dot):""));if(!restored.createNewFile())throw new IOException("恢复文件已存在，不覆盖已有文件");}
            }
            if(!complete){
                try {
                try(InputStream in=new FileInputStream(target);FileOutputStream out=new FileOutputStream(restored)){byte[] buf=new byte[1024*1024];int n;while((n=in.read(buf))!=-1)out.write(buf,0,n);out.getFD().sync();}
                if(restored.length()!=j.getLong("size")||!digest(restored).equals(j.getString("sha256")))throw new IOException("恢复校验失败，电脑原件仍保留");
                restored.setLastModified(j.getLong("modified"));j.put("state","restored").put("restoredPath",sharedRoot.toPath().relativize(restored.toPath()).toString());put(j);
                } catch(Exception e) {Files.deleteIfExists(restored.toPath());throw e;}
            }
            File shortcut=new File(original.getParentFile(),original.getName()+".stepxlink");if(shortcut.isFile()&&read(shortcut).optString("id").equals(id))Files.delete(shortcut.toPath());
            android.media.MediaScannerConnection.scanFile(context,new String[]{restored.getPath()},null,null);try{ExpansionGallery.remove(context,j);}catch(Exception e){j.put("galleryError","原件已恢复，预览图暂未移除");put(j);}Files.deleteIfExists(target.toPath());
        }
        ContentValues v=new ContentValues();v.put("state","ready");v.put("error","");db.update("requests",v,"id=?",new String[]{id});trimCache(id);return j;
    }
    File available(JSONObject j)throws Exception{
        if(j.optString("state").equals("restored")){File f=source(j.optString("restoredPath",j.getString("path")));if(f.isFile()&&f.length()==j.getLong("size"))return f;}
        File f=cached(j.getString("id"));return f.isFile()&&f.length()==j.getLong("size")?f:null;
    }
    private void trimCache(String keep){try{File dir=cached(keep).getParentFile();File[] files=dir.listFiles();if(files==null)return;Arrays.sort(files,Comparator.comparingLong(File::lastModified));long total=0;for(File f:files)total+=f.length();for(File f:files)if(!f.getName().equals(keep)&&(total>512L*1024*1024||System.currentTimeMillis()-f.lastModified()>86400000L)){long size=f.length();if(f.delete())total-=size;}}catch(Exception ignored){}}
    void forget(String id)throws Exception {synchronized(COMMIT_LOCK){
        JSONObject j=get(id);String state=j.optString("state");File original=source(j.optString("restoredPath",j.getString("path")));
        if(!Arrays.asList("restored","prepared","failed").contains(state)||!original.isFile()||new File(original.getParentFile(),".stepx-migrate-"+id).exists())throw new IOException("请先恢复原件到手机，再移除扩容记录");
        // A pending operation can be dismissed only while its verified original is intact.
        if(!state.equals("restored")&&(original.length()!=j.getLong("size")||!digest(original).equals(j.getString("sha256"))))throw new IOException("原文件已变化，请保留记录并检查文件");
        File shortcut=new File(original.getParentFile(),original.getName()+".stepxlink");if(shortcut.isFile()&&read(shortcut).optString("id").equals(id))Files.delete(shortcut.toPath());
        ExpansionGallery.remove(context,j);j.put("state","forgotten");put(j);Files.deleteIfExists(cached(id).toPath());Files.deleteIfExists(thumbnail(id).toPath());
    }}
    void clearCache()throws Exception{File dir=cached("00000000000000000000000000000000").getParentFile();File[] files=dir.listFiles();if(files!=null)for(File f:files)if(id(f.getName()))Files.deleteIfExists(f.toPath());}
}
