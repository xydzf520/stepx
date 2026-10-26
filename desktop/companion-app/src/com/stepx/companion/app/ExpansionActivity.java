package com.stepx.companion.app;
import android.app.*;
import android.content.*;
import android.net.Uri;
import android.os.*;
import android.provider.*;
import android.view.*;
import android.widget.*;
import org.json.*;
import java.text.SimpleDateFormat;
import java.util.*;
import java.util.concurrent.*;

/** Settings and library stay inside the existing companion application. */
public final class ExpansionActivity extends Activity {
 private CompanionUi ui;private LinearLayout body;private final Handler handler=new Handler(Looper.getMainLooper());private final ExecutorService worker=Executors.newSingleThreadExecutor();private JSONObject status=new JSONObject();private JSONArray items=new JSONArray();private String kind="all",sort="date",filter="all";private boolean descending=true,settings=false,busy=false;private int limit=40,offset=0;private ScrollView pageScroll;private android.database.ContentObserver observer;private String notice="";private boolean shortcutOpened=false;private CheckBox photos,videos,documents;private Spinner retention;private long generation=0;
 private JSONObject draft;private LinearLayout root;private boolean firstRender=true;private ProgressBar loadingLine;private Runnable loadingReveal;private String searchQuery="";private LinearLayout libraryList;
 private final android.util.LruCache<String,android.graphics.Bitmap> thumbCache=new android.util.LruCache<>(64);
 private volatile int listGeneration=0;
 private void stopLoading(){if(loadingReveal!=null)handler.removeCallbacks(loadingReveal);if(loadingLine!=null)loadingLine.setVisibility(View.INVISIBLE);}
 private static final String[] KINDS={"all","photos","videos","documents"},LABELS={"全部","照片","视频","文档"},SORTS={"date","name","size"};
 public void onCreate(Bundle saved){super.onCreate(saved);ui=new CompanionUi(this);if(saved!=null&&saved.containsKey("draft")){try{draft=new JSONObject(saved.getString("draft","{}"));}catch(Exception ignored){}}android.content.SharedPreferences p=getSharedPreferences("expansion-ui",0);kind=p.getString("kind","all");sort=p.getString("sort","date");descending=p.getBoolean("descending",true);settings=getIntent().getBooleanExtra("settings",false);observer=new android.database.ContentObserver(handler){public void onChange(boolean self){if(!settings&&!busy){handler.removeCallbacks(refresh);handler.postDelayed(refresh,400);}}};getContentResolver().registerContentObserver(DocumentsContract.buildChildDocumentsUri(ExpansionStore.DOCS,"root"),true,observer);render();}
 public void onResume(){super.onResume();if(ui!=null){captureDraft();load();}}
 private final Runnable refresh=()->{if(!isDestroyed()&&!busy)load();};
 public void onPause(){stopLoading();CompanionMotion.stop(body);handler.removeCallbacks(refresh);super.onPause();}
 public void onDestroy(){if(observer!=null)getContentResolver().unregisterContentObserver(observer);generation++;handler.removeCallbacksAndMessages(null);worker.shutdownNow();super.onDestroy();}
 private void load(){long g=++generation;stopLoading();loadingReveal=()->{if(g==generation&&loadingLine!=null)loadingLine.setVisibility(View.VISIBLE);};handler.postDelayed(loadingReveal,150);worker.execute(()->{try{ExpansionStore s=new ExpansionStore(this);JSONObject next=s.status();if(next.optBoolean("permission"))try{next.put("inspection",s.inspect(null));}catch(Exception ignored){}JSONArray list=s.list(kind,sort,descending,offset,limit,filter);handler.post(()->{if(isDestroyed()||g!=generation)return;stopLoading();status=next;items=list;render();String id=getIntent().getStringExtra("fileId");if(!shortcutOpened&&id!=null){shortcutOpened=true;worker.execute(()->{try{JSONObject record=new ExpansionStore(this).get(id);handler.post(()->{if(!isDestroyed())details(record);});}catch(Exception ignored){}});}});}catch(Exception e){handler.post(()->{if(!isDestroyed()&&g==generation){stopLoading();notice=e.getMessage();render();}});}});}
 private void saveSort(){getSharedPreferences("expansion-ui",0).edit().putString("kind",kind).putString("sort",sort).putBoolean("descending",descending).apply();load();}
 static String bytes(long n){if(n>=1073741824L)return String.format(Locale.ROOT,"%.1f GB",n/1073741824.0);if(n>=1048576)return String.format(Locale.ROOT,"%.1f MB",n/1048576.0);return String.format(Locale.ROOT,"%.1f KB",n/1024.0);}
 private JSONObject policy(){JSONObject p=status.optJSONObject("policy");return p==null?new JSONObject():p;}
 private void render(){
  int previousY=pageScroll==null?0:pageScroll.getScrollY();
  root=ui.column();root.setBackgroundColor(CompanionUi.BG);setContentView(root);ui.bars(root);ui.back(root,"存储扩容",()->leave(this::finish));loadingLine=CompanionMotion.progress(this);root.addView(loadingLine,new LinearLayout.LayoutParams(-1,ui.dp(3)));ScrollView scroll=new ScrollView(this);pageScroll=scroll;root.addView(scroll,new LinearLayout.LayoutParams(-1,0,1));body=ui.column();body.setPadding(ui.dp(20),ui.dp(4),ui.dp(20),ui.dp(28));scroll.addView(body);
  LinearLayout tabs=new LinearLayout(this);for(String label:new String[]{"文件库","自动迁移"}){boolean active=settings==label.equals("自动迁移");Button b=new Button(this);b.setText(label);ui.styleButton(b,active);tabs.addView(b,new LinearLayout.LayoutParams(0,ui.dp(48),1));b.setOnClickListener(v->leave(()->{settings=label.equals("自动迁移");notice="";draft=null;load();}));}body.addView(tabs);
  if(status.length()==0){ui.text(body,"正在读取文件库…",15,CompanionUi.MUTED,24);if(!notice.isEmpty())ui.text(body,notice,13,CompanionUi.AMBER,12);return;}
  LinearLayout hero=ui.card(body,16);
  if(settings){ui.title(hero,policy().optBoolean("enabled")?"当前：自动迁移已开启":"当前：自动迁移未开启",17,0);ui.text(hero,"这里显示已生效的状态，下方选项需要确认后保存。",13,CompanionUi.MUTED,8);}
  else{ui.text(hero,"已释放空间",13,CompanionUi.MUTED,0);ui.title(hero,bytes(status.optLong("releasedBytes")),32,6);}
  ui.text(hero,status.optLong("count")+" 个文件保存在电脑 · "+(status.optBoolean("online")?"电脑在线":"电脑离线"),13,status.optBoolean("online")?CompanionUi.GREEN:CompanionUi.MUTED,8);
  JSONObject report=status.optJSONObject("inspection");if(!settings&&report!=null){LinearLayout check=ui.card(body,12);ui.title(check,policy().optBoolean("enabled")?"自动迁移已开启":"自动迁移未开启",17,0);ui.text(check,"可迁移 "+report.optInt("candidateCount")+" 个 · 保留期内 "+(report.optInt("retainedCount")+report.optInt("writingCount"))+" 个",14,CompanionUi.INK,10);if(report.optLong("nextEligibleAt")>0)ui.text(check,"最早一批："+new SimpleDateFormat("MM-dd HH:mm",Locale.CHINA).format(new Date(report.optLong("nextEligibleAt")))+" 达到迁移时间，届时请连接电脑。",13,CompanionUi.MUTED,8);ui.text(check,"保留最近 7 天 / 30 天：只迁移更早的文件。更改规则后需要确认才会生效。",13,CompanionUi.MUTED,8);}
  if(!notice.isEmpty())ui.text(body,notice,13,CompanionUi.AMBER,14);
  if(settings){settings();saveBar();}else library();scroll.post(()->scroll.scrollTo(0,previousY));if(firstRender){firstRender=false;CompanionMotion.enter(body);}
 }
 private CheckBox choice(LinearLayout p,String title,boolean checked){CheckBox c=new CheckBox(this);c.setText(title);c.setTextColor(CompanionUi.INK);c.setTextSize(15);c.setChecked(checked);c.setMinHeight(ui.dp(52));c.setButtonTintList(android.content.res.ColorStateList.valueOf(CompanionUi.INK));p.addView(c);return c;}
 private void settings(){JSONObject p=draft==null?policy():draft,host=status.optJSONObject("host");LinearLayout card=ui.card(body,16);ui.title(card,"1  选择迁移范围",18,0);ui.text(card,"勾选仅修改草稿。点击底部“查看变更”并确认后，才会更新手机迁移规则。",13,CompanionUi.MUTED,8);
  photos=choice(card,"拍摄照片 · DCIM 根目录与 Camera",p.optBoolean("photos",true));videos=choice(card,"拍摄视频 · DCIM 根目录与 Camera",p.optBoolean("videos"));documents=choice(card,"文档 · Documents / Download",p.optBoolean("documents"));
  ui.text(card,"2  什么时候迁移",13,CompanionUi.MUTED,12);retention=new Spinner(this);retention.setAdapter(new ArrayAdapter<>(this,android.R.layout.simple_spinner_dropdown_item,new String[]{"写入完成后迁移（至少 2 分钟）","保留最近 7 天","保留最近 30 天"}));int d=p.optInt("retentionDays",7);retention.setSelection(d==0?0:d==7?1:2);card.addView(retention,new LinearLayout.LayoutParams(-1,ui.dp(48)));
  LinearLayout dest=ui.card(body,16);ui.row(dest,UiGlyph.COMPUTER,host==null?"连接电脑后设置":host.optString("name","已连接电脑"),host==null?"在 desktop 左侧「存储扩容」选择保存文件夹":host.optString("folder","在 desktop 选择保存文件夹"),null);
  if(!status.optBoolean("permission"))ui.button(dest,"允许文件管理",false,()->{try{startActivity(new Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,Uri.parse("package:"+getPackageName())));}catch(Exception e){startActivity(new Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION));}});
  ui.text(body,"传输 → 双端校验 → 保留文件入口 → 清理手机原件",13,CompanionUi.INK,20);ui.text(body,"电脑保留原件，请妥善保管保存目录。手机文件库和系统文件选择器保留入口；离线时可以看缩略图，打开原件需要连接原电脑。相册保留带标记的预览图；从相册分享至「取回电脑原图」即可取回，无需进入文件库。单击预览图不会自动下载原件。",13,CompanionUi.MUTED,8);
  if(policy().optBoolean("enabled"))ui.button(body,"暂停当前自动迁移",false,()->new AlertDialog.Builder(this).setTitle("暂停自动迁移？").setMessage("停止开始新文件迁移。已迁移的电脑原件与手机入口保留，正在校验的单个文件可能先完成。").setNegativeButton("取消",null).setPositiveButton("确认暂停",(x,w)->configure(false)).show());
  ui.button(body,"清理手机查看缓存",false,()->new AlertDialog.Builder(this).setTitle("清理查看缓存？").setMessage("仅清除临时取回的缓存，电脑原件保留。清理后，离线时无法查看尚未再次缓存的原件。").setNegativeButton("取消",null).setPositiveButton("清理缓存",(dialogWhich,w)->task(()->{new ExpansionStore(this).clearCache();return "查看缓存已清理，电脑原件保留";})).show());
 }
 private void captureDraft(){if(!settings||photos==null||retention==null)return;try{draft=new JSONObject().put("photos",photos.isChecked()).put("videos",videos.isChecked()).put("documents",documents.isChecked()).put("retentionDays",new int[]{0,7,30}[retention.getSelectedItemPosition()]);}catch(Exception ignored){}}
 private boolean dirty(){captureDraft();if(draft==null)return false;JSONObject p=policy();return draft.optBoolean("photos")!=p.optBoolean("photos",true)||draft.optBoolean("videos")!=p.optBoolean("videos")||draft.optBoolean("documents")!=p.optBoolean("documents")||draft.optInt("retentionDays",7)!=p.optInt("retentionDays",7);}
 private void leave(Runnable next){if(busy){Toast.makeText(this,"正在保存，请等待完成",Toast.LENGTH_SHORT).show();return;}if(!dirty()){next.run();return;}new AlertDialog.Builder(this).setTitle("更改还未保存").setMessage("离开会放弃当前更改，手机继续使用原来的迁移规则。").setNegativeButton("继续编辑",null).setPositiveButton("放弃并离开",(d,w)->{draft=null;next.run();}).show();}
 @Override public void onBackPressed(){leave(()->super.onBackPressed());}
 @Override protected void onSaveInstanceState(Bundle b){captureDraft();if(draft!=null)b.putString("draft",draft.toString());super.onSaveInstanceState(b);}
 private void saveBar(){LinearLayout bar=ui.column();bar.setPadding(ui.dp(20),ui.dp(8),ui.dp(20),ui.dp(12));bar.setBackgroundColor(android.graphics.Color.WHITE);ui.text(bar,busy?"正在保存并核对手机…":"选择不会立即生效，请确认后保存",12,CompanionUi.MUTED,0);ui.button(bar,"查看变更并确认",true,this::review).setEnabled(!busy);root.addView(bar,new LinearLayout.LayoutParams(-1,-2));}
 private void review(){if(busy)return;captureDraft();if(!photos.isChecked()&&!videos.isChecked()&&!documents.isChecked()){Toast.makeText(this,"请至少选择一种文件",Toast.LENGTH_SHORT).show();return;}JSONObject h=status.optJSONObject("host");if(h==null||!status.optBoolean("online")||!h.optBoolean("ready")){Toast.makeText(this,"请先连接电脑并选择保存位置",Toast.LENGTH_LONG).show();return;}String types=(photos.isChecked()?"拍摄照片 ":"")+(videos.isChecked()?"拍摄视频 ":"")+(documents.isChecked()?"文档":"");int days=draft.optInt("retentionDays",7);new AlertDialog.Builder(this).setTitle("开启迁移并清理手机原件？").setMessage("范围："+types+"\n保留规则："+(days==0?"全部迁移":"手机保留最近 "+days+" 天")+"\n电脑："+h.optString("name")+"\n保存到："+h.optString("folder")+" / StepX Storage\n\n确认后生效。连接时自动复制并校验，成功后清理手机原件，保留入口和预览。离线时只可查看已缓存的内容。").setNegativeButton("返回编辑",null).setPositiveButton("确认并开启",(x,w)->configure(true)).show();}

 private void configure(boolean enabled){try{captureDraft();JSONObject h=status.optJSONObject("host");JSONObject q=new JSONObject((enabled?draft:policy()).toString()).put("enabled",enabled).put("desktopId",h==null?policy().optString("desktopId"):h.optString("desktopId"));task(()->{ExpansionStore store=new ExpansionStore(this);store.configure(q);JSONObject result=store.status().getJSONObject("policy");for(String key:new String[]{"enabled","photos","videos","documents","retentionDays","desktopId"})if(!String.valueOf(q.opt(key)).equals(String.valueOf(result.opt(key))))throw new java.io.IOException("手机设置回读不一致，请重试");draft=null;return enabled?"已保存并核对，连接电脑时自动迁移":"已确认暂停，电脑原件和手机入口保留";});}catch(Exception e){notice="保存失败："+e.getMessage();render();}}
 private void library(){
  HorizontalScrollView sc=new HorizontalScrollView(this);LinearLayout chips=new LinearLayout(this);for(int i=0;i<KINDS.length;i++){final String k=KINDS[i];Button b=new Button(this);b.setText(LABELS[i]);ui.styleButton(b,kind.equals(k));chips.addView(b,new LinearLayout.LayoutParams(ui.dp(80),ui.dp(46)));b.setOnClickListener(v->{kind=k;offset=0;pageScroll=null;saveSort();});}sc.addView(chips);LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.topMargin=ui.dp(16);body.addView(sc,lp);
  LinearLayout order=new LinearLayout(this);Spinner sorts=new Spinner(this);sorts.setAdapter(new ArrayAdapter<>(this,android.R.layout.simple_spinner_dropdown_item,new String[]{"按日期","按名称","按大小"}));sorts.setSelection(Arrays.asList(SORTS).indexOf(sort)<0?0:Arrays.asList(SORTS).indexOf(sort));order.addView(sorts,new LinearLayout.LayoutParams(0,ui.dp(48),1));sorts.setOnItemSelectedListener(new android.widget.AdapterView.OnItemSelectedListener(){public void onNothingSelected(android.widget.AdapterView<?> p){}public void onItemSelected(android.widget.AdapterView<?> p,View v,int pos,long id){if(!sort.equals(SORTS[pos])){sort=SORTS[pos];saveSort();}}});Button dir=new Button(this);dir.setText(descending?"降序 ↓":"升序 ↑");ui.styleButton(dir,false);order.addView(dir);dir.setOnClickListener(v->{descending=!descending;saveSort();});body.addView(order);
  Spinner states=new Spinner(this);String[] ids={"all","archived","restored","prepared","failed"};states.setAdapter(new ArrayAdapter<>(this,android.R.layout.simple_spinner_dropdown_item,new String[]{"全部状态","已迁移到电脑","已恢复到手机","待完成迁移","迁移失败"}));states.setSelection(Math.max(0,Arrays.asList(ids).indexOf(filter)));body.addView(states,new LinearLayout.LayoutParams(-1,ui.dp(48)));states.setOnItemSelectedListener(new android.widget.AdapterView.OnItemSelectedListener(){public void onNothingSelected(android.widget.AdapterView<?> p){}public void onItemSelected(android.widget.AdapterView<?> p,View v,int pos,long id){if(!filter.equals(ids[pos])){filter=ids[pos];offset=0;pageScroll=null;load();}}});
  EditText search=new EditText(this);search.setHint("搜索文件名或路径…");search.setSingleLine(true);search.setTextSize(14);search.setTextColor(CompanionUi.INK);search.setHintTextColor(CompanionUi.MUTED);search.setPadding(ui.dp(12),ui.dp(10),ui.dp(12),ui.dp(10));search.setBackground(ui.shape(android.graphics.Color.WHITE,CompanionUi.LINE,8));search.setText(searchQuery);LinearLayout.LayoutParams sp=new LinearLayout.LayoutParams(-1,ui.dp(46));sp.topMargin=ui.dp(8);body.addView(search,sp);
  libraryList=ui.column();body.addView(libraryList,new LinearLayout.LayoutParams(-1,-2));renderLibraryItems();
  search.addTextChangedListener(new android.text.TextWatcher(){public void beforeTextChanged(CharSequence s,int a,int b,int c){}public void onTextChanged(CharSequence s,int a,int b,int c){}public void afterTextChanged(android.text.Editable e){searchQuery=e==null?"":e.toString().trim();renderLibraryItems();}});
  if(offset>0)ui.button(body,"上一页",false,()->{offset=Math.max(0,offset-limit);pageScroll=null;load();});if(items.length()>=limit)ui.button(body,"下一页",false,()->{offset+=limit;pageScroll=null;load();});ui.button(body,"刷新文件库",false,this::load);
 }
 private void renderLibraryItems(){
  if(libraryList==null)return;libraryList.removeAllViews();
  int listGen=++listGeneration;
  if(items.length()==0){LinearLayout empty=ui.card(libraryList,12);ui.title(empty,"这里保留你的文件入口",17,0);ui.text(empty,"选择自动迁移类型并连接电脑后，文件会出现在这里。点击文件可放大预览、查看原件或恢复到手机。",13,CompanionUi.MUTED,10);ui.button(empty,"设置自动迁移",false,()->{settings=true;render();});return;}
  String q=searchQuery.toLowerCase(Locale.ROOT);int shown=0;
  SimpleDateFormat fmt=new SimpleDateFormat("yyyy-MM-dd HH:mm",Locale.CHINA);
  for(int i=0;i<items.length();i++){JSONObject j=items.optJSONObject(i);if(j==null)continue;String name=j.optString("name"),path=j.optString("path");if(!q.isEmpty()&&!name.toLowerCase(Locale.ROOT).contains(q)&&!path.toLowerCase(Locale.ROOT).contains(q))continue;shown++;LinearLayout card=ui.card(libraryList,10);String date=fmt.format(new Date(j.optLong("modified")));thumbnail(card,j,listGen);ui.row(card,j.optString("kind").equals("photos")?UiGlyph.IMAGE:j.optString("kind").equals("videos")?UiGlyph.VIDEO:UiGlyph.FILE,name,date+" · "+bytes(j.optLong("size"))+"\n"+stateLabel(j.optString("state"))+(j.optString("kind").equals("photos")?" · 点击放大预览":""),()->details(j));}
  if(shown==0){LinearLayout empty=ui.card(libraryList,12);ui.title(empty,"没有匹配“"+searchQuery+"”的文件",16,0);ui.text(empty,"请尝试其他关键词，或切换上方分类与状态筛选。",13,CompanionUi.MUTED,8);}
 }
 static boolean isImageName(String name){if(name==null)return false;String n=name.toLowerCase(Locale.ROOT);return n.endsWith(".jpg")||n.endsWith(".jpeg")||n.endsWith(".png")||n.endsWith(".webp")||n.endsWith(".gif")||n.endsWith(".bmp");}
 static final class DecodedImage {
  final android.graphics.Bitmap bitmap;final int width,height;
  DecodedImage(android.graphics.Bitmap b,int w,int h){bitmap=b;width=w;height=h;}
 }
 static DecodedImage decodeSampled(java.io.File f){
  if(f==null||!f.isFile())return null;
  android.graphics.BitmapFactory.Options bounds=new android.graphics.BitmapFactory.Options();bounds.inJustDecodeBounds=true;android.graphics.BitmapFactory.decodeFile(f.getPath(),bounds);
  if(bounds.outWidth<=0||bounds.outHeight<=0)return null;
  android.graphics.BitmapFactory.Options opts=new android.graphics.BitmapFactory.Options();opts.inSampleSize=1;
  while(Math.max(bounds.outWidth,bounds.outHeight)/opts.inSampleSize>2048)opts.inSampleSize*=2;
  android.graphics.Bitmap b=android.graphics.BitmapFactory.decodeFile(f.getPath(),opts);
  return b==null?null:new DecodedImage(b,bounds.outWidth,bounds.outHeight);
 }
 static final class ZoomableImageView extends View {
  private android.graphics.Bitmap bitmap;private int origW,origH;private float scale=1f,panX=0f,panY=0f,lastX,lastY;private int rotation=0;private boolean dragging=false;private Runnable onChange;
  private final android.graphics.Paint paint=new android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG|android.graphics.Paint.FILTER_BITMAP_FLAG);
  private final ScaleGestureDetector scaleDetector;private final GestureDetector gestureDetector;
  ZoomableImageView(Context c){
   super(c);setContentDescription("可缩放图片预览");setBackgroundColor(0xff18181b);
   scaleDetector=new ScaleGestureDetector(c,new ScaleGestureDetector.SimpleOnScaleGestureListener(){@Override public boolean onScale(ScaleGestureDetector d){setZoom(scale*d.getScaleFactor());return true;}});
   gestureDetector=new GestureDetector(c,new GestureDetector.SimpleOnGestureListener(){@Override public boolean onDown(MotionEvent e){return true;}@Override public boolean onDoubleTap(MotionEvent e){setZoom(scale>1.2f?1f:2.5f);return true;}});
  }
  void setOnChange(Runnable r){onChange=r;}
  void setImage(DecodedImage d){if(d==null)return;bitmap=d.bitmap;origW=d.width;origH=d.height;scale=1f;panX=0f;panY=0f;rotation=0;fire();invalidate();}
  void setZoom(float next){scale=Math.max(1f,Math.min(5f,next));if(scale<=1.01f){panX=0f;panY=0f;}fire();invalidate();}
  void zoomBy(float factor){setZoom(scale*factor);}
  void resetZoom(){scale=1f;panX=0f;panY=0f;rotation=0;fire();invalidate();}
  void rotate90(){rotation=(rotation+90)%360;panX=0f;panY=0f;fire();invalidate();}
  int zoomPercent(){return Math.round(scale*100f);}
  int origWidth(){return origW;}
  int origHeight(){return origH;}
  boolean hasImage(){return bitmap!=null&&!bitmap.isRecycled();}
  private void fire(){if(onChange!=null)onChange.run();}
  @Override protected void onDraw(android.graphics.Canvas canvas){
   super.onDraw(canvas);canvas.drawColor(0xff18181b);
   if(bitmap==null||bitmap.isRecycled()||getWidth()<=0||getHeight()<=0)return;
   boolean swapped=(rotation%180)!=0;
   float iw=swapped?bitmap.getHeight():bitmap.getWidth(),ih=swapped?bitmap.getWidth():bitmap.getHeight();
   if(iw<=0||ih<=0)return;
   float base=Math.min(getWidth()/iw,getHeight()/ih),fw=iw*base*scale,fh=ih*base*scale;
   float maxX=Math.max(0f,(fw-getWidth())/2f),maxY=Math.max(0f,(fh-getHeight())/2f);
   panX=Math.max(-maxX,Math.min(maxX,panX));panY=Math.max(-maxY,Math.min(maxY,panY));
   canvas.save();canvas.translate(getWidth()/2f+panX,getHeight()/2f+panY);canvas.rotate(rotation);
   float total=base*scale;canvas.scale(total,total);
   canvas.drawBitmap(bitmap,-bitmap.getWidth()/2f,-bitmap.getHeight()/2f,paint);canvas.restore();
  }
  @Override public boolean onTouchEvent(MotionEvent e){
   if(getParent()!=null)getParent().requestDisallowInterceptTouchEvent(scale>1.01f||e.getPointerCount()>1);
   scaleDetector.onTouchEvent(e);gestureDetector.onTouchEvent(e);
   switch(e.getActionMasked()){
    case MotionEvent.ACTION_DOWN:lastX=e.getX();lastY=e.getY();dragging=true;break;
    case MotionEvent.ACTION_MOVE:
     if(dragging&&!scaleDetector.isInProgress()&&e.getPointerCount()==1&&scale>1.01f){panX+=e.getX()-lastX;panY+=e.getY()-lastY;invalidate();}
     lastX=e.getX();lastY=e.getY();break;
    case MotionEvent.ACTION_UP:case MotionEvent.ACTION_CANCEL:case MotionEvent.ACTION_POINTER_UP:dragging=false;break;
   }
   return true;
  }
 }
 static ZoomableImageView addZoomBox(Context ctx,CompanionUi ui,LinearLayout parent,int heightDp,TextView metaLabel,String sourceHint){
  ZoomableImageView zv=new ZoomableImageView(ctx);LinearLayout.LayoutParams zp=new LinearLayout.LayoutParams(-1,ui.dp(heightDp));zp.topMargin=ui.dp(12);parent.addView(zv,zp);
  LinearLayout bar=new LinearLayout(ctx);LinearLayout.LayoutParams bp=new LinearLayout.LayoutParams(-1,-2);bp.topMargin=ui.dp(8);parent.addView(bar,bp);
  Button out=new Button(ctx),reset=new Button(ctx),in=new Button(ctx),rot=new Button(ctx);
  out.setText("缩小 -");reset.setText("100%");in.setText("放大 +");rot.setText("旋转 90°");
  for(Button b:new Button[]{out,reset,in,rot}){ui.styleButton(b,false);b.setTextSize(12);b.setMinHeight(ui.dp(40));b.setPadding(ui.dp(6),ui.dp(6),ui.dp(6),ui.dp(6));LinearLayout.LayoutParams cp=new LinearLayout.LayoutParams(0,ui.dp(40),1);if(bar.getChildCount()>0)cp.leftMargin=ui.dp(6);bar.addView(b,cp);}
  out.setOnClickListener(v->zv.zoomBy(0.8f));reset.setOnClickListener(v->zv.resetZoom());in.setOnClickListener(v->zv.zoomBy(1.25f));rot.setOnClickListener(v->zv.rotate90());
  zv.setOnChange(()->{reset.setText(zv.zoomPercent()+"%");if(metaLabel!=null&&zv.origWidth()>0)metaLabel.setText(zv.origWidth()+" × "+zv.origHeight()+" px · 缩放 "+zv.zoomPercent()+"% · 双指缩放 / 双击 2.5x / 拖动"+(sourceHint==null||sourceHint.isEmpty()?"":" · "+sourceHint));});
  return zv;
 }
 private static final class ThumbnailView extends View {
  private android.graphics.Bitmap bitmap;private final android.graphics.Paint paint=new android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG|android.graphics.Paint.FILTER_BITMAP_FLAG);
  ThumbnailView(Context c){super(c);setContentDescription("文件缩略图，点击放大预览");}
  void setBitmap(android.graphics.Bitmap b){bitmap=b;invalidate();}
  protected void onDraw(android.graphics.Canvas canvas){super.onDraw(canvas);if(bitmap==null||bitmap.isRecycled())return;float scale=Math.max(getWidth()/(float)bitmap.getWidth(),getHeight()/(float)bitmap.getHeight());float w=bitmap.getWidth()*scale,h=bitmap.getHeight()*scale;canvas.drawBitmap(bitmap,null,new android.graphics.RectF((getWidth()-w)/2,(getHeight()-h)/2,(getWidth()+w)/2,(getHeight()+h)/2),paint);}
 }
 private void thumbnail(LinearLayout card,JSONObject j,int listGen){
  if(!j.optString("kind").equals("photos")&&!j.optString("kind").equals("videos"))return;
  ThumbnailView view=new ThumbnailView(this);view.setOnClickListener(v->details(j));
  String id=j.optString("id");
  android.graphics.Bitmap cached=id.isEmpty()?null:thumbCache.get(id);
  if(cached!=null&&!cached.isRecycled()){
   view.setBitmap(cached);view.setVisibility(View.VISIBLE);
   card.addView(view,new LinearLayout.LayoutParams(-1,ui.dp(150)));
   return;
  }
  view.setVisibility(View.GONE);card.addView(view,new LinearLayout.LayoutParams(-1,ui.dp(150)));
  long g=generation;
  worker.execute(()->{
   if(listGen!=listGeneration||g!=generation)return;
   try{
    android.graphics.Bitmap b=android.graphics.BitmapFactory.decodeFile(new ExpansionStore(this).thumbnail(id).getPath());
    if(b!=null){
     if(!id.isEmpty())thumbCache.put(id,b);
     handler.post(()->{if(!isDestroyed()&&g==generation&&listGen==listGeneration){view.setBitmap(b);view.setVisibility(View.VISIBLE);}});
    }
   }catch(Exception ignored){}
  });
 }

 static String stateLabel(String s){switch(s){case "archived":return "原件在电脑";case "restored":return "已恢复到手机";case "failed":return "迁移未完成";case "releasing":return "正在完成迁移";default:return "待完成迁移";}}
 private void details(JSONObject j){
  Dialog dialog=new Dialog(this);ScrollView scroll=new ScrollView(this);LinearLayout card=ui.column();card.setPadding(ui.dp(20),ui.dp(20),ui.dp(20),ui.dp(20));card.setBackground(ui.shape(android.graphics.Color.WHITE,CompanionUi.LINE,12));scroll.addView(card);ui.title(card,j.optString("name"),18,0);ui.text(card,stateLabel(j.optString("state"))+" · "+bytes(j.optLong("size")),13,CompanionUi.MUTED,8);ui.text(card,j.optString("path"),12,CompanionUi.MUTED,6);if(!j.optString("error").isEmpty())ui.text(card,j.optString("error"),13,CompanionUi.AMBER,10);
  boolean visual=j.optString("kind").equals("photos")||j.optString("kind").equals("videos")||isImageName(j.optString("name"));
  if(visual){
   TextView meta=ui.text(card,"正在加载预览（支持双指缩放、双击放大、拖动查看）…",12,CompanionUi.MUTED,8);
   ZoomableImageView zv=addZoomBox(this,ui,card,260,meta,"");
   Button hdBtn=new Button(this);ui.styleButton(hdBtn,false);hdBtn.setText("从电脑加载高清原图到此处预览");hdBtn.setVisibility(View.GONE);
   LinearLayout.LayoutParams hp=new LinearLayout.LayoutParams(-1,-2);hp.topMargin=ui.dp(8);card.addView(hdBtn,hp);
   worker.execute(()->{try{
    ExpansionStore s=new ExpansionStore(this);java.io.File full=j.optString("kind").equals("videos")?null:s.available(j);
    DecodedImage dec=decodeSampled(full!=null?full:s.thumbnail(j.getString("id")));
    boolean isFull=full!=null&&dec!=null;
    if(dec!=null)handler.post(()->{if(!isDestroyed()&&dialog.isShowing()){zv.setOnChange(()->{meta.setText(zv.origWidth()+" × "+zv.origHeight()+" px · 缩放 "+zv.zoomPercent()+"% · "+(isFull?"高清原图":"本地预览缩略图")+" · 双指/双击缩放");});zv.setImage(dec);if(!isFull&&j.optString("kind").equals("photos")&&status.optBoolean("online")&&(j.optString("state").equals("archived")||j.optString("state").equals("restored")))hdBtn.setVisibility(View.VISIBLE);}});
   }catch(Exception ignored){}});
   hdBtn.setOnClickListener(v->{hdBtn.setEnabled(false);hdBtn.setText("正在从电脑取回高清原图…");worker.execute(()->{try{
    ExpansionStore s=new ExpansionStore(this);JSONObject r=s.request(j.getString("id"),"view");long end=SystemClock.elapsedRealtime()+120000;
    while(!r.optString("state").equals("ready")){if(Thread.currentThread().isInterrupted())return;if(r.optString("state").equals("error"))throw new java.io.IOException(r.optString("error"));if(!s.online(s.host()))throw new java.io.IOException("电脑已断开");if(SystemClock.elapsedRealtime()>end)throw new java.io.IOException("取回超时");Thread.sleep(400);r=s.requestState(j.getString("id"));}
    DecodedImage hd=decodeSampled(s.available(j));
    handler.post(()->{if(!isDestroyed()&&dialog.isShowing()&&hd!=null){hdBtn.setVisibility(View.GONE);zv.setOnChange(()->{meta.setText(zv.origWidth()+" × "+zv.origHeight()+" px · 缩放 "+zv.zoomPercent()+"% · 高清原图已缓存");});zv.setImage(hd);}});
   }catch(Exception e){handler.post(()->{if(!isDestroyed()&&dialog.isShowing()){hdBtn.setEnabled(true);hdBtn.setText("重试加载高清原图（"+e.getMessage()+"）");}});}});});
  }
  if(j.optString("state").equals("archived")||j.optString("state").equals("restored")){ui.button(card,"外部应用打开",true,()->{dialog.dismiss();retrieve(j,"view");});if(j.optString("state").equals("archived"))ui.button(card,"恢复到手机",false,()->new AlertDialog.Builder(this).setTitle("恢复原件到这台手机？").setMessage(j.optString("name")+"\n需要手机空间："+bytes(j.optLong("size"))+"\n电脑副本保留。请保持与原电脑连接。").setNegativeButton("取消",null).setPositiveButton("恢复到手机",(x,w)->{dialog.dismiss();retrieve(j,"restore");}).show());else ui.button(card,"移除记录",false,()->{dialog.dismiss();task(()->{new ExpansionStore(this).forget(j.getString("id"));return "已移除记录，手机原件和电脑副本保留";});});}
  if(j.optString("state").equals("prepared")||j.optString("state").equals("failed"))ui.button(card,"取消此文件的迁移",false,()->{dialog.dismiss();task(()->{new ExpansionStore(this).forget(j.getString("id"));return "已取消，手机原件保留且不再自动迁移";});});
  ui.button(card,"关闭",false,dialog::dismiss);dialog.setContentView(scroll);dialog.getWindow().setBackgroundDrawableResource(android.R.color.transparent);dialog.show();dialog.getWindow().setLayout(getResources().getDisplayMetrics().widthPixels-ui.dp(32),-2);
 }
 private interface Job{String run()throws Exception;}
 private void task(Job job){if(busy)return;busy=true;notice="正在处理…";render();worker.execute(()->{String n;try{n=job.run();}catch(Exception e){n=e.getMessage();}String next=n;handler.post(()->{if(isDestroyed())return;busy=false;notice=next;load();});});}
 private void retrieve(JSONObject j,String mode){task(()->{ExpansionStore s=new ExpansionStore(this);JSONObject r=s.request(j.getString("id"),mode);long end=SystemClock.elapsedRealtime()+300000;while(!r.optString("state").equals("ready")){if(Thread.currentThread().isInterrupted())throw new InterruptedException();if(r.optString("state").equals("error"))throw new java.io.IOException(r.optString("error"));if(!s.online(s.host()))throw new java.io.IOException("电脑已断开，请重新连接");if(SystemClock.elapsedRealtime()>end)throw new java.io.IOException("文件仍在取回，请稍后重试");Thread.sleep(500);r=s.requestState(j.getString("id"));}if(mode.equals("view"))handler.post(()->{try{Uri u=DocumentsContract.buildDocumentUri(ExpansionStore.DOCS,j.optString("id"));startActivity(new Intent(Intent.ACTION_VIEW).setDataAndType(u,TransferStore.mime(j.optString("name"))).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION));}catch(Exception e){notice="没有可打开此格式的应用，请在电脑查看";render();}});return mode.equals("restore")?"已恢复到手机，电脑副本仍保留":"文件已取回，可离线查看";});}
}
