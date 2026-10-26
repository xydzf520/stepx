package com.stepx.companion.app;
import android.provider.*;
import android.provider.DocumentsContract.*;
import android.database.*;
import android.os.*;
import android.content.res.AssetFileDescriptor;
import android.graphics.Point;
import org.json.*;
import java.io.*;
/** Read-only file access for the companion and Android's system file picker. */
public final class ExpansionDocumentsProvider extends DocumentsProvider {
 static final String[] DOC={Document.COLUMN_DOCUMENT_ID,Document.COLUMN_DISPLAY_NAME,Document.COLUMN_MIME_TYPE,Document.COLUMN_SIZE,Document.COLUMN_LAST_MODIFIED,Document.COLUMN_FLAGS};
 public boolean onCreate(){return true;}
 public Cursor queryRoots(String[] projection){MatrixCursor c=new MatrixCursor(projection!=null?projection:new String[]{Root.COLUMN_ROOT_ID,Root.COLUMN_DOCUMENT_ID,Root.COLUMN_TITLE,Root.COLUMN_FLAGS,Root.COLUMN_MIME_TYPES});c.newRow().add(Root.COLUMN_ROOT_ID,"expansion").add(Root.COLUMN_DOCUMENT_ID,"root").add(Root.COLUMN_TITLE,"StepX · 电脑扩容文件").add(Root.COLUMN_FLAGS,Root.FLAG_SUPPORTS_RECENTS).add(Root.COLUMN_MIME_TYPES,"*/*");return c;}
 private void row(MatrixCursor c,ExpansionStore store,JSONObject j)throws Exception{String id=j.getString("id");c.newRow().add(Document.COLUMN_DOCUMENT_ID,id).add(Document.COLUMN_DISPLAY_NAME,j.getString("name")).add(Document.COLUMN_MIME_TYPE,TransferStore.mime(j.getString("name"))).add(Document.COLUMN_SIZE,j.getLong("size")).add(Document.COLUMN_LAST_MODIFIED,j.getLong("modified")).add(Document.COLUMN_FLAGS,store.thumbnail(id).isFile()?Document.FLAG_SUPPORTS_THUMBNAIL:0);}
 public Cursor queryDocument(String id,String[] projection)throws FileNotFoundException{MatrixCursor c=new MatrixCursor(projection==null?DOC:projection);try{if(id.equals("root"))c.newRow().add(Document.COLUMN_DOCUMENT_ID,"root").add(Document.COLUMN_DISPLAY_NAME,"电脑扩容文件").add(Document.COLUMN_MIME_TYPE,Document.MIME_TYPE_DIR).add(Document.COLUMN_FLAGS,0);else{ExpansionStore store=new ExpansionStore(getContext());row(c,store,store.get(id));}return c;}catch(Exception e){throw new FileNotFoundException(e.getMessage());}}
 public Cursor queryChildDocuments(String parent,String[] projection,String sort)throws FileNotFoundException{if(!parent.equals("root"))throw new FileNotFoundException("文件夹不存在");MatrixCursor c=new MatrixCursor(projection==null?DOC:projection);try{ExpansionStore store=new ExpansionStore(getContext());int offset=0;while(true){JSONArray a=store.list("all","date",true,offset,100,"archived");for(int i=0;i<a.length();i++)row(c,store,a.getJSONObject(i));if(a.length()<100)break;offset+=100;}c.setNotificationUri(getContext().getContentResolver(),DocumentsContract.buildChildDocumentsUri(ExpansionStore.DOCS,"root"));return c;}catch(Exception e){throw new FileNotFoundException(e.getMessage());}}
 public Cursor queryRecentDocuments(String root,String[] projection)throws FileNotFoundException{MatrixCursor c=new MatrixCursor(projection==null?DOC:projection);try{ExpansionStore store=new ExpansionStore(getContext());JSONArray a=store.list("all","date",true,0,30,"archived");for(int i=0;i<a.length();i++)row(c,store,a.getJSONObject(i));return c;}catch(Exception e){throw new FileNotFoundException(e.getMessage());}}
 public ParcelFileDescriptor openDocument(String id,String mode,CancellationSignal cancel)throws FileNotFoundException{
  if(!mode.equals("r"))throw new FileNotFoundException("扩容文件只读，请先恢复到手机再编辑");try{ExpansionStore s=new ExpansionStore(getContext());JSONObject j=s.get(id);File file=s.available(j);if(file==null){s.request(id,"view");long end=android.os.SystemClock.elapsedRealtime()+300000;while(file==null&&android.os.SystemClock.elapsedRealtime()<end){if(cancel!=null)cancel.throwIfCanceled();JSONObject state=s.requestState(id);if(state.optString("state").equals("error"))throw new IOException(state.optString("error"));if(!s.online(s.host()))throw new IOException("电脑已断开，请重新连接后打开");Thread.sleep(400);file=s.available(s.get(id));}if(file==null)throw new IOException("文件仍在取回，请稍后重试");}return ParcelFileDescriptor.open(file,ParcelFileDescriptor.MODE_READ_ONLY);}catch(Exception e){throw new FileNotFoundException(e.getMessage());}
 }
 public AssetFileDescriptor openDocumentThumbnail(String id,Point hint,CancellationSignal cancel)throws FileNotFoundException{try{return new AssetFileDescriptor(ParcelFileDescriptor.open(new ExpansionStore(getContext()).thumbnail(id),ParcelFileDescriptor.MODE_READ_ONLY),0,AssetFileDescriptor.UNKNOWN_LENGTH);}catch(Exception e){throw new FileNotFoundException(e.getMessage());}}
}
