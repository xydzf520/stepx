package com.stepx.companion.app;

import android.content.ContentValues;
import android.database.Cursor;
import android.database.sqlite.SQLiteDatabase;
import org.json.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.util.*;

/** One transaction for the message window, notifications, tombstones and source cursor.
 * Original JSON remains untouched. Database errors never fall back to stale JSON. */
final class PushDatabase {
    private final SQLiteDatabase db;
    private final Map<String,Map<String,StoredRow>> rowCache=new HashMap<>();
    private static final class StoredRow {final String body;final int position;StoredRow(String b,int p){body=b;position=p;}}
    private static JSONObject without(JSONObject source,String key)throws JSONException {
        JSONObject result=new JSONObject();Iterator<String> keys=source.keys();while(keys.hasNext()){String k=keys.next();if(!k.equals(key))result.put(k,source.get(k));}return result;
    }
    PushDatabase(File directory) throws Exception {
        if (directory==null) throw new IOException("消息存储目录不可用");
        if (!directory.isDirectory() && !directory.mkdirs()) throw new IOException("无法创建消息数据库目录");
        File path=new File(directory,"push.sqlite3");
        if (Files.isSymbolicLink(path.toPath())) throw new IOException("消息数据库不能是链接");
        db=SQLiteDatabase.openOrCreateDatabase(path.getPath(),null,new android.database.DatabaseErrorHandler(){
            public void onCorruption(SQLiteDatabase corrupted){throw new android.database.sqlite.SQLiteDatabaseCorruptException("消息数据库损坏，保留原文件等待恢复");}
        });
        db.enableWriteAheadLogging();
        db.execSQL("PRAGMA synchronous=FULL");
        try(Cursor timeout=db.rawQuery("PRAGMA busy_timeout=3000",null)){timeout.moveToFirst();}
        if (db.getVersion()>1) {db.close();throw new IOException("消息数据库需要更新版客户端");}
        if (db.getVersion()==0) {
            db.beginTransaction();
            try {
                db.execSQL("CREATE TABLE meta (name TEXT PRIMARY KEY, body TEXT NOT NULL)");
                db.execSQL("CREATE TABLE records (section TEXT NOT NULL,id TEXT NOT NULL,position INTEGER NOT NULL,body TEXT NOT NULL,PRIMARY KEY(section,id))");
                db.execSQL("CREATE INDEX records_page ON records(section,position)");
                db.setVersion(1);db.setTransactionSuccessful();
            } finally {db.endTransaction();}
        }
        try(Cursor c=db.rawQuery("PRAGMA quick_check(1)",null)) {
            if(!c.moveToFirst() || !"ok".equals(c.getString(0))) {db.close();throw new IOException("消息数据库完整性检查失败，停止覆盖");}
        }
    }
    synchronized JSONObject[] load(File legacy) throws Exception {
        if (getMeta("initialized")!=null) return new JSONObject[]{read("threads"),read("feed")};
        JSONObject threads=legacy(legacy,"im_threads.json","threads"),feed=legacy(legacy,"push_feed.json","notifications");
        // Both legacy documents must parse before starting the migration transaction.
        save(threads,feed,true);
        return new JSONObject[]{threads,feed};
    }
    private static JSONObject legacy(File dir,String name,String field) throws Exception {
        File file=new File(dir,name);
        if(!file.exists()) return new JSONObject().put(field,new JSONArray());
        if(!file.isFile() || Files.isSymbolicLink(file.toPath()) || file.length()>64L*1024*1024) throw new IOException("旧消息文件无效，保留原件");
        JSONObject root=new JSONObject(new String(Files.readAllBytes(file.toPath()),StandardCharsets.UTF_8));
        if(!(root.opt(field) instanceof JSONArray)) throw new IOException("旧消息结构无效，保留原件");
        return root;
    }
    private String getMeta(String key) {
        try(Cursor c=db.rawQuery("SELECT body FROM meta WHERE name=?",new String[]{key})) {return c.moveToFirst()?c.getString(0):null;}
    }
    private void putMeta(String key,String body) {
        if(body.equals(getMeta(key))) return;
        ContentValues v=new ContentValues();v.put("name",key);v.put("body",body);
        if(db.insertWithOnConflict("meta",null,v,SQLiteDatabase.CONFLICT_REPLACE)<0)throw new android.database.sqlite.SQLiteException("消息游标写入失败");
    }
    private JSONObject read(String name) throws Exception {
        String raw=getMeta(name);JSONObject root=raw==null?new JSONObject():new JSONObject(raw);
        String field=name.equals("threads")?"threads":"notifications";
        JSONArray rows=rows(name);
        if(name.equals("threads")) for(int i=0;i<rows.length();i++) {
            JSONObject t=rows.getJSONObject(i);t.put("messages",rows("messages:"+t.getString("threadId")));
        }
        return root.put(field,rows);
    }
    private JSONArray rows(String section) throws Exception {
        JSONArray rows=new JSONArray();
        try(Cursor c=db.rawQuery("SELECT body FROM records WHERE section=? ORDER BY position,id",new String[]{section})) {while(c.moveToNext()) rows.put(new JSONObject(c.getString(0)));}
        return rows;
    }
    synchronized void save(JSONObject threads,JSONObject feed,boolean importing) throws Exception {
        db.beginTransaction();
        try {
            store("threads","threads",threads);store("feed","notifications",feed);
            if(importing) {
                if(!equivalent(threads,read("threads")) || !equivalent(feed,read("feed"))) throw new IOException("消息迁移校验失败，保留原件");
                putMeta("legacyDigest",digest(threads.toString()+"\n"+feed.toString()));
                putMeta("initialized","1");
            }
            db.setTransactionSuccessful();
        } catch(Exception error){rowCache.clear();throw error;} finally {try{db.endTransaction();}catch(Exception error){rowCache.clear();throw error;}}
    }
    private void store(String name,String field,JSONObject source) throws Exception {
        JSONObject head=without(source,field);JSONArray values=source.getJSONArray(field);
        putMeta(name,head.toString());
        Set<String> sections=new HashSet<>();
        JSONArray records=new JSONArray();
        for(int i=0;i<values.length();i++) {
            JSONObject v=values.getJSONObject(i);
            if(name.equals("threads")) {
                String section="messages:"+v.getString("threadId");sections.add(section);
                JSONArray messages=v.optJSONArray("messages");if(messages==null) messages=new JSONArray();
                storeRows(section,messages,"id");v=without(v,"messages");
            }
            records.put(v);
        }
        storeRows(name,records,name.equals("threads")?"threadId":"seqId");
        if(name.equals("threads")) {
            List<String> obsolete=new ArrayList<>();
            try(Cursor c=db.rawQuery("SELECT DISTINCT section FROM records WHERE section LIKE 'messages:%'",null)) {while(c.moveToNext())if(!sections.contains(c.getString(0)))obsolete.add(c.getString(0));}
            for(String section:obsolete){db.delete("records","section=?",new String[]{section});rowCache.remove(section);}
        }
    }
    private void storeRows(String section,JSONArray values,String idField) throws Exception {
        Map<String,StoredRow> cached=rowCache.get(section);
        if(cached==null){cached=new HashMap<>();try(Cursor c=db.rawQuery("SELECT id,body,position FROM records WHERE section=?",new String[]{section})) {while(c.moveToNext())cached.put(c.getString(0),new StoredRow(c.getString(1),c.getInt(2)));}}
        Map<String,StoredRow> old=new HashMap<>(cached),next=new HashMap<>();
        Set<String> ids=new HashSet<>();
        for(int i=0;i<values.length();i++) {
            JSONObject value=values.getJSONObject(i);String id=value.optString(idField,"#"+i);String body=value.toString();
            if(!ids.add(id)) {id=id+"#"+i; if(!ids.add(id))throw new IOException("重复消息标识冲突");}
            StoredRow previous=old.remove(id);next.put(id,new StoredRow(body,i));
            if(previous!=null && body.equals(previous.body) && previous.position==i)continue;
            ContentValues v=new ContentValues();v.put("section",section);v.put("id",id);v.put("position",i);v.put("body",body);
            if(db.insertWithOnConflict("records",null,v,SQLiteDatabase.CONFLICT_REPLACE)<0)throw new IOException("消息写入失败");
        }
        for(String id:old.keySet())db.delete("records","section=? AND id=?",new String[]{section,id});
        rowCache.put(section,next);
    }
    static boolean equivalent(Object a,Object b) throws Exception {
        if(a instanceof JSONObject && b instanceof JSONObject) {
            JSONObject x=(JSONObject)a,y=(JSONObject)b;if(x.length()!=y.length())return false;
            Iterator<String> keys=x.keys();while(keys.hasNext()){String key=keys.next();if(!y.has(key)||!equivalent(x.get(key),y.get(key)))return false;}return true;
        }
        if(a instanceof JSONArray && b instanceof JSONArray) {
            JSONArray x=(JSONArray)a,y=(JSONArray)b;if(x.length()!=y.length())return false;
            for(int i=0;i<x.length();i++)if(!equivalent(x.get(i),y.get(i)))return false;return true;
        }
        return String.valueOf(a).equals(String.valueOf(b));
    }
    private static String digest(String s) throws Exception {byte[] bytes=MessageDigest.getInstance("SHA-256").digest(s.getBytes(StandardCharsets.UTF_8));StringBuilder out=new StringBuilder();for(byte b:bytes)out.append(String.format("%02x",b&255));return out.toString();}
    synchronized void close(){db.close();}
}
