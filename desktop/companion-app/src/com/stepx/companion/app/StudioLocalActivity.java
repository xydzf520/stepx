package com.stepx.companion.app;

import android.app.*;
import android.content.*;
import android.graphics.Color;
import android.os.*;
import android.webkit.*;
import android.view.*;
import android.widget.*;
import java.io.*;
import java.util.*;

/** Offline local app surface: no JS/native bridge, external resources, or file access. */
public final class StudioLocalActivity extends Activity {
    private CompanionUi design;private WebView web;private String id;private String workName="";private byte[] html;private TextView status;private LinearLayout page;
    @Override public void onCreate(Bundle state){super.onCreate(state);design=new CompanionUi(this);loadWork(getIntent());}
    @Override protected void onNewIntent(Intent intent){super.onNewIntent(intent);setIntent(intent);loadWork(intent);}
    private void loadWork(Intent intent){
        id=intent!=null?intent.getStringExtra("id"):null;String token=intent!=null?intent.getStringExtra("openToken"):null;if(token==null)token=UUID.randomUUID().toString();getSharedPreferences("studio-local-state",0).edit().putString("lastOpen",id).putString("openToken",token).putLong("readyAt",0).putString("title","").putInt("errorCount",0).putString("lastError","").apply();
        try{html=StudioLocal.html(this,id);workName=StudioLocal.item(this,id).getString("name");
            if(web==null){
                page=new LinearLayout(this);page.setOrientation(LinearLayout.VERTICAL);page.setBackgroundColor(Color.WHITE);
                LinearLayout bar=new LinearLayout(this);bar.setGravity(Gravity.CENTER_VERTICAL);bar.setPadding(dp(8),0,dp(12),0);page.addView(bar,new LinearLayout.LayoutParams(-1,dp(52)));
                Button back=new Button(this);back.setText("作品");design.styleButton(back,false);UiGlyph.attach(back,UiGlyph.BACK,android.graphics.Color.rgb(25,25,29),dp(18));back.setAllCaps(false);back.setOnClickListener(v->{startActivity(new Intent(this,StudioActivity.class).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP));finish();});bar.addView(back,new LinearLayout.LayoutParams(dp(88),-1));
                status=new TextView(this);status.setPadding(dp(10),0,dp(8),0);status.setTextColor(Color.rgb(25,25,29));bar.addView(status,new LinearLayout.LayoutParams(0,-2,1));
                Button reload=new Button(this);reload.setText("刷新");design.styleButton(reload,false);reload.setOnClickListener(v->{if(web!=null)web.reload();else loadWork(getIntent());});bar.addView(reload,new LinearLayout.LayoutParams(dp(72),-1));
                web=new WebView(this);page.addView(web,new LinearLayout.LayoutParams(-1,0,1));setContentView(page);
                page.setOnApplyWindowInsetsListener((v,i)->{android.graphics.Insets b=i.getInsets(WindowInsets.Type.systemBars());v.setPadding(b.left,b.top,b.right,b.bottom);return i;});design.bars(page);
                WebSettings s=web.getSettings();s.setJavaScriptEnabled(true);s.setDomStorageEnabled(true);s.setAllowFileAccess(false);s.setAllowContentAccess(false);s.setAllowFileAccessFromFileURLs(false);s.setAllowUniversalAccessFromFileURLs(false);s.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);s.setSupportMultipleWindows(false);s.setMediaPlaybackRequiresUserGesture(true);s.setCacheMode(WebSettings.LOAD_NO_CACHE);CookieManager.getInstance().setAcceptThirdPartyCookies(web,false);
                web.setWebViewClient(new WebViewClient(){
                    @Override public boolean shouldOverrideUrlLoading(WebView v,WebResourceRequest r){return !StudioLocal.isDocument(id,r.getUrl().toString());}
                    @Override public WebResourceResponse shouldInterceptRequest(WebView v,WebResourceRequest r){
                        if(StudioLocal.isDocument(id,r.getUrl().toString())&&r.isForMainFrame()&&"GET".equals(r.getMethod())){
                            HashMap<String,String> headers=new HashMap<>();headers.put("Content-Security-Policy","default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; worker-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'");headers.put("Cache-Control","no-store");headers.put("X-Content-Type-Options","nosniff");
                            return new WebResourceResponse("text/html","UTF-8",200,"OK",headers,new ByteArrayInputStream(html));
                        }
                        return new WebResourceResponse("text/plain","UTF-8",403,"Blocked",Collections.emptyMap(),new ByteArrayInputStream(new byte[0]));
                    }
                    @Override public void onPageFinished(WebView v,String url){if(StudioLocal.isDocument(id,url)){getSharedPreferences("studio-local-state",0).edit().putLong("readyAt",System.currentTimeMillis()).putString("title",v.getTitle()==null?workName:v.getTitle()).apply();status.setText(workName+" · 已保存到手机");}}
                    @Override public void onReceivedError(WebView v,WebResourceRequest r,WebResourceError e){if(r.isForMainFrame())error("作品页面载入失败");}
                    @Override public boolean onRenderProcessGone(WebView v,RenderProcessGoneDetail d){error("作品页面已停止，请返回后重新打开");web=null;v.destroy();return true;}
                });
                web.setWebChromeClient(new WebChromeClient(){@Override public boolean onConsoleMessage(ConsoleMessage m){if(m.messageLevel()==ConsoleMessage.MessageLevel.ERROR)error("作品运行出现错误，请在电脑验证后重新发送");return true;}@Override public void onPermissionRequest(PermissionRequest r){r.deny();}@Override public void onGeolocationPermissionsShowPrompt(String origin,GeolocationPermissions.Callback callback){callback.invoke(origin,false,false);}});
            }else{
                web.stopLoading();
            }
            status.setText(workName+" · 离线作品");
            getSharedPreferences("studio-local-state",0).edit().putString("lastOpen",id).putLong("readyAt",0).putString("title",workName).putInt("errorCount",0).putString("lastError","").apply();
            web.loadUrl(StudioLocal.origin(id)+"/index.html");
        }catch(Exception e){error("作品文件无法打开，请从电脑重新发送");if(web!=null){web.stopLoading();web.destroy();web=null;}TextView error=new TextView(this);error.setText("无法打开作品："+e.getMessage());error.setPadding(dp(24),dp(48),dp(24),dp(24));setContentView(error);}
    }
    private int dp(int x){return Math.round(x*getResources().getDisplayMetrics().density);}
    private void error(String text){android.content.SharedPreferences p=getSharedPreferences("studio-local-state",0);p.edit().putString("lastError",text).putInt("errorCount",p.getInt("errorCount",0)+1).apply();if(status!=null)status.setText(text);}
    @Override protected void onPause(){if(web!=null)web.onPause();super.onPause();}
    @Override protected void onResume(){super.onResume();if(web!=null)web.onResume();}
    @Override public void onDestroy(){if(web!=null){web.stopLoading();web.destroy();web=null;}super.onDestroy();}
}
