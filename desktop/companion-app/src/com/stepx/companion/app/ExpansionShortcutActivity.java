package com.stepx.companion.app;
import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.os.Environment;
import android.widget.Toast;
import java.io.*;
import java.nio.file.Files;
import java.util.concurrent.*;
import org.json.JSONObject;
/** The shortcut contains an opaque journal ID, never a trusted filesystem path. */
public final class ExpansionShortcutActivity extends Activity {
 private static final ExecutorService SHORTCUT_EXECUTOR = new ThreadPoolExecutor(
     0, 1, 30L, TimeUnit.SECONDS, new ArrayBlockingQueue<Runnable>(4)
 );
 static boolean isTrustedShortcutAuthority(String authority) {
  return McpRoutePolicy.isTrustedShortcutAuthority(authority);
 }
 private byte[] readShortcutBytes(Uri uri) throws IOException {
  if (uri == null) throw new IOException("文件入口无效");
  String scheme = uri.getScheme();
  if ("file".equals(scheme)) {
   String rawPath = uri.getPath();
   if (rawPath == null || !rawPath.endsWith(".stepxlink") || rawPath.contains("..")) {
    throw new IOException("文件入口无效");
   }
   File target = new File(rawPath);
   if (Files.isSymbolicLink(target.toPath()) || !target.isFile()) {
    throw new IOException("文件入口无效");
   }
   File extRoot = Environment.getExternalStorageDirectory();
   if (extRoot != null) {
    String rootCanonical = extRoot.getCanonicalPath();
    String fileCanonical = target.getCanonicalPath();
    if (!fileCanonical.startsWith(rootCanonical + File.separator)) {
     throw new IOException("文件入口路径不受信任");
    }
   }
   try (InputStream in = new FileInputStream(target)) {
    return in.readNBytes(32769);
   }
  }
  if ("content".equals(scheme)) {
   if (!isTrustedShortcutAuthority(uri.getAuthority())) {
    throw new IOException("文件入口来源不受信任");
   }
   try (InputStream in = getContentResolver().openInputStream(uri)) {
    if (in == null) throw new IOException("无法读取入口");
    return in.readNBytes(32769);
   }
  }
  throw new IOException("文件入口协议不支持");
 }
 public void onCreate(Bundle b){
  super.onCreate(b);
  final Uri data = getIntent() == null ? null : getIntent().getData();
  try {
   SHORTCUT_EXECUTOR.execute(()->{
    try{
     byte[] bytes = readShortcutBytes(data);
     if(bytes.length>32768)throw new IOException("文件入口无效");
     JSONObject shortcut=new JSONObject(new String(bytes,java.nio.charset.StandardCharsets.UTF_8));
     if(!shortcut.optString("type").equals("stepx-storage-shortcut"))throw new IOException("不是存储扩容文件入口");
     JSONObject record=new ExpansionStore(this).get(shortcut.getString("id"));
     if(!record.getString("desktop").equals(shortcut.getString("desktopId")))throw new IOException("文件入口与原电脑不匹配");
     runOnUiThread(()->{startActivity(new Intent(this,ExpansionActivity.class).putExtra("fileId",record.optString("id")));finish();});
    }catch(Exception e){
     runOnUiThread(()->{Toast.makeText(this,e.getMessage(),Toast.LENGTH_LONG).show();finish();});
    }
   });
  } catch (RejectedExecutionException busy) {
   Toast.makeText(this,"正在打开上一个文件入口，请稍候",Toast.LENGTH_SHORT).show();
   finish();
  }
 }
}
