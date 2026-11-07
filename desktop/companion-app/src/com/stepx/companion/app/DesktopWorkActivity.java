package com.stepx.companion.app;

import android.app.*;
import android.content.*;
import android.graphics.Color;
import android.net.Uri;
import android.os.*;
import android.view.*;
import android.widget.*;
import org.json.*;
import java.util.*;
import java.util.concurrent.*;

/** Explicit source selection and confirmation; phone and MCP share the same durable task store. */
public final class DesktopWorkActivity extends Activity {
    private final ExecutorService worker=Executors.newSingleThreadExecutor();
    private final Handler ui=new Handler(Looper.getMainLooper());
    private final ArrayList<Uri> selected=new ArrayList<>();
    private EditText goal;
    private TextView selection,status,connection;
    private Button start,pick,prepare;
    private LinearLayout history;
    private boolean resumed,destroyed,loading,confirming;
    private int generation;
    private CompanionUi design;
    private String lastTasks="",handledSubmission="";
    private final Runnable refresh=this::load;

    // Keep only the current submission across Activity recreation. It holds no Activity,
    // and serializes confirmation even when the user rotates or reopens the workbench.
    private static final class Submission {
        final String id=UUID.randomUUID().toString(),prompt;
        final ArrayList<Uri> sources;
        final boolean prepareOnly;
        volatile boolean complete,success;
        volatile String message="正在准备并校验资料…";
        Submission(String prompt,ArrayList<Uri> sources,boolean prepareOnly){this.prompt=prompt;this.sources=sources;this.prepareOnly=prepareOnly;}
    }
    private static volatile Submission currentSubmission;
    private static synchronized Submission beginSubmission(String prompt,ArrayList<Uri> sources,boolean prepareOnly){
        if(currentSubmission!=null&&!currentSubmission.complete)return null;
        currentSubmission=new Submission(prompt,sources,prepareOnly);
        return currentSubmission;
    }
    private boolean submitting(){Submission value=currentSubmission;return value!=null&&!value.complete;}
    private int dp(int n){return Math.round(n*getResources().getDisplayMetrics().density);}
    private TextView text(LinearLayout parent,String value,int size){TextView t=new TextView(this);t.setText(value);t.setTextSize(size);t.setTextColor(CompanionUi.INK);t.setLineSpacing(dp(3),1);t.setPadding(0,dp(8),0,dp(8));parent.addView(t);return t;}
    private Button button(LinearLayout parent,String label,boolean primary,Runnable action){
        Button b=new Button(this);b.setText(label);b.setTextSize(14);b.setAllCaps(false);b.setBackgroundTintList(null);b.setTextColor(primary?Color.WHITE:CompanionUi.INK);b.setBackground(design.touch(primary?CompanionUi.INK:Color.WHITE,CompanionUi.LINE,12));b.setMinHeight(dp(48));b.setPadding(dp(12),dp(10),dp(12),dp(10));
        LinearLayout.LayoutParams p=new LinearLayout.LayoutParams(-1,-2);p.topMargin=dp(10);parent.addView(b,p);b.setOnClickListener(v->action.run());return b;
    }
    @Override public void onCreate(Bundle state){
        super.onCreate(state);design=new CompanionUi(this);getWindow().setStatusBarColor(CompanionUi.BG);getWindow().setNavigationBarColor(Color.WHITE);
        ScrollView scroll=new ScrollView(this);scroll.setFillViewport(true);scroll.setOnApplyWindowInsetsListener((v,insets)->{android.graphics.Insets bars=insets.getInsets(WindowInsets.Type.systemBars()|WindowInsets.Type.ime());v.setPadding(bars.left,bars.top,bars.right,bars.bottom);return insets;});
        LinearLayout page=new LinearLayout(this);page.setOrientation(LinearLayout.VERTICAL);page.setPadding(dp(22),dp(14),dp(22),dp(30));page.setBackgroundColor(CompanionUi.BG);scroll.addView(page);setContentView(scroll);
        button(page,"‹ 返回",false,this::finish);text(page,"任务与成果",25).setTypeface(null,1);
        text(page,"手机选资料，电脑接着做。任务记录与已送达成果保存在手机，离线也能查看。",14);connection=text(page,"正在检查电脑…",13);
        text(page,"1 · 选择资料",16).setTypeface(null,1);selection=text(page,"还未选择文件",14);
        pick=button(page,"选择照片、表格或文档",false,()->{
            if(submitting())return;
            Intent i=new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*").putExtra(Intent.EXTRA_ALLOW_MULTIPLE,true);startActivityForResult(i,41);
        });
        text(page,"2 · 告诉 Amoo 要做什么",16).setTypeface(null,1);
        goal=new EditText(this);goal.setTextSize(15);goal.setHint("例如：统计销售额，生成一份分析报告和汇总表");goal.setMinLines(3);goal.setGravity(Gravity.TOP);goal.setPadding(dp(14),dp(14),dp(14),dp(14));goal.setBackground(design.touch(Color.WHITE,CompanionUi.LINE,12));goal.setInputType(android.text.InputType.TYPE_CLASS_TEXT|android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE);page.addView(goal,new LinearLayout.LayoutParams(-1,-2));
        text(page,"3 · 确认后开始",16).setTypeface(null,1);text(page,"使用手机 Amoo 的模型与账号。资料发送到确认的电脑；断线保留记录，连回原电脑后更新进度与传输。",13);
        start=button(page,"确认交给电脑处理",true,()->confirm(false));
        prepare=button(page,"只准备资料，稍后在 Amoo 中说需求",false,()->confirm(true));
        status=text(page,"选择资料并确认后才会发送",13);status.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        text(page,"最近任务",18).setTypeface(null,1);button(page,"刷新任务与连接",false,this::load);
        history=new LinearLayout(this);history.setOrientation(LinearLayout.VERTICAL);page.addView(history);
        SharedPreferences saved=getSharedPreferences("desktop_work_ui",0);
        goal.setText(state==null?saved.getString("goal",""):state.getString("goal",""));
        ArrayList<String> uris=state==null?null:state.getStringArrayList("sources");
        if(uris!=null)for(String uri:uris)selected.add(Uri.parse(uri));
        else if(state==null)try{JSONArray values=new JSONArray(saved.getString("sources","[]"));for(int i=0;i<Math.min(10,values.length());i++)selected.add(Uri.parse(values.getString(i)));}catch(Exception ignored){}
        if(state!=null)handledSubmission=state.getString("handledSubmission","");
        updateSelection();syncSubmission();
    }
    private void confirm(boolean prepareOnly){
        if(submitting()||confirming)return;
        if(selected.isEmpty()){status.setText("请先选择资料");return;}
        String prompt=goal.getText().toString().trim();
        if(!prepareOnly&&(prompt.isEmpty()||prompt.length()>4000)){goal.requestFocus();goal.setError(prompt.isEmpty()?"请填写要完成的任务":"任务描述最多 4000 字");return;}
        TransferStore.Link link=new TransferStore(this).link();
        if(!link.online){status.setText("电脑离线 · 草稿已保留，连接电脑后即可提交");saveDraft();return;}
        final ArrayList<Uri> sources=new ArrayList<>(selected);
        confirming=true;
        AlertDialog dialog=new AlertDialog.Builder(this).setTitle(prepareOnly?"把资料交给 Amoo？":"在电脑开始处理？")
            .setMessage("将 "+sources.size()+" 个资料副本发送到 "+link.name+"。"+(prepareOnly?"稍后可在 Amoo 正常对话中安排任务。":"Amoo 会安排电脑处理，成果自动送回手机。")+"原文件不会删除。")
            .setNegativeButton("取消",null).setPositiveButton("确认",(d,w)->submit(link,prompt,sources,prepareOnly)).create();
        dialog.setOnDismissListener(d->confirming=false);dialog.show();
    }
    private void submit(TransferStore.Link target,String prompt,ArrayList<Uri> sources,boolean prepareOnly){
        Submission job=beginSubmission(prompt,sources,prepareOnly);
        if(job==null){status.setText("资料正在提交，请等待当前结果");return;}
        saveDraft();syncSubmission();
        final Context app=getApplicationContext();
        worker.execute(()->{
            try{
                TransferStore store=new TransferStore(app);JSONArray ids=new JSONArray();
                for(Uri uri:job.sources)ids.put(store.enqueue(uri,target,true).id);
                if(!job.prepareOnly)new DesktopWorkStore(app).create(job.id,job.prompt,ids,0);
                job.success=true;
                job.message=job.prepareOnly?"资料已准备好。现在可以对 Amoo 说：用 StepX伴侣把刚选的资料交给电脑处理。":"任务已提交 · 可离开本页面，稍后回来查看成果";
            }catch(Exception e){job.message="未提交成功："+e.getMessage()+"。草稿已保留。";}
            finally{job.complete=true;}
            ui.post(()->{if(destroyed)return;syncSubmission();load();});
        });
    }
    private void syncSubmission(){
        Submission job=currentSubmission;boolean active=job!=null&&!job.complete;
        start.setEnabled(!active);pick.setEnabled(!active);prepare.setEnabled(!active);goal.setEnabled(!active);
        start.setText(active?"正在提交资料…":"确认交给电脑处理");
        if(job==null)return;
        if(active){status.setText(job.message);return;}
        if(job.id.equals(handledSubmission))return;
        handledSubmission=job.id;
        // A confirmation submits its snapshot, never later edits to another draft.
        if(job.success&&selected.equals(job.sources)&&goal.getText().toString().trim().equals(job.prompt)){
            selected.clear();if(!job.prepareOnly)goal.setText("");updateSelection();saveDraft();
        }
        status.setText(job.message);
    }
    private void load(){
        ui.removeCallbacks(refresh);if(!resumed||destroyed||loading)return;
        syncSubmission();loading=true;final int requestGeneration=generation;
        worker.execute(()->{
            try{
                JSONArray tasks=new DesktopWorkStore(getApplicationContext()).tasks();TransferStore.Link link=new TransferStore(getApplicationContext()).link();
                ui.post(()->{
                    loading=false;if(destroyed)return;
                    if(!resumed||requestGeneration!=generation){if(resumed)load();return;}
                    syncSubmission();connection.setText(link.online?"已连接 · "+link.name:"电脑离线 · 可查看已送达成果，草稿与任务记录保留");
                    String snapshot=tasks.toString();
                    if(!snapshot.equals(lastTasks)){lastTasks=snapshot;renderTasks(tasks);}
                    ui.postDelayed(refresh,4000);
                });
            }catch(Exception e){ui.post(()->{loading=false;if(!destroyed&&resumed){status.setText("读取任务失败："+e.getMessage()+"，可点击刷新重试");ui.postDelayed(refresh,8000);}});}
        });
    }
    private void renderTasks(JSONArray tasks){
        history.removeAllViews();
        for(int i=0;i<tasks.length();i++){
            JSONObject task=tasks.optJSONObject(i);if(task==null)continue;
            LinearLayout card=new LinearLayout(this);card.setOrientation(LinearLayout.VERTICAL);card.setPadding(dp(16),dp(6),dp(16),dp(16));card.setBackground(design.touch(Color.WHITE,CompanionUi.LINE,14));LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.topMargin=dp(12);history.addView(card,lp);
            String phase=task.optString("phase");text(card,DesktopWorkPolicy.phaseLabel(phase),13);
            TextView request=text(card,task.optString("goal"),16);request.setTypeface(null,1);request.setMaxLines(4);request.setEllipsize(android.text.TextUtils.TruncateAt.END);
            if(task.optString("goal").length()>100)button(card,"查看完整需求",false,()->new AlertDialog.Builder(this).setTitle("任务需求").setMessage(task.optString("goal")).setPositiveButton("关闭",null).show());
            String hint=DesktopWorkPolicy.connectionHint(phase,task.optBoolean("desktopOnline"),task.optBoolean("differentDesktop"));
            text(card,task.optString("detail")+(hint.isEmpty()?"":"\n"+hint),13);
            JSONArray outputs=task.optJSONArray("files");
            if(outputs!=null)for(int j=0;j<outputs.length();j++){
                JSONObject file=outputs.optJSONObject(j);if(file!=null&&file.optBoolean("delivered"))button(card,"打开 · "+file.optString("name"),false,()->open(file.optString("id")));
            }
            if(Arrays.asList("failed","interrupted","cancelled").contains(phase))button(card,task.has("retryTask")?"查看 / 继续上次重试":"重新尝试",false,()->retry(task.optString("task")));
            if(Arrays.asList("queued","running","awaiting_delivery").contains(phase))button(card,"取消此任务",false,()->cancel(task.optString("task")));
        }
        if(tasks.length()==0)text(history,"任务进度和返回的成果会显示在这里。",14);
    }
    private void cancel(String id){
        if(confirming)return;confirming=true;
        AlertDialog dialog=new AlertDialog.Builder(this).setTitle("取消电脑任务？").setMessage("已生成的文件会保留，原始资料不会改变。")
            .setNegativeButton("继续处理",null).setPositiveButton("取消任务",(d,w)->worker.execute(()->{
                try{JSONObject result=new DesktopWorkStore(getApplicationContext()).cancel(id,-1);ui.post(()->{if(destroyed)return;Toast.makeText(this,"completed".equals(result.optString("phase"))?"任务已完成，成果已送达；未删除文件。":"已请求取消，等待原电脑停止确认。",Toast.LENGTH_LONG).show();load();});}
                catch(Exception e){ui.post(()->{if(!destroyed)status.setText(e.getMessage());});}
            })).create();
        dialog.setOnDismissListener(d->confirming=false);dialog.show();
    }
    private void retry(String id){
        if(confirming)return;confirming=true;
        AlertDialog dialog=new AlertDialog.Builder(this).setTitle("重新尝试此任务？").setMessage("使用原资料与任务要求，需要连接原电脑。重复点击沿用同一次重试，原记录和成果保留。")
            .setNegativeButton("取消",null).setPositiveButton("确认重试",(d,w)->worker.execute(()->{
                try{new DesktopWorkStore(getApplicationContext()).retry(id,UUID.randomUUID().toString(),-1);ui.post(()->{if(destroyed)return;status.setText("已提交重试，可在最近任务中查看进度");load();});}
                catch(Exception e){ui.post(()->{if(!destroyed)status.setText("未能重试："+e.getMessage());});}
            })).create();
        dialog.setOnDismissListener(d->confirming=false);dialog.show();
    }
    private void open(String id){
        try{TransferStore store=new TransferStore(this);TransferStore.Entry entry=store.get("inbox",id);if(TransferPreviewActivity.supports(entry.name)){TransferPreviewActivity.open(this,"inbox",id);return;}Intent i=new Intent(Intent.ACTION_VIEW).setDataAndType(entry.uri(),TransferStore.mime(entry.name)).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);i.setClipData(ClipData.newRawUri(entry.name,entry.uri()));startActivity(Intent.createChooser(i,"打开成果"));}
        catch(Exception e){Toast.makeText(this,"可到伴侣文件页保存或分享成果",Toast.LENGTH_LONG).show();}
    }
    @Override protected void onActivityResult(int request,int result,Intent data){
        super.onActivityResult(request,result,data);if(request!=41||result!=RESULT_OK||data==null||submitting())return;
        ClipData clips=data.getClipData();
        if(clips!=null){if(clips.getItemCount()>10){status.setText("一次最多选择 10 个资料");return;}selected.clear();for(int i=0;i<clips.getItemCount();i++){Uri uri=clips.getItemAt(i).getUri();if(!selected.contains(uri))selected.add(uri);}}
        else if(data.getData()!=null){selected.clear();selected.add(data.getData());}
        for(Uri uri:selected)try{getContentResolver().takePersistableUriPermission(uri,data.getFlags()&Intent.FLAG_GRANT_READ_URI_PERMISSION);}catch(Exception ignored){}
        updateSelection();saveDraft();
    }
    private void updateSelection(){
        selection.setText(selected.isEmpty()?"还未选择文件":"已选择 "+selected.size()+" 个资料");
        if(selected.isEmpty()||destroyed)return;
        final ArrayList<Uri> sources=new ArrayList<>(selected);
        // Document providers can be remote or slow; never query their names on the UI thread.
        worker.execute(()->{String label=selectionLabel(sources);ui.post(()->{if(!destroyed&&selected.equals(sources))selection.setText(label);});});
    }
    private String selectionLabel(ArrayList<Uri> sources){
        StringBuilder label=new StringBuilder("已选择 "+sources.size()+" 个资料");
        for(Uri uri:sources){String name=uri.getLastPathSegment();try(android.database.Cursor cursor=getContentResolver().query(uri,new String[]{android.provider.OpenableColumns.DISPLAY_NAME},null,null,null)){if(cursor!=null&&cursor.moveToFirst())name=cursor.getString(0);}catch(Exception ignored){}label.append("\n• ").append(name);}
        return label.toString();
    }
    private void saveDraft(){
        if(goal==null)return;JSONArray sources=new JSONArray();for(Uri uri:selected)sources.put(uri.toString());
        getSharedPreferences("desktop_work_ui",0).edit().putString("goal",goal.getText().toString()).putString("sources",sources.toString()).apply();
    }
    @Override protected void onSaveInstanceState(Bundle state){
        state.putString("goal",goal.getText().toString());ArrayList<String> uris=new ArrayList<>();for(Uri uri:selected)uris.add(uri.toString());state.putStringArrayList("sources",uris);state.putString("handledSubmission",handledSubmission);super.onSaveInstanceState(state);
    }
    @Override protected void onResume(){super.onResume();resumed=true;syncSubmission();load();}
    @Override protected void onPause(){saveDraft();resumed=false;generation++;ui.removeCallbacks(refresh);super.onPause();}
    @Override protected void onDestroy(){destroyed=true;worker.shutdown();ui.removeCallbacksAndMessages(null);super.onDestroy();}
}
