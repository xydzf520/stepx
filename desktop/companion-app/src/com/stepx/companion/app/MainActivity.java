package com.stepx.companion.app;

import android.Manifest;
import android.app.*;
import android.content.*;
import android.content.pm.PackageManager;
import android.graphics.*;
import android.graphics.drawable.*;
import android.net.Uri;
import android.os.*;
import android.provider.Settings;
import android.view.*;
import android.widget.*;
import org.json.*;
import java.io.*;
import java.util.*;
import java.util.concurrent.*;

/** Phone companion: an explicit connection, scoped file sharing, and user-owned background controls. */
public final class MainActivity extends Activity {
    private static final int PICK_FILES=30,SAVE_FILE=31;
    private final Handler ui=new Handler(Looper.getMainLooper());
    private final ExecutorService files=Executors.newSingleThreadExecutor();
    private final ExecutorService statusWorker=Executors.newSingleThreadExecutor();
    private CompanionUi design;
    private ProgressBar loadingLine; private Runnable loadingReveal;
    private void endLoading(){if(loadingReveal!=null)ui.removeCallbacks(loadingReveal);if(loadingLine!=null)loadingLine.setVisibility(View.INVISIBLE);}
    private EditText taskInput; private TextView taskReadiness,taskBadge,taskMetrics,taskGoal,homeStatus,homeComputer,homeHint,homeTask;
    private LinearLayout taskSteps,taskHistory,taskDock,composeCard,liveCard; private ScrollView pageScroll; private Button taskDetails;
    private String taskDraft="",lastTaskRender="",lastHistoryRender="";
    private JSONObject lastAgent=new JSONObject(); private boolean refreshPending;
    private int pageGeneration,historyOrigin=3,connectionOrigin=0;
    private TransferStore.Link cachedLink=new TransferStore.Link("","电脑",false);
    private final int ink=CompanionUi.INK,secondary=CompanionUi.MUTED,line=CompanionUi.LINE,green=CompanionUi.GREEN;
    private LinearLayout header; private TextView historyTitle; private final Map<String,TextView> permissionLabels=new HashMap<>();
    private FrameLayout content; private LinearLayout navigation,transferList;
    private TextView linkTitle,linkHint,connectionFoot,transferHint,amooState,gameState,notice;
    private TextView guiState;
    private Button guiStart,guiPause,guiResume,guiStop,homeScan,connScan;
    private String guiPhase=""; private boolean guiReady;
    private Button send; private Switch amooSwitch;
    private TransferStore store; private TransferStore.Link selectedComputer; private TransferStore.Entry saving;
    private int tab=0,transferDirection; private boolean resumed,busy,updatingSwitch,listing,destroyed;
    private String lastListing="",enqueueError="";
    private int dp(float n){return Math.round(n*getResources().getDisplayMetrics().density);}
    private String version(){try{return getPackageManager().getPackageInfo(getPackageName(),0).versionName;}catch(Exception ignored){return "";}}
    private GradientDrawable surface(int fill,int stroke,int radius){GradientDrawable d=new GradientDrawable();d.setColor(fill);d.setCornerRadius(dp(radius));if(stroke!=0)d.setStroke(dp(1),stroke);return d;}
    private LinearLayout column(){LinearLayout v=new LinearLayout(this);v.setOrientation(LinearLayout.VERTICAL);return v;}
    private TextView text(LinearLayout p,String value,int size,int color,int top){TextView t=new TextView(this);t.setText(value);t.setTextSize(size);t.setTextColor(color);t.setLineSpacing(dp(3),1);LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.topMargin=dp(top);p.addView(t,lp);return t;}
    private TextView heading(LinearLayout p,String value,int top){TextView t=text(p,value,17,ink,top);t.setTypeface(CompanionUi.MEDIUM);return t;}
    private LinearLayout card(LinearLayout p,int top){LinearLayout v=column();v.setPadding(dp(16),dp(16),dp(16),dp(16));v.setBackground(surface(Color.WHITE,line,12));LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.topMargin=dp(top);p.addView(v,lp);return v;}
    private Button button(LinearLayout p,String title,boolean primary,Runnable action){Button b=new Button(this);b.setText(title);b.setAllCaps(false);b.setTextSize(15);b.setTextColor(primary?Color.WHITE:ink);b.setBackgroundTintList(null);b.setBackground(surface(primary?ink:Color.WHITE,primary?0:line,12));b.setPadding(dp(12),dp(8),dp(12),dp(8));b.setMinHeight(dp(48));LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.topMargin=dp(12);p.addView(b,lp);design.styleButton(b,primary);b.setOnClickListener(v->action.run());return b;}
    private LinearLayout page(String title,String subtitle){ScrollView scroll=new ScrollView(this);pageScroll=scroll;scroll.setFillViewport(true);scroll.setClipToPadding(false);LinearLayout page=column();page.setPadding(dp(16),dp(20),dp(16),dp(24));if(tab<4)heading(page,title,0).setTextSize(21);if(!subtitle.isEmpty())text(page,subtitle,13,secondary,6);scroll.addView(page);content.addView(scroll,new FrameLayout.LayoutParams(-1,-1));return page;}
    @Override public void onCreate(Bundle saved){super.onCreate(saved);design=new CompanionUi(this);store=new TransferStore(this);taskDraft=getSharedPreferences("phone_ui",0).getString("taskDraft","");if(saved!=null){tab=saved.getInt("tab",0);historyOrigin=saved.getInt("historyOrigin",3);connectionOrigin=saved.getInt("connectionOrigin",0);taskDraft=saved.getString("taskDraft",taskDraft);transferDirection=saved.getInt("direction",0);try{String id=saved.getString("savingId");if(id!=null)saving=store.get(saved.getString("savingDirection"),id);String target=saved.getString("targetId");if(target!=null)selectedComputer=new TransferStore.Link(target,saved.getString("targetName","电脑"),true);}catch(Exception ignored){}}
        LinearLayout root=column();root.setBackgroundColor(CompanionUi.BG);root.setOnApplyWindowInsetsListener((v,insets)->{Insets b=insets.getInsets(WindowInsets.Type.systemBars()|WindowInsets.Type.ime());v.setPadding(b.left,b.top,b.right,b.bottom);return insets;});
        header=new LinearLayout(this);header.setGravity(Gravity.CENTER_VERTICAL);header.setPadding(dp(12),dp(4),dp(16),dp(4));header.setMinimumHeight(dp(56));header.setBackgroundColor(Color.WHITE);root.addView(header);
        content=new FrameLayout(this);root.addView(content,new LinearLayout.LayoutParams(-1,0,1));navigation=new LinearLayout(this);navigation.setPadding(dp(12),dp(6),dp(12),dp(6));navigation.setBackgroundColor(Color.WHITE);root.addView(navigation,new LinearLayout.LayoutParams(-1,dp(64)));setContentView(root);root.requestApplyInsets();
        getWindow().getInsetsController().setSystemBarsAppearance(WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS|WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS|WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS);
        AutoConnectService.resume(this);showTab(tab);
    }
    private void showTab(int next){
        endLoading();CompanionMotion.stop(content);rememberDraft();hostSurface(false);if(next==8&&tab<4)historyOrigin=tab;if(next==4&&tab<4)connectionOrigin=tab;tab=next;pageGeneration++;content.removeAllViews();navigation.removeAllViews();linkTitle=linkHint=connectionFoot=transferHint=amooState=gameState=notice=null;guiState=taskReadiness=taskBadge=taskMetrics=taskGoal=homeStatus=homeComputer=homeHint=homeTask=null;guiStart=guiPause=guiResume=guiStop=taskDetails=homeScan=connScan=null;taskInput=null;taskSteps=taskHistory=taskDock=composeCard=liveCard=null;pageScroll=null;send=null;amooSwitch=null;transferList=null;historyTitle=null;permissionLabels.clear();lastListing=lastTaskRender=lastHistoryRender="";
        updateHeader();
        String[] labels={"首页","文件","设置","任务"};int[] icons={UiGlyph.HOME,UiGlyph.FOLDER,UiGlyph.SETTINGS,UiGlyph.TASK};
        for(int i:new int[]{0,3,1,2}){final int index=i;boolean active=tab==i||parentTab(tab)==i;TextView item=new TextView(this);item.setText(labels[i]);item.setContentDescription(labels[i]);item.setTextSize(11);item.setGravity(Gravity.CENTER);item.setTextColor(active?ink:secondary);item.setCompoundDrawablePadding(dp(4));UiGlyph glyph=new UiGlyph(icons[i],active?ink:secondary);glyph.setBounds(0,0,dp(22),dp(22));item.setCompoundDrawables(null,glyph,null,null);item.setBackground(design.touch(active?0xfff1f1f4:Color.WHITE,0,8));LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(0,-1,1);lp.leftMargin=dp(4);lp.rightMargin=dp(4);navigation.addView(item,lp);item.setOnClickListener(v->{if(tab!=index)showTab(index);});}
        if(tab==3)taskPage();else if(tab==0)homePage();else if(tab==4)connectionPage();else if(tab==5)backscreenPage();else if(tab==1)filesPage();else if(tab==6)permissionsPage();else if(tab==7)backgroundPage();else if(tab==8)historyPage();else if(tab==9)aboutPage();else if(tab==10)integrationPage();else settingsPage();
        loadingLine=CompanionMotion.progress(this);FrameLayout.LayoutParams progressLayout=new FrameLayout.LayoutParams(-1,dp(3),Gravity.TOP);content.addView(loadingLine,progressLayout);
        final int g=pageGeneration;loadingReveal=()->{if(resumed&&g==pageGeneration&&loadingLine!=null)loadingLine.setVisibility(View.VISIBLE);};ui.postDelayed(loadingReveal,150);CompanionMotion.enter(pageScroll==null?content:pageScroll);refresh();
    }
    private int parentTab(int page){return page==4?connectionOrigin:page==5?0:page==8?historyOrigin:page>=6?2:-1;}
    private void updateHeader(){
        header.removeAllViews();boolean detail=tab>=4;
        if(detail){ImageButton back=new ImageButton(this);back.setImageDrawable(new UiGlyph(UiGlyph.BACK,ink));back.setContentDescription("返回");back.setPadding(dp(12),dp(12),dp(12),dp(12));back.setBackground(design.touch(Color.WHITE,0,8));header.addView(back,new LinearLayout.LayoutParams(dp(48),dp(48)));back.setOnClickListener(v->showTab(parentTab(tab)));}
        else {ImageView icon=new ImageView(this);icon.setImageResource(getApplicationInfo().icon);LinearLayout.LayoutParams ip=new LinearLayout.LayoutParams(dp(26),dp(26));ip.leftMargin=dp(4);ip.rightMargin=dp(12);header.addView(icon,ip);}
        String[] titles={"StepX伴侣","StepX伴侣","StepX伴侣","StepX伴侣","连接管理","背屏与游戏","权限与服务","后台协作","任务记录","关于伴侣","Amoo 委派"};
        TextView title=new TextView(this);title.setText(titles[tab]);title.setTextSize(17);title.setTypeface(CompanionUi.MEDIUM);title.setTextColor(ink);header.addView(title,new LinearLayout.LayoutParams(0,-2,1));
    }
    private LinearLayout group(LinearLayout p,String title){if(!title.isEmpty())text(p,title,12,secondary,20);LinearLayout c=card(p,8);c.setPadding(dp(12),0,dp(12),0);return c;}
    private void row(LinearLayout p,int icon,String title,String detail,Runnable action){if(p.getChildCount()>0){View divider=new View(this);divider.setBackgroundColor(line);LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,dp(1));lp.leftMargin=dp(40);p.addView(divider,lp);}design.row(p,icon,title,detail,action);}
    private boolean guiCommandPending,officialSkillPending;
    private void homePage(){
        LinearLayout p=page("首页","");LinearLayout hero=card(p,12);
        homeStatus=text(hero,"正在检查连接…",12,secondary,0);homeComputer=heading(hero,"连接你的电脑",6);
        homeHint=text(hero,"连接后，在电脑使用 Amoo、文件与手机工具。",13,secondary,8);
        homeScan=button(hero,"扫码连接",true,()->startActivity(new Intent(this,PairingActivity.class)));UiGlyph.attach(homeScan,UiGlyph.SCAN,Color.WHITE,dp(18));
        row(hero,UiGlyph.COMPUTER,"连接管理","自动重连、USB 与无线配对",()->showTab(4));
        LinearLayout agent=group(p,"手机独立执行");row(agent,UiGlyph.TASK,"手机任务","输入目标，由 GUI Agent 看屏操作",()->showTab(3));homeTask=text(agent,"正在检查任务状态…",12,secondary,0);homeTask.setPadding(dp(40),0,0,dp(12));homeTask.setMaxLines(2);homeTask.setEllipsize(android.text.TextUtils.TruncateAt.END);
        LinearLayout tools=group(p,"更多工具");row(tools,UiGlyph.FOLDER,"存储扩容","原件存电脑 · 手机保留文件入口",()->startActivity(new Intent(this,ExpansionActivity.class)));row(tools,UiGlyph.STUDIO,"Studio 作品","打开电脑推送的作品与官网项目",()->startActivity(new Intent(this,StudioActivity.class)));row(tools,UiGlyph.SCREEN,"背屏与游戏","三敲启动，倾斜手机游玩",()->showTab(5));
    }
    private Button twoButton(LinearLayout row,String label,int icon,boolean primary,Runnable action){LinearLayout cell=column();LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(0,-2,1);if(row.getChildCount()>0)lp.leftMargin=dp(8);row.addView(cell,lp);Button b=button(cell,label,primary,action);UiGlyph.attach(b,icon,primary?Color.WHITE:ink,dp(18));return b;}
    private void taskPage(){
        LinearLayout p=page("任务","由手机看屏执行，随时暂停或接管。");
        LinearLayout compose=card(p,16);composeCard=compose;
        taskInput=new EditText(this);taskInput.setTextSize(14);taskInput.setTextColor(ink);taskInput.setHintTextColor(secondary);taskInput.setHint("例如：打开设置，查看 Android 版本");taskInput.setGravity(Gravity.TOP);taskInput.setMinLines(3);taskInput.setMaxLines(6);taskInput.setInputType(android.text.InputType.TYPE_CLASS_TEXT|android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE|android.text.InputType.TYPE_TEXT_FLAG_CAP_SENTENCES);taskInput.setPadding(dp(12),dp(12),dp(12),dp(12));taskInput.setBackground(surface(CompanionUi.SURFACE,line,8));LinearLayout.LayoutParams ip=new LinearLayout.LayoutParams(-1,-2);ip.topMargin=dp(12);compose.addView(taskInput,ip);taskInput.setText(taskDraft);
        taskInput.addTextChangedListener(new android.text.TextWatcher(){public void beforeTextChanged(CharSequence v,int a,int c,int f){}public void onTextChanged(CharSequence v,int a,int b,int c){taskDraft=v.toString();updateGuiControls();}public void afterTextChanged(android.text.Editable e){}});
        TextView examples=text(compose,"试试一个示例",13,secondary,4);examples.setMinHeight(dp(44));examples.setGravity(Gravity.CENTER_VERTICAL);UiGlyph.attach(examples,UiGlyph.PLAY,secondary,dp(16));examples.setOnClickListener(v->new AlertDialog.Builder(this).setTitle("选择任务示例").setItems(new String[]{"查看 Android 版本","搜索电池设置"},(d,w)->fillTask(w==0?"打开设置，查看 Android 版本":"打开设置，在搜索框中输入“电池”，查看搜索结果，不修改设置")).show());
        guiStart=button(compose,"开始任务",true,this::startTask);UiGlyph.attach(guiStart,UiGlyph.PLAY,Color.WHITE,dp(18));
        taskReadiness=text(compose,"正在检查执行器…",12,secondary,10);
        LinearLayout live=card(p,12);liveCard=live;live.setVisibility(View.GONE);taskBadge=text(live,"正在检查执行器…",13,secondary,0);taskBadge.setTypeface(CompanionUi.MEDIUM);taskGoal=heading(live,"当前任务",12);guiState=text(live,"正在读取任务状态",14,secondary,8);guiState.setTextIsSelectable(true);taskMetrics=text(live,"",12,secondary,12);taskSteps=column();live.addView(taskSteps);
        taskDock=column();taskDock.setPadding(dp(16),0,dp(16),dp(12));taskDock.setBackground(design.shape(Color.WHITE,line,0));FrameLayout.LayoutParams dockParams=new FrameLayout.LayoutParams(-1,-2,Gravity.BOTTOM);content.addView(taskDock,dockParams);LinearLayout controls=new LinearLayout(this);taskDock.addView(controls);guiPause=twoButton(controls,"暂停",UiGlyph.PAUSE,false,()->guiControl("pause"));guiResume=twoButton(controls,"继续",UiGlyph.PLAY,false,()->guiControl("resume"));guiStop=twoButton(controls,"停止",UiGlyph.STOP,false,()->guiControl("stop"));
        taskDetails=button(live,"查看完整任务详情",false,this::showTaskDetails);
        LinearLayout setup=card(p,12);setup.setPadding(dp(12),dp(2),dp(12),dp(2));design.row(setup,UiGlyph.MODEL,"模型设置","默认全部应用，按需调整执行范围",this::openModelSettings);
        historyTitle=text(p,"最近任务",12,secondary,20);taskHistory=column();p.addView(taskHistory);LinearLayout more=group(p,"");row(more,UiGlyph.CLOCK,"全部任务记录","查看结果、重新编辑",()->showTab(8));updateGuiControls();
    }
    private void fillTask(String value){taskDraft=value;if(taskInput!=null){taskInput.setText(value);taskInput.setSelection(value.length());taskInput.requestFocus();}}
    private void rememberDraft(){if(taskInput!=null)taskDraft=taskInput.getText().toString();getSharedPreferences("phone_ui",0).edit().putString("taskDraft",taskDraft).apply();}
    private void openModelSettings(){try{if(NeoAgent.installed(this))NeoAgent.openSettings(this);else if(NeoAgent.selected(this))new AlertDialog.Builder(this).setTitle("需要恢复任务组件").setMessage("当前安装包缺少已选择的 GUI Agent。请连接电脑，在小工具中更新带内置 GUI Agent 的 StepX伴侣。已有的模型配置与任务草稿会保留。").setPositiveButton("知道了",null).show();else guiKeyDialog();}catch(Exception e){toast(e.getMessage());}}
    private void showTaskDetails(){taskDetail(lastAgent);}
    private void taskDetail(JSONObject value){
        JSONObject result=value.optJSONObject("result");String phase=value.optString("phase");
        String detail=NeoPolicy.statusText(phase,result!=null&&result.optBoolean("verified"),result!=null&&result.optBoolean("phoneStopped"),value.optString("progress"));
        TextView body=new TextView(this);body.setPadding(dp(22),dp(12),dp(22),dp(12));body.setTextSize(15);body.setTextColor(ink);body.setTextIsSelectable(true);body.setText(value.optString("goal","尚未发起任务")+"\n\n"+detail+"\n\n"+"执行器反馈：\n"+value.optString("progress")+"\n\n模型："+value.optString("model","未配置")+"\n执行动作 "+value.optInt("actions")+" · 看屏 "+value.optInt("screenshots")+" · 模型调用 "+value.optInt("modelCalls"));ScrollView scroll=new ScrollView(this);scroll.addView(body);
        new AlertDialog.Builder(this).setTitle("任务详情").setView(scroll).setNegativeButton("关闭",null).setPositiveButton("再次编辑",(d,w)->{String goal=value.optString("goal");if(tab!=3)showTab(3);fillTask(goal);}).show();
    }
    private void startTask(){
        if(guiCommandPending||!NeoPolicy.canStart(guiPhase,guiReady)){toast("请先完成模型与权限配置，或结束当前任务");return;}
        final String goal=taskInput==null?"":taskInput.getText().toString().trim();if(goal.isEmpty()){if(taskInput!=null)taskInput.setError("请先描述任务");return;}if(goal.length()>4000){taskInput.setError("任务描述最多 4000 字");return;}
        rememberDraft();guiCommandPending=true;updateGuiControls();guiStart.setText("正在提交…");
        statusWorker.execute(()->{try{JSONObject request=new JSONObject().put("op","agent").put("agentOp","start").put("goal",goal);
            JSONObject value=GuiDelegation.desktop(this,request);if(!value.optBoolean("ok",true))throw new IOException(value.optString("error","任务未被接受"));
            ui.post(()->{guiCommandPending=false;if(destroyed)return;lastAgent=value;guiPhase=value.optString("phase");updateGuiControls();if("running".equals(guiPhase)||"queued".equals(guiPhase)||"planning".equals(guiPhase)){hideTaskKeyboard();taskDraft="";if(taskInput!=null)taskInput.setText("");rememberDraft();moveTaskToBack(true);}else{toast("任务未进入执行，请查看状态");refresh();}});
        }catch(Exception e){ui.post(()->{guiCommandPending=false;if(destroyed)return;if(guiStart!=null)guiStart.setText("开始任务");if(guiState!=null)guiState.setText("未能开始："+e.getMessage());updateGuiControls();toast(e.getMessage());});}});
    }
    private void officialSkill(boolean install,TextView target){
        if(officialSkillPending){toast("正在查询官方技能，请稍候");return;}
        if(install&&!NeoAgent.installed(this)){target.setText("请先更新包含 Neo Agent Touch 的完整伴侣 APK，再连接到 Amoo。");return;}
        officialSkillPending=true;target.setText(install?"正在检查并安装官方测试技能…":"正在读取官方技能状态…");
        new Thread(()->{String result;try{
            JSONObject status=OfficialGuiSkill.status(getApplicationContext());
            if(install)status=OfficialGuiSkill.install(getApplicationContext(),new JSONObject().put("scope",status.getString("scope")).put("version",status.getJSONObject("candidate").getString("version")).put("testing",true).put("requestId",java.util.UUID.randomUUID().toString()));
            result=OfficialGuiSkill.display(status);
        }catch(Exception e){result="未完成："+OfficialGuiSkillPolicy.safeMessage(e.getMessage());}
        final String message=result;ui.post(()->{officialSkillPending=false;if(!destroyed)target.setText(message);});},"Official-GUI-skill").start();
    }
    private void connectionPage(){LinearLayout p=page("连接管理","");LinearLayout c=card(p,24);text(c,"当前连接",12,secondary,0);linkTitle=heading(c,"正在读取连接状态…",12);linkHint=text(c,"",14,secondary,8);connScan=button(c,"扫码连接",true,()->startActivity(new Intent(this,PairingActivity.class)));UiGlyph.attach(connScan,UiGlyph.SCAN,Color.WHITE,dp(18));connectionFoot=text(c,"",12,secondary,12);
        LinearLayout automatic=card(p,18);heading(automatic,"同一局域网自动连接",0);text(automatic,AutoConnectService.enabled(this)?"已开启 · 电脑打开后自动连接，无需再次扫码。手机重启后完成首次解锁，连接服务自动恢复。":"配对一次后可自动重连，不需要每次扫码。",14,secondary,8);
        button(automatic,AutoConnectService.enabled(this)?"暂停自动连接":"开启自动连接",!AutoConnectService.enabled(this),()->{
            boolean on=!AutoConnectService.enabled(this);
            new AlertDialog.Builder(this).setTitle(on?"开启自动连接？":"暂停自动连接？").setMessage(on?"同一局域网内，电脑运行 StepX Companion 后自动连接。仅使用已授权的电脑和 Android 安全无线调试；首次使用新 Wi-Fi 时，系统仍可能要求确认。后台会显示连接通知。":"暂停后台自动恢复无线连接，保留已配对记录与当前连接。要断开当前无线连接，可在系统无线调试中关闭。").setNegativeButton("取消",null).setPositiveButton(on?"开启":"暂停",(d,w)->{if(on&&checkSelfPermission("android.permission.WRITE_SECURE_SETTINGS")!=PackageManager.PERMISSION_GRANTED){repairHelp("自动连接");return;}AutoConnectService.setEnabled(this,on);showTab(4);}).show();
        });
        row(automatic,UiGlyph.LINK,"系统无线调试","查看已配对电脑、撤销授权或关闭无线连接",()->{try{startActivity(new Intent("android.settings.WIRELESS_DEBUGGING_SETTINGS"));}catch(Exception e){startActivity(new Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS));}});
        LinearLayout guide=card(p,18);heading(guide,"首次连接",0);text(guide,"无线连接",15,ink,16);text(guide,"1. 手机和电脑连接同一 Wi-Fi。\n2. desktop 打开“连接设备”并显示二维码。\n3. 在此扫码，确认要连接的电脑。",14,secondary,8);text(guide,"USB 连接",15,ink,18);text(guide,"使用 USB 数据线连接，在手机上确认这台电脑。desktop 连接成功后，此处会显示电脑名称。",14,secondary,8);
        
    }
    private void filesPage(){LinearLayout p=page("文件","与已连接的电脑互传文件。");LinearLayout expGroup=group(p,"");row(expGroup,UiGlyph.FOLDER,"存储扩容","自动迁移 · 文件分类 · 查看与恢复",()->startActivity(new Intent(this,ExpansionActivity.class)));LinearLayout c=card(p,22);transferHint=text(c,"正在读取电脑连接…",14,secondary,0);send=button(c,busy?"正在准备文件…":"选择文件，发送到电脑",true,this::chooseFiles);text(c,"单个文件最多 256 MB，一次可选择 10 个。",12,secondary,10);notice=text(c,"",13,secondary,8);
        LinearLayout tabs=new LinearLayout(this);LinearLayout.LayoutParams tp=new LinearLayout.LayoutParams(-1,dp(46));tp.topMargin=dp(22);p.addView(tabs,tp);String[] labels={"收到的文件","发出的文件"};for(int i=0;i<2;i++){final int d=i;TextView t=new TextView(this);t.setText(labels[i]);t.setGravity(Gravity.CENTER);t.setTextSize(15);t.setTextColor(transferDirection==i?Color.WHITE:secondary);t.setBackground(surface(transferDirection==i?ink:Color.TRANSPARENT,0,8));tabs.addView(t,new LinearLayout.LayoutParams(0,-1,1));t.setOnClickListener(v->{transferDirection=d;showTab(1);});}
        transferList=column();p.addView(transferList);loadTransfers();
    }
    private void settingsPage(){
        LinearLayout p=page("设置","");
        LinearLayout task=group(p,"任务与模型");row(task,UiGlyph.MODEL,"模型设置","API、密钥与应用范围",this::openModelSettings);row(task,UiGlyph.LINK,"Amoo 委派","实验接入 · 查看实际可用状态",()->showTab(10));
        LinearLayout services=group(p,"设备服务");row(services,UiGlyph.SHIELD,"权限与服务","看屏、通知同步、扫码与背屏",()->showTab(6));row(services,UiGlyph.COMPUTER,"后台协作","管理电脑连接期间的 Amoo 任务",()->showTab(7));
        LinearLayout look=group(p,"显示");look.setPadding(dp(12),0,dp(12),dp(14));Switch motion=new Switch(this);motion.setText("减少动态效果");motion.setTextColor(ink);motion.setTextSize(15);motion.setMinHeight(dp(52));motion.setChecked(getSharedPreferences("phone_ui",0).getBoolean("reduced_motion",false));look.addView(motion);text(look,"关闭页面过渡，加载时使用静态提示。",13,secondary,4);motion.setOnCheckedChangeListener((v,on)->{motion.setOnCheckedChangeListener(null);motion.setChecked(!on);new AlertDialog.Builder(this).setTitle("保存显示设置？").setMessage("仅影响这台手机的伴侣界面。减少动态效果："+(on?"开启":"关闭")+"。确认后立即生效。").setNegativeButton("取消",(d,w)->showTab(2)).setOnCancelListener(d->showTab(2)).setPositiveButton("保存并应用",(d,w)->{getSharedPreferences("phone_ui",0).edit().putBoolean("reduced_motion",on).apply();if(on)CompanionMotion.stop(content);toast("显示设置已保存");showTab(2);}).show();});
        LinearLayout app=group(p,"应用");row(app,UiGlyph.CLOCK,"任务记录","查看历史与管理本机记录",()->showTab(8));row(app,UiGlyph.HOME,"关于伴侣","版本 "+version()+" · 项目说明",()->showTab(9));
    }
    private void permissionRow(LinearLayout p,String id,int icon,String title,Runnable action){LinearLayout r=design.row(p,icon,title,"正在检查…",action);TextView label=(TextView)((LinearLayout)r.getChildAt(1)).getChildAt(1);permissionLabels.put(id,label);}
    private void permissionsPage(){
        LinearLayout p=page("权限与服务","只在使用对应功能时生效；可随时在系统设置中管理。");
        LinearLayout core=group(p,"手机操作");permissionRow(core,"gui",UiGlyph.SCREEN,"看屏操作",()->startActivity(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)));permissionRow(core,"notifications",UiGlyph.TASK,"手机通知同步",()->startActivity(new Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)));
        LinearLayout device=group(p,"连接与背屏");Runnable app=()->startActivity(new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,Uri.parse("package:"+getPackageName())));
        permissionRow(device,"camera",UiGlyph.SCAN,"扫码相机",app);permissionRow(device,"notice",UiGlyph.TASK,"服务通知",app);permissionRow(device,"screen",UiGlyph.SCREEN,"背屏控制",()->repairHelp("背屏控制"));permissionRow(device,"pairing",UiGlyph.LINK,"安全扫码服务",()->startActivity(new Intent(this,PairingActivity.class)));permissionRow(device,"power",UiGlyph.CLOCK,"后台电量管理",()->startActivity(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)));
    }
    private boolean bootRecovering(){return android.os.SystemClock.elapsedRealtime()<180000||AutoConnectService.state(this).startsWith("正在恢复开机");}
    private void repairHelp(String feature){new AlertDialog.Builder(this).setTitle(feature).setMessage(bootRecovering()?"手机正在恢复开机服务，请完成首次解锁后等待片刻。已配对电脑会自动连接，不需要再次扫码。":"权限检查未通过。请先确认已安装最新版伴侣并完成首次解锁；仍未恢复时，在电脑连接设备页选择“更新 / 修复 StepX伴侣”。修复会同时配置后续的开机恢复。").setPositiveButton("知道了",null).show();}
    private void backgroundPage(){
        LinearLayout p=page("后台协作","电脑连接期间，按需维持 Amoo 任务。断开后自动释放。");LinearLayout c=card(p,16);
        amooSwitch=new Switch(this);amooSwitch.setText("允许后台协作");amooSwitch.setTextSize(15);amooSwitch.setTextColor(ink);amooSwitch.setMinHeight(dp(48));c.addView(amooSwitch,new LinearLayout.LayoutParams(-1,-2));amooState=text(c,"正在检查…",13,secondary,8);
        text(p,"此设置需要 StepX 固件适配，不会绕过手机锁屏或系统授权。",12,secondary,16);
        amooSwitch.setOnCheckedChangeListener((b,checked)->{if(updatingSwitch)return;updatingSwitch=true;amooSwitch.setChecked(!checked);updatingSwitch=false;new AlertDialog.Builder(this).setTitle(checked?"开启 Amoo 后台连接？":"关闭 Amoo 后台连接？").setMessage(checked?"将在这台手机启动后台服务，用于熄屏期间保持 Amoo 任务连接。可能增加耗电，通知栏会显示运行状态。":"正在执行的后台任务可能失去连接。关闭后可在这里重新开启。").setNegativeButton("取消",null).setPositiveButton("确认",(d,w)->{try{if(checked)notifications();startForegroundService(new Intent(this,AmooConnectionService.class).putExtra("op",checked?"enable":"disable").putExtra("request_id","phone-"+System.currentTimeMillis()));amooState.setText(checked?"正在开启…":"正在停止…");}catch(Exception e){toast(e.getMessage());refresh();}}).show();});
    }
    private void integrationPage(){LinearLayout p=page("Amoo 委派","");LinearLayout c=card(p,12);heading(c,"实验接入",0);text(c,"端侧 MCP 已能调用手机 GUI Agent。新会话自动发现工具仍不稳定，官方技能安装尚未完成验收。",14,secondary,8);TextView state=text(c,"你仍可在任务页直接发起手机操作。",13,secondary,12);button(c,"检查接入状态",true,()->officialSkill(false,state));}
    private void historyPage(){LinearLayout p=page("任务记录","");taskHistory=column();p.addView(taskHistory);LinearLayout data=group(p,"记录管理");row(data,UiGlyph.CLOCK,"清空任务记录","只移除本机摘要，保留当前任务和模型配置",()->new AlertDialog.Builder(this).setTitle("清空任务记录？").setMessage("清空后无法恢复，不中断正在执行的任务。").setNegativeButton("取消",null).setPositiveButton("清空",(d,w)->statusWorker.execute(()->{AgentHistory.clear(this);ui.post(()->{lastHistoryRender="";refresh();});})).show());}
    private void aboutPage(){LinearLayout p=page("关于伴侣","");LinearLayout c=card(p,12);heading(c,"StepX Companion",0);text(c,"StepX伴侣 · "+version(),14,secondary,8);text(c,"手机独立执行，电脑协作工作。",14,ink,20);text(c,"独立社区项目，与 StepX、Amoo 无官方隶属关系。GUI 执行器由 Neo Agent Touch 提供。",13,secondary,12);}
    private void backscreenPage(){LinearLayout p=page("背屏与游戏","");LinearLayout game=card(p,16);heading(game,"重力对齐",0);text(game,"三敲进入 → 左右倾斜 → 长条落入底部\n从 5 格到 1 格循环挑战，翻到手机正面退出。",14,secondary,10);gameState=text(game,"正在读取背屏状态…",14,secondary,16);button(game,"开启三敲游戏",true,()->game("standby"));button(game,"直接开始一局",false,()->game("game"));button(game,"停止并恢复背屏",false,()->game("stop"));text(game,"如未亮灯，请确认背屏朝上。更多像素动画与亮度设置可在电脑背屏模块完成。",12,secondary,12);}
    private void hideTaskKeyboard(){if(taskInput!=null){getSystemService(android.view.inputmethod.InputMethodManager.class).hideSoftInputFromWindow(taskInput.getWindowToken(),0);taskInput.clearFocus();}}
    private void hostSurface(boolean visible){if(destroyed||statusWorker.isShutdown()||!NeoAgent.selected(this))return;statusWorker.execute(()->{try{NeoAgent.call(this,new JSONObject().put("op","host_ui").put("visible",visible));}catch(Exception ignored){/* Older engines keep their overlay visible. */}});}
    private void updateGuiControls(){
        boolean active=NeoPolicy.active(guiPhase);hostSurface(resumed&&tab==3&&active);if(composeCard!=null){if(active&&composeCard.getVisibility()==View.VISIBLE){hideTaskKeyboard();if(pageScroll!=null)pageScroll.smoothScrollTo(0,0);}composeCard.setVisibility(active?View.GONE:View.VISIBLE);if(taskInput!=null)taskInput.setEnabled(!active);}
        if(taskDock!=null){taskDock.setVisibility(active?View.VISIBLE:View.GONE);if(pageScroll!=null)pageScroll.setPadding(0,0,0,active?dp(84):0);}
        Button[] buttons={guiStart,guiPause,guiResume,guiStop};
        boolean[] enabled={NeoPolicy.canStart(guiPhase,guiReady)&&!taskDraft.trim().isEmpty(),NeoPolicy.canPause(guiPhase),NeoPolicy.canResume(guiPhase),NeoPolicy.active(guiPhase)};
        for(int i=0;i<buttons.length;i++)if(buttons[i]!=null){buttons[i].setEnabled(!guiCommandPending&&enabled[i]);buttons[i].setAlpha(!guiCommandPending&&enabled[i]?1f:0.4f);}
    }
    private void guiControl(String op){
        if(!NeoPolicy.canControl(op,guiPhase)){refresh();return;}
        if(guiCommandPending){toast("正在处理上一项操作，请稍候");return;}
        if(NeoAgent.selected(this)){
            guiCommandPending=true;updateGuiControls();
            new Thread(()->{try{
                JSONObject v=NeoAgent.status(this);
                JSONObject result=GuiDelegation.desktop(this,new JSONObject().put("op","agent").put("agentOp",op).put("task",v.optString("task")));
                ui.post(()->{guiCommandPending=false;if(destroyed)return;if(op.equals("resume")&&(result.optString("phase").equals("running")||result.optString("phase").equals("queued")))moveTaskToBack(true);else refresh();});
            }catch(Exception e){ui.post(()->{guiCommandPending=false;if(!destroyed){updateGuiControls();toast(e.getMessage());}});}},"Neo-control").start();return;
        }
        GuiAccessibilityService s=GuiAccessibilityService.instance;if(s==null){toast("请先启用看屏服务");return;}
        guiCommandPending=true;
        try{java.util.concurrent.CompletableFuture<JSONObject> result=new java.util.concurrent.CompletableFuture<>();result.whenComplete((v,e)->ui.post(()->{guiCommandPending=false;if(destroyed)return;if(e!=null)toast(e.getMessage());else if(v!=null&&!v.optBoolean("ok",true))toast(v.optString("error","操作未完成"));else if(op.equals("resume")){finish();return;}refresh();}));s.request(new JSONObject().put("op","user_"+op).put("task",s.task.id),result);}catch(Exception e){guiCommandPending=false;toast(e.getMessage());}
    }
    private void guiKeyDialog(){EditText input=new EditText(this);input.setInputType(android.text.InputType.TYPE_CLASS_TEXT|android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD);input.setHint("Step Plan API Key");input.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO);new AlertDialog.Builder(this).setTitle("Step Plan 模型").setMessage("模型：step-3.7-flash\nAPI：api.stepfun.com/step_plan/v1\n密钥仅加密保存在手机，不写入 Amoo 技能或对话。").setView(input).setNegativeButton("取消",null).setPositiveButton("保存",(d,w)->{try{GuiModelVault.save(this,input.getText().toString());input.setText("");toast("密钥已加密保存");refresh();}catch(Exception e){toast(e.getMessage());}}).show();}
    private String grant(String permission){return checkSelfPermission(permission)==PackageManager.PERMISSION_GRANTED?"已授权":"未授权，可在系统权限中恢复";}
    private void notifications(){if(checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=PackageManager.PERMISSION_GRANTED)requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS},1);}
    private void game(String operation){if(!operation.equals("stop")&&checkSelfPermission("com.stepos.permission.MANAGE_BACK_SCREEN")!=PackageManager.PERMISSION_GRANTED){repairHelp("背屏控制");return;}try{if(!operation.equals("stop")){List<String> missing=new ArrayList<>();if(checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=PackageManager.PERMISSION_GRANTED)missing.add(Manifest.permission.POST_NOTIFICATIONS);if(checkSelfPermission(Manifest.permission.READ_PHONE_STATE)!=PackageManager.PERMISSION_GRANTED)missing.add(Manifest.permission.READ_PHONE_STATE);if(!missing.isEmpty())requestPermissions(missing.toArray(new String[0]),2);}startForegroundService(new Intent(this,CompanionService.class).putExtra("op",operation).putExtra("request_id","phone-"+System.currentTimeMillis()));}catch(Exception e){toast("无法启动背屏："+e.getMessage());}}
    private JSONObject state(String name){try{return TransferStore.readJson(new File(store.root(),name));}catch(Exception ignored){return new JSONObject();}}
    private void refresh(){
        if(destroyed||refreshPending)return;refreshPending=true;final int generation=pageGeneration,viewTab=tab;
        statusWorker.execute(()->{
            TransferStore.Link link=store.link();JSONObject agent=null;String error="";JSONArray history=new JSONArray();JSONObject amoo=new JSONObject(),pairing=new JSONObject(),game=new JSONObject();
            if(viewTab==0||viewTab==3)try{agent=NeoAgent.selected(this)?NeoAgent.status(this):PhoneGuiAgent.status(this);}catch(Exception e){error=e.getMessage()==null?"任务服务未返回状态，请稍后重试。":e.getMessage();}
            if(viewTab==3)try{history=AgentHistory.list(this);}catch(Exception ignored){/* A service outage must not hide readable task history. */}
            if(viewTab==0||viewTab==4||viewTab==6||viewTab==7)amoo=state("amoo-connection.json");
            if(viewTab==6||viewTab==7)pairing=state("pairing.json");if(viewTab==5)game=state("status.json");
            if(viewTab==8)try{history=AgentHistory.list(this);}catch(Exception e){history=null;error="记录读取失败，请返回后重试。";}
            final JSONObject a=agent,as=amoo,ps=pairing,gs=game;final JSONArray hs=history;final String failure=error;
            ui.post(()->{refreshPending=false;if(destroyed)return;cachedLink=link;if(generation!=pageGeneration){refresh();return;}if(viewTab!=1)endLoading();
                if(a!=null){lastAgent=a;guiReady=a.optBoolean("ready");guiPhase=a.optString("phase");}else if(viewTab==0||viewTab==3){guiReady=false;guiPhase="";}
                AutoConnectService.rememberFirstConnection(this,link.online);
                long nowElapsed=SystemClock.elapsedRealtime();
                boolean amooActive=as.optBoolean("active")&&as.optLong("leaseExpiresElapsedMs")>nowElapsed&&as.optLong("uptimeMs")<=nowElapsed&&nowElapsed-as.optLong("uptimeMs")<30000;
                if(homeStatus!=null){homeStatus.setText(link.online?(amooActive?"● 已连接 · 电脑正在使用 Amoo 协作":"● 已连接 · Amoo 与工具就绪"):AutoConnectService.enabled(this)?"尚未连接 · 自动连接待命中":"尚未连接");homeStatus.setTextColor(link.online?green:secondary);homeComputer.setText(link.online?link.name:"连接你的电脑");if(homeScan!=null)homeScan.setText(link.online?"扫码配对其他电脑":"扫码连接");if(homeHint!=null)homeHint.setText(link.online?(amooActive?"电脑正在执行 Amoo 协作（"+as.optString("message","保持唤醒")+"），文件互传与手机工具已就绪。":"已与电脑建立连接，可在电脑使用 Amoo 对话、Amoo Work、文件互传与手机工具。"):AutoConnectService.enabled(this)?"自动连接已开启 · 同一 Wi-Fi 下打开电脑端即可自动重连，也可扫码或 USB 连接。":"连接后，在电脑使用 Amoo、文件与手机工具。");homeTask.setText(a==null?(failure.isEmpty()?"暂时无法读取任务状态":failure):NeoPolicy.active(guiPhase)?phaseLabel(guiPhase)+" · "+a.optString("goal"):guiReady?"已就绪 · 无需连接电脑即可运行":readiness(a));}
                if(guiState!=null){if(a!=null)renderAgent(a,hs);else{liveCard.setVisibility(View.VISIBLE);taskBadge.setText("任务服务未就绪");taskGoal.setText("请先恢复任务服务");taskReadiness.setText("暂时无法开始任务 · 草稿已保留");taskReadiness.setTextColor(CompanionUi.AMBER);guiState.setText(failure.isEmpty()?"请稍后重试":failure);taskMetrics.setText("");taskSteps.removeAllViews();taskDetails.setVisibility(View.GONE);lastTaskRender="";renderHistory(hs,5);}updateGuiControls();}
                if(linkTitle!=null){linkTitle.setText(link.online?"已连接 · "+link.name:"尚未连接电脑");linkTitle.setTextColor(link.online?green:ink);if(connScan!=null)connScan.setText(link.online?"扫码配对其他电脑":"扫码连接");linkHint.setText(link.online?(amooActive?"电脑正在执行 Amoo 协作任务，可互传文件与调用手机工具。":"可以互传文件，在电脑使用 Amoo 与 Amoo Work。"):AutoConnectService.enabled(this)?"等待已配对电脑，请确认电脑已打开 StepX Companion。":"首次使用请扫码或通过 USB 配对。");connectionFoot.setText(link.online?(amooActive?"电脑已连接 · Amoo 后台协作进行中（"+as.optString("message","保持唤醒")+"）":"连接状态由电脑实时确认 · 支持 USB 与无线热备切换"):AutoConnectService.state(this));}
                if(send!=null){send.setEnabled(!busy);send.setAlpha(!busy?1f:0.45f);send.setText(busy?"正在准备文件…":link.online?"选择文件，发送到电脑":"连接电脑");transferHint.setText(link.online?"发送给 "+link.name:"请先连接电脑，再选择文件。");loadTransfers();}
                if(viewTab==6||viewTab==7)renderPermissions(as,ps);
                if(viewTab==8){if(hs!=null)renderHistory(hs,20);else if(taskHistory.getChildCount()==0)text(taskHistory,failure,14,secondary,12);}
                if(gameState!=null&&checkSelfPermission("com.stepos.permission.MANAGE_BACK_SCREEN")!=PackageManager.PERMISSION_GRANTED){gameState.setText(bootRecovering()?"背屏权限正在恢复 · 请稍候…":"背屏权限未就绪 · 点击开始可查看修复方式。");}else if(gameState!=null){String mode=gs.optString("mode");gameState.setText((mode.equals("standby")?"三敲监听已开启":mode.equals("game")?"游戏进行中 · "+gs.optLong("score")+" 分":mode.equals("animation")?"正在显示："+gs.optString("appliedTitle"):"游戏未启动")+"\n最高积分 "+gs.optLong("bestScore")+" 分"+(gs.optString("pausedReason").isEmpty()?"":"\n"+gs.optString("pausedReason"))+(gs.optString("lastError").isEmpty()?"":"\n"+gs.optString("lastError")));}
            });
        });
    }
    private boolean screenPermissionEnabled(){String services=Settings.Secure.getString(getContentResolver(),Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES);if(services==null)return false;String target=NeoAgent.selected(this)?"com.stepx.companion.neo.ControlService":getPackageName()+".GuiAccessibilityService";for(String service:services.split(":")){ComponentName c=ComponentName.unflattenFromString(service);if(c!=null&&getPackageName().equals(c.getPackageName())&&target.equals(c.getClassName()))return true;}return false;}
    private String readiness(JSONObject value){if(value.optBoolean("ready"))return "保持手机解锁 · 使用你配置的模型服务";if(!value.optBoolean("executorConnected"))return screenPermissionEnabled()?"看屏服务正在重新连接…":"请在设置 → 权限与服务中启用看屏操作。";if(!value.optBoolean("modelConfigured"))return "请先配置支持图片输入的模型。";return "请在模型设置中确认允许操作的应用。";}
    private String phaseLabel(String phase){return NeoPolicy.phaseLabel(phase);}
    private void renderAgent(JSONObject value,JSONArray history){
        JSONObject result=value.optJSONObject("result");boolean verified=result!=null&&result.optBoolean("verified")&&result.optBoolean("phoneStopped");
        liveCard.setVisibility(NeoPolicy.active(guiPhase)?View.VISIBLE:View.GONE);taskBadge.setText(phaseLabel(guiPhase));taskBadge.setTextColor(verified?green:guiPhase.equals("reported")||guiPhase.equals("needs_input")?CompanionUi.AMBER:ink);
        taskGoal.setText(value.optString("goal").isEmpty()?"还没有运行中的任务":value.optString("goal"));
        String info=value.optString("progress");String readiness=readiness(value);taskReadiness.setText(readiness);taskReadiness.setTextColor(guiReady?secondary:CompanionUi.AMBER);
        if(verified)info="执行结束时已核对目标应用与页面，结果验证通过。";
        if(guiPhase.equals("reported"))info="执行已结束，模型报告完成；尚无足够的屏幕证据确认结果。\n"+info;
        if(info.isEmpty())info=guiReady?"模型与看屏权限已就绪，输入目标即可开始。":"请先完成模型与权限配置。";
        guiState.setText(info);taskMetrics.setText(value.optString("model","未配置模型")+"  ·  "+value.optInt("actions")+" 次操作 / "+value.optInt("screenshots")+" 次看屏");
        if(guiStart!=null)guiStart.setText(guiCommandPending?"正在提交…":"开始任务");taskDetails.setVisibility(value.optString("goal").isEmpty()?View.GONE:View.VISIBLE);
        String signature=value.optString("workflow");if(!signature.equals(lastTaskRender)){lastTaskRender=signature;taskSteps.removeAllViews();JSONObject workflow=value.optJSONObject("workflow");JSONArray steps=workflow==null?null:workflow.optJSONArray("tasks");if(steps!=null&&steps.length()>1)for(int i=0;i<Math.min(steps.length(),3);i++){JSONObject step=steps.optJSONObject(i);if(step==null)continue;TextView t=text(taskSteps,(i+1)+". "+step.optString("app")+" · "+phaseLabel(verified&&steps.length()==1?"completed":step.optString("state"))+"\n"+step.optString("goal"),13,ink,12);t.setTextIsSelectable(true);}}
        if(historyTitle!=null)historyTitle.setText(NeoPolicy.active(guiPhase)?"之前的任务":"最近任务");
        renderHistory(history,5);
    }
    private void renderHistory(JSONArray history,int limit){
        String signature=history.toString()+limit;if(signature.equals(lastHistoryRender))return;lastHistoryRender=signature;taskHistory.removeAllViews();
        if(history.length()==0){LinearLayout empty=card(taskHistory,8);text(empty,"还没有任务记录",15,ink,0);text(empty,"完成任务后，可在这里查看结果或再次编辑。",13,secondary,8);return;}
        LinearLayout list=group(taskHistory,"");for(int i=0;i<Math.min(history.length(),limit);i++){JSONObject entry=history.optJSONObject(i);if(entry==null)continue;String when=android.text.format.DateFormat.format("MM-dd HH:mm",entry.optLong("createdAt")).toString();row(list,UiGlyph.TASK,entry.optString("goal","手机任务"),phaseLabel(entry.optString("phase"))+" · "+when,()->taskDetail(entry));}
    }
    private void renderPermissions(JSONObject s,JSONObject pairing){
        boolean permissions=AmooConnectionService.permissionsGranted(this),supported=s.optBoolean("supported",permissions)&&permissions,enabled=getSharedPreferences("amoo_connection",MODE_PRIVATE).getBoolean("enabled",true);if(amooSwitch!=null){updatingSwitch=true;amooSwitch.setChecked(supported&&enabled);amooSwitch.setEnabled(supported);updatingSwitch=false;}
        long elapsed=SystemClock.elapsedRealtime();boolean fresh=s.optBoolean("active")&&s.optLong("leaseExpiresElapsedMs")>elapsed&&s.optLong("uptimeMs")<=elapsed&&elapsed-s.optLong("uptimeMs")<30000;
        String error=s.optString("lastError");if(amooState!=null)amooState.setText(!permissions?(bootRecovering()?"正在恢复熄屏协作权限…":"后台协作权限未就绪，请在权限与服务中查看详情。"):!error.isEmpty()?error:!supported?"后台协作未就绪，请在电脑检查固件适配。":!enabled?"已停止后台协作":fresh?s.optString("message","协作进行中"):cachedLink.online?"已连接 "+cachedLink.name+" · 后台协作就绪，电脑发起 Amoo 任务时自动保持唤醒":"已允许 · 等待电脑连接");
        PowerManager power=getSystemService(PowerManager.class);NotificationManager notifications=getSystemService(NotificationManager.class);
        boolean pairingReady=PairingBridgeProvider.available()&&pairing.optBoolean("pairingSupported")&&System.currentTimeMillis()-pairing.optLong("updatedAtMs")<30000;
        labelPermission("camera",grant(Manifest.permission.CAMERA));labelPermission("notice",notifications.areNotificationsEnabled()?"已开启":"未开启 · 点击管理");
        labelPermission("screen",checkSelfPermission("com.stepos.permission.MANAGE_BACK_SCREEN")==PackageManager.PERMISSION_GRANTED?"已适配":"待修复 · 查看电脑操作步骤");labelPermission("pairing",pairingReady?"已就绪":"待连接确认 · 查看修复步骤");labelPermission("power",power.isIgnoringBatteryOptimizations(getPackageName())?"不受限制":"受系统管理 · 点击调整");
        String accessibility=Settings.Secure.getString(getContentResolver(),Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES);labelPermission("gui",screenPermissionEnabled()?"已开启":"未开启 · 点击授权");
        String listeners=Settings.Secure.getString(getContentResolver(),"enabled_notification_listeners");labelPermission("notifications",listeners!=null&&listeners.contains(getPackageName())?"已开启":"未开启 · 点击授权");

    }
    private void labelPermission(String id,String value){TextView label=permissionLabels.get(id);if(label!=null)label.setText(value);}
    private void chooseFiles(){selectedComputer=cachedLink;if(!selectedComputer.online){showTab(4);return;}Intent i=new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*").putExtra(Intent.EXTRA_ALLOW_MULTIPLE,true);startActivityForResult(i,PICK_FILES);}
    private void loadTransfers(){if(listing||transferList==null||destroyed)return;listing=true;final int generation=pageGeneration;files.execute(()->{try{List<TransferStore.Entry> entries=store.list();StringBuilder key=new StringBuilder();for(TransferStore.Entry e:entries)key.append(e.id).append(e.status).append(e.error);String signature=key.toString();ui.post(()->{listing=false;if(transferList==null||destroyed)return;if(generation!=pageGeneration){loadTransfers();return;}endLoading();updateSendNotice(entries);if(lastListing.equals(signature)&&transferList.getChildCount()>0)return;lastListing=signature;renderTransfers(entries);});}catch(Exception e){ui.post(()->{listing=false;if(generation!=pageGeneration||destroyed)return;endLoading();if(transferList!=null&&transferList.getChildCount()==0)text(transferList,"暂时无法读取文件："+e.getMessage(),14,secondary,24);});}});}
    private void updateSendNotice(List<TransferStore.Entry> entries){
        if(notice==null||transferDirection!=1||busy)return;
        int received=0,queued=0,failed=0;
        for(TransferStore.Entry e:entries){if(!e.direction.equals("outbox"))continue;if("received".equals(e.status))received++;else if("error".equals(e.status))failed++;else queued++;}
        String summary=received+queued+failed==0?"":"累计已接收 "+received+" 个文件"+(queued>0?" · 等待接收 "+queued+" 个":"")+(failed>0?" · 传输失败 "+failed+" 个":"");
        if(!enqueueError.isEmpty())summary+=(summary.isEmpty()?"":"\n")+enqueueError;
        notice.setText(summary);notice.setTextColor(queued==0&&failed==0&&enqueueError.isEmpty()?green:secondary);
    }
    private void finishQueued(int sent,List<String> errors){
        ui.post(()->{busy=false;if(destroyed)return;enqueueError=errors.isEmpty()?"":errors.size()+" 个文件未能发送："+errors.get(0);
            if(sent>0){transferDirection=1;if(tab==1)showTab(1);if(notice!=null){notice.setText("已提交 "+sent+" 个文件，正在确认电脑接收状态…");notice.setTextColor(secondary);}else toast("已提交 "+sent+" 个文件，可在发出的文件中查看进度。");}
            else if(notice!=null)notice.setText(enqueueError);else toast(enqueueError);
            lastListing="";refresh();loadTransfers();
        });
    }
    private void renderTransfers(List<TransferStore.Entry> entries){transferList.removeAllViews();int count=0;for(TransferStore.Entry e:entries){if(!e.direction.equals(transferDirection==0?"inbox":"outbox"))continue;if(++count>100)break;LinearLayout row=card(transferList,12);TextView filename=text(row,e.name,16,ink,0);filename.setTypeface(CompanionUi.MEDIUM);String time=android.text.format.DateFormat.format("MM-dd HH:mm",e.createdAt).toString();text(row,TransferStore.size(e.size)+" · "+time,12,secondary,6);String status=e.direction.equals("inbox")?"来自 "+e.computer:"received".equals(e.status)?"电脑已接收":"error".equals(e.status)?"传输失败："+e.error:"等待 "+e.computer+" 接收";text(row,status,13,"received".equals(e.status)?green:secondary,6);
            boolean isImg=ExpansionActivity.isImageName(e.name);
            if(e.direction.equals("inbox")||isImg){LinearLayout actions=new LinearLayout(this);row.addView(actions);if(isImg)twoButton(actions,"放大预览",UiGlyph.IMAGE,false,()->previewTransferImage(e));if(e.direction.equals("inbox")){twoButton(actions,"打开",UiGlyph.FOLDER,false,()->open(e));twoButton(actions,"保存到手机",UiGlyph.UPLOAD,false,()->save(e));}}TextView more=text(row,"移除记录",12,secondary,6);more.setMinHeight(dp(44));more.setGravity(Gravity.CENTER_VERTICAL);more.setOnClickListener(v->confirmDelete(e));}
        if(count==0){LinearLayout empty=card(transferList,16);heading(empty,transferDirection==0?"还没有收到文件":"还没有发出文件",0);text(empty,transferDirection==0?"在 desktop 的 StepX伴侣中选择“发送文件到手机”，接收后会出现在这里。":"连接电脑后，选择上方的文件发送按钮。电脑确认收到后，会显示接收状态。",14,secondary,10);}
    }
    private void previewTransferImage(TransferStore.Entry e){
        if(busy)return;
        files.execute(()->{try{
            File f=store.verifiedFile(e.direction,e.id);
            ExpansionActivity.DecodedImage dec=ExpansionActivity.decodeSampled(f);
            if(dec==null)throw new IOException("无法解码该图片格式");
            ui.post(()->{if(destroyed)return;
                Dialog dialog=new Dialog(this);ScrollView scroll=new ScrollView(this);LinearLayout box=column();box.setPadding(dp(20),dp(20),dp(20),dp(20));box.setBackground(surface(Color.WHITE,line,12));scroll.addView(box);
                heading(box,e.name,0);text(box,TransferStore.size(e.size)+" · "+(e.direction.equals("inbox")?"来自 "+e.computer:"发往 "+e.computer),13,secondary,6);
                TextView meta=text(box,"支持双指缩放 · 双击 2.5x · 拖动查看细节",12,secondary,8);
                ExpansionActivity.ZoomableImageView zv=ExpansionActivity.addZoomBox(this,design,box,280,meta,"已校验原图");
                zv.setImage(dec);
                if(e.direction.equals("inbox")){button(box,"外部应用打开",true,()->{dialog.dismiss();open(e);});button(box,"保存到手机",false,()->{dialog.dismiss();save(e);});}
                button(box,"关闭",false,dialog::dismiss);
                dialog.setContentView(scroll);dialog.getWindow().setBackgroundDrawableResource(android.R.color.transparent);dialog.show();dialog.getWindow().setLayout(getResources().getDisplayMetrics().widthPixels-dp(32),-2);
            });
        }catch(Exception ex){ui.post(()->toast("预览失败："+ex.getMessage()));}});
    }
    private void open(TransferStore.Entry e){if(busy)return;busy=true;refresh();if(notice!=null)notice.setText("正在校验文件…");files.execute(()->{try{store.verifiedFile(e.direction,e.id);ui.post(()->{busy=false;refresh();if(destroyed)return;try{Intent i=new Intent(Intent.ACTION_VIEW).setDataAndType(e.uri(),TransferStore.mime(e.name)).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);i.setClipData(ClipData.newRawUri(e.name,e.uri()));startActivity(Intent.createChooser(i,"打开文件"));if(notice!=null)notice.setText("文件校验通过。");}catch(ActivityNotFoundException ex){toast("没有可以打开此格式的应用，请先保存到手机。");}});}catch(Exception ex){finishFiles("打开失败："+ex.getMessage());}});}
    private void save(TransferStore.Entry e){saving=e;Intent i=new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType(TransferStore.mime(e.name)).putExtra(Intent.EXTRA_TITLE,e.name);startActivityForResult(i,SAVE_FILE);}
    private void confirmDelete(TransferStore.Entry e){new AlertDialog.Builder(this).setTitle("移除这条传输？").setMessage("将删除伴侣中的传输记录和缓存文件。已经保存到手机或电脑的文件不受影响。").setNegativeButton("保留",null).setPositiveButton("移除",(d,w)->files.execute(()->{try{store.delete(e);ui.post(()->{lastListing="";loadTransfers();});}catch(Exception ex){ui.post(()->toast("未能移除："+ex.getMessage()));}})).show();}
    @Override protected void onActivityResult(int request,int result,Intent data){super.onActivityResult(request,result,data);if(result!=RESULT_OK||data==null)return;
        if(request==PICK_FILES){List<Uri> uris=new ArrayList<>();ClipData clips=data.getClipData();if(clips!=null){if(clips.getItemCount()>10){toast("一次最多选择 10 个文件，请重新选择。");return;}for(int i=0;i<clips.getItemCount();i++){Uri u=clips.getItemAt(i).getUri();if(u!=null&&!uris.contains(u))uris.add(u);}}else if(data.getData()!=null)uris.add(data.getData());if(uris.isEmpty())return;TransferStore.Link target=selectedComputer;if(target==null){toast("电脑连接已变化，请重新选择文件。");return;}busy=true;enqueueError="";refresh();if(notice!=null){notice.setText("正在准备 "+uris.size()+" 个文件…");notice.setTextColor(secondary);}files.execute(()->{int sent=0;List<String> errors=new ArrayList<>();for(Uri u:uris){try{store.enqueue(u,target);sent++;}catch(Exception ex){errors.add(ex.getMessage());}}finishQueued(sent,errors);});
        }else if(request==SAVE_FILE&&data.getData()!=null&&saving!=null){TransferStore.Entry entry=saving;saving=null;Uri destination=data.getData();busy=true;refresh();files.execute(()->{try{store.save(entry,destination);finishFiles("已保存到你选择的位置。");}catch(Exception e){finishFiles("保存失败："+e.getMessage());}});}
    }
    private void finishFiles(String message){ui.post(()->{busy=false;if(destroyed)return;refresh();if(notice!=null)notice.setText(message);else toast(message);lastListing="";loadTransfers();});}
    private void toast(String message){if(!destroyed)Toast.makeText(this,message,Toast.LENGTH_LONG).show();}
    @Override public void onBackPressed(){if(tab>=4)showTab(parentTab(tab));else if(tab!=0)showTab(0);else super.onBackPressed();}
    private final Runnable tick=new Runnable(){public void run(){if(resumed){refresh();ui.postDelayed(this,2000);}}};
    @Override protected void onResume(){super.onResume();resumed=true;AutoConnectService.resume(this);ui.removeCallbacks(tick);ui.post(tick);}
    @Override protected void onPause(){endLoading();CompanionMotion.stop(content);CompanionMotion.stop(pageScroll);rememberDraft();hostSurface(false);resumed=false;ui.removeCallbacks(tick);super.onPause();}
    @Override protected void onSaveInstanceState(Bundle out){super.onSaveInstanceState(out);out.putInt("tab",tab);out.putInt("historyOrigin",historyOrigin);out.putInt("connectionOrigin",connectionOrigin);out.putString("taskDraft",taskDraft);out.putInt("direction",transferDirection);if(saving!=null){out.putString("savingId",saving.id);out.putString("savingDirection",saving.direction);}if(selectedComputer!=null){out.putString("targetId",selectedComputer.id);out.putString("targetName",selectedComputer.name);}}
    @Override protected void onDestroy(){destroyed=true;ui.removeCallbacksAndMessages(null);files.shutdown();statusWorker.shutdown();super.onDestroy();}
}
