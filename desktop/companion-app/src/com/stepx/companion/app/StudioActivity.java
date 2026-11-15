package com.stepx.companion.app;

import android.app.*;
import android.content.*;
import android.content.pm.*;
import android.graphics.*;
import android.graphics.drawable.*;
import android.net.Uri;
import android.os.*;
import android.view.*;
import android.widget.*;
import org.json.*;
import java.util.concurrent.*;

/** The phone owns its library. Desktop disconnecting does not remove any entry. */
public final class StudioActivity extends Activity {
    private LinearLayout page;private StudioStore store;private boolean destroyed,resumedOnce;private int generation;private CompanionUi design;
    private final ExecutorService worker=Executors.newSingleThreadExecutor();
    private final int ink=CompanionUi.INK,muted=CompanionUi.MUTED;
    private int dp(int x){return Math.round(x*getResources().getDisplayMetrics().density);}
    private TextView text(LinearLayout parent,String s,int size,int color){TextView t=new TextView(this);t.setText(s);t.setTextSize(size);t.setTextColor(color);t.setPadding(0,dp(8),0,dp(8));parent.addView(t);return t;}
    private Button button(LinearLayout parent,String s,Runnable action){Button b=new Button(this);boolean primary=s.equals("打开作品");b.setAllCaps(false);b.setText(s);b.setTextSize(15);b.setTextColor(primary?Color.WHITE:ink);b.setMinHeight(dp(48));b.setPadding(dp(12),dp(8),dp(12),dp(8));b.setBackgroundTintList(null);GradientDrawable bg=new GradientDrawable();bg.setColor(primary?ink:Color.WHITE);bg.setCornerRadius(dp(10));if(!primary)bg.setStroke(dp(1),CompanionUi.LINE);b.setBackground(bg);LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.topMargin=dp(8);lp.rightMargin=dp(4);parent.addView(b,lp);design.styleButton(b,primary);b.setOnClickListener(v->action.run());return b;}
    static String sanitizeOp(String op){return McpRoutePolicy.sanitizeStudioOp(op);}
    static String sanitizeId(String id){return McpRoutePolicy.sanitizeStudioId(id);}
    @Override public void onCreate(Bundle state){super.onCreate(state);design=new CompanionUi(this);store=new StudioStore(this);build();Intent in=getIntent();String op=state==null&&in!=null?sanitizeOp(in.getStringExtra("op")):null;String id=in!=null?sanitizeId(in.getStringExtra("id")):null;load(op,id);}
    @Override protected void onNewIntent(Intent intent){super.onNewIntent(intent);setIntent(intent);build();load(intent==null?null:sanitizeOp(intent.getStringExtra("op")),intent==null?null:sanitizeId(intent.getStringExtra("id")));}
    @Override protected void onResume(){super.onResume();if(resumedOnce){build();load(null,null);}resumedOnce=true;}
    private void build(){generation++;
        ScrollView scroll=new ScrollView(this);page=new LinearLayout(this);page.setOrientation(LinearLayout.VERTICAL);page.setPadding(dp(16),dp(4),dp(16),dp(24));page.setBackgroundColor(CompanionUi.BG);scroll.addView(page);setContentView(scroll);
        scroll.setOnApplyWindowInsetsListener((v,i)->{Insets b=i.getInsets(WindowInsets.Type.systemBars());v.setPadding(b.left,b.top,b.right,b.bottom);return i;});scroll.requestApplyInsets();
        getWindow().getInsetsController().setSystemBarsAppearance(WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS|WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS|WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS);
        design.back(page,"Studio 作品",this::finish);
        text(page,"电脑推送到手机的应用、游戏与作品。",13,muted);
        LinearLayout destinations=design.card(page,16);
        design.row(destinations,UiGlyph.STUDIO,"官网项目与草稿","继续编辑官网项目",()->startActivity(new Intent(this,StudioWebActivity.class)));
        design.row(destinations,UiGlyph.LINK,"账号与同步","登录官网账号并同步作品",()->startActivity(new Intent(this,StudioAccountActivity.class)));
        design.row(destinations,UiGlyph.TASK,"Amoo 我的作品","确认技能与智能体安装",()->{try{startActivity(new Intent(Intent.ACTION_VIEW,Uri.parse("stepos://amoo/testspace")));}catch(Exception e){message("此手机无法打开 Amoo 我的作品");}});
    }
    private void load(String op,String id){final int requestGeneration=generation;worker.execute(()->{try{JSONArray all=store.list(),local=StudioLocal.list(this);runOnUiThread(()->{if(destroyed||requestGeneration!=generation)return;renderLocal(local);render(all);if(op!=null&&id!=null){try{JSONObject w=null;for(int i=0;i<all.length();i++)if(all.getJSONObject(i).getString("id").equals(id))w=all.getJSONObject(i);if(w==null)throw new Exception("作品不在这台手机中，请重新发送");if(op.equals("open"))open(w);else if(op.equals("pin"))pin(w);}catch(Exception e){message(e.getMessage());}}});}catch(Exception e){runOnUiThread(()->message("读取失败："+e.getMessage()));}});}
    private void renderLocal(JSONArray local){
        text(page,"本地作品 · "+local.length(),18,ink).setTypeface(CompanionUi.MEDIUM);if(local.length()==0){LinearLayout empty=design.card(page,10);design.row(empty,UiGlyph.FOLDER,"还没有本地作品","从电脑 Studio 推送应用或网页游戏，保存在手机后可独立运行。",null);return;}
        for(int i=0;i<local.length();i++)try{JSONObject w=local.getJSONObject(i);String id=w.getString("id");LinearLayout card=new LinearLayout(this);card.setOrientation(LinearLayout.VERTICAL);card.setPadding(dp(16),dp(12),dp(16),dp(12));GradientDrawable bg=new GradientDrawable();bg.setColor(Color.WHITE);bg.setCornerRadius(dp(12));bg.setStroke(dp(1),CompanionUi.LINE);card.setBackground(bg);LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.topMargin=dp(12);page.addView(card,lp);text(card,w.getString("name"),17,ink).setTypeface(CompanionUi.MEDIUM);text(card,"已保存在手机 · 可离线使用 · "+Math.max(1,w.getInt("size")/1024)+" KB",13,muted);button(card,"打开作品",()->startActivity(new Intent(this,StudioLocalActivity.class).putExtra("id",id)));button(card,"移除本地作品",()->new AlertDialog.Builder(this).setTitle("移除此手机上的作品？").setMessage("电脑文件和官网项目不受影响。").setNegativeButton("取消",null).setPositiveButton("移除",(d,v)->worker.execute(()->{try{StudioLocal.delete(this,id);runOnUiThread(()->{if(!destroyed){build();load(null,null);}});}catch(Exception e){runOnUiThread(()->message("移除失败"));}})).show());}catch(Exception e){message("本地作品读取失败");}
    }
    private void render(JSONArray all){
        text(page,"官网体验链接",18,ink).setTypeface(CompanionUi.MEDIUM);
        if(all.length()==0)text(page,"在电脑 Studio 发送官网作品后，链接会显示在这里。",14,muted);
        for(int i=0;i<all.length();i++){try{
            JSONObject w=all.getJSONObject(i);LinearLayout card=new LinearLayout(this);card.setOrientation(LinearLayout.VERTICAL);card.setPadding(dp(16),dp(12),dp(16),dp(12));GradientDrawable bg=new GradientDrawable();bg.setColor(Color.WHITE);bg.setCornerRadius(dp(12));bg.setStroke(dp(1),CompanionUi.LINE);card.setBackground(bg);LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.topMargin=dp(18);page.addView(card,lp);
            text(card,w.getString("name"),17,ink).setTypeface(CompanionUi.MEDIUM);text(card,w.getString("category")+" · Studio 体验链接",13,muted);
            button(card,"打开作品",()->open(w));LinearLayout row=new LinearLayout(this);card.addView(row);
            for(int action=0;action<2;action++){final int which=action;LinearLayout cell=new LinearLayout(this);cell.setOrientation(LinearLayout.VERTICAL);row.addView(cell,new LinearLayout.LayoutParams(0,-2,1));button(cell,action==0?"添加桌面图标":"分享链接",()->{if(which==0)pin(w);else share(w);});}
        }catch(Exception e){message(e.getMessage());}}
    }
    private void open(JSONObject w){try{StudioStore.validate(w);startActivity(new Intent(Intent.ACTION_VIEW,Uri.parse(w.getString("url"))).addCategory(Intent.CATEGORY_BROWSABLE));getSharedPreferences("studio-state",0).edit().putString("lastOpen",w.getString("id")).putLong("lastOpenAt",System.currentTimeMillis()).putString("lastError","").apply();}catch(Exception e){getSharedPreferences("studio-state",0).edit().putString("lastError",String.valueOf(e.getMessage())).apply();message("无法打开："+e.getMessage());}}
    private void share(JSONObject w){try{StudioStore.validate(w);Intent i=new Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT,w.getString("name")+"\n"+w.getString("url"));startActivity(Intent.createChooser(i,"分享 Studio 作品"));}catch(Exception e){message(e.getMessage());}}
    private void pin(JSONObject w){try{
        StudioStore.validate(w);ShortcutManager m=getSystemService(ShortcutManager.class);if(m==null||!m.isRequestPinShortcutSupported()){message("当前桌面不支持添加图标，作品仍可从伴侣打开");return;}
        Intent launch=new Intent(this,StudioActivity.class).setAction(Intent.ACTION_VIEW).putExtra("op","open").putExtra("id",w.getString("id"));
        ShortcutInfo s=new ShortcutInfo.Builder(this,w.getString("id")).setShortLabel(w.getString("name")).setIcon(Icon.createWithResource(this,getApplicationInfo().icon)).setIntent(launch).build();
        if(m.requestPinShortcut(s,null))message("已请求添加，请在系统提示中确认；作品库仍保留此入口");else message("桌面未接受添加请求");
    }catch(Exception e){message("添加失败："+e.getMessage());}}
    private void message(String s){if(!destroyed)Toast.makeText(this,s,Toast.LENGTH_LONG).show();}
    @Override public void onDestroy(){destroyed=true;worker.shutdown();super.onDestroy();}
}
