package com.stepx.companion.neo;

import android.content.Context;
import android.content.SharedPreferences;
import android.graphics.Color;
import android.graphics.PixelFormat;
import android.graphics.Rect;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.content.res.ColorStateList;
import android.view.Gravity;
import android.view.View;
import android.view.WindowManager;
import android.view.accessibility.AccessibilityWindowInfo;
import android.widget.*;

/** Companion presentation for the existing native control callbacks and exclusion bounds. */
final class CompanionControlBar {
    private static final int INK=0xff18181b,MUTED=0xff71717a,LINE=0xffe5e5ea,GREEN=0xff10b981;
    private final ControlService host;
    private WindowManager windows;
    ControlCapsule view;
    TextView detail;
    private TextView brief;
    private boolean expanded,human;
    private String windowTitle="",status="",next="观察";
    private Runnable takeover,resume,stop;
    private long dragEpoch,avoidingEpoch=-1;
    CompanionControlBar(ControlService host){this.host=host;}
    private int dp(int value){return host.dp(value);}
    private int clamp(int value,int min,int max){return Math.max(min,Math.min(Math.max(min,max),value));}
    boolean owns(AccessibilityWindowInfo window){return window.getType()==AccessibilityWindowInfo.TYPE_ACCESSIBILITY_OVERLAY&&!windowTitle.isEmpty()&&windowTitle.contentEquals(window.getTitle()==null?"":window.getTitle());}
    Rect bounds(){
        if(view==null||!view.isAttachedToWindow())return null;
        for(AccessibilityWindowInfo window:host.getWindows())if(owns(window)){Rect rect=new Rect();window.getBoundsInScreen(rect);if(!rect.isEmpty())return rect;}
        return null;
    }
    private GradientDrawable shape(int color,int stroke,int radius){GradientDrawable d=new GradientDrawable();d.setColor(color);d.setCornerRadius(dp(radius));if(stroke!=0)d.setStroke(dp(1),stroke);return d;}
    private TextView text(String value,int size,int color){TextView t=new TextView(host);t.setText(value);t.setTextSize(size);t.setTextColor(color);t.setGravity(Gravity.CENTER);t.setIncludeFontPadding(false);return t;}
    private void safeUpdateLayout(WindowManager.LayoutParams p){
        if(windows!=null&&view!=null&&view.isAttachedToWindow()){
            try{windows.updateViewLayout(view,p);}catch(RuntimeException ignored){}
        }
    }
    void show(boolean paused,String message,Runnable pause,Runnable continueTask,Runnable end){
        hide();human=paused;status=message;next=CompanionOverlayPolicy.phase(paused,message);expanded=false;
        takeover=pause;resume=continueTask;stop=end;windows=(WindowManager)host.getSystemService(Context.WINDOW_SERVICE);
        final int[] origin=new int[6],wanted=new int[2];final boolean[] ready={false};
        view=new ControlCapsule(host,new ControlCapsule.Drag(){
            public void begin(){dragEpoch++;ready[0]=false;Rect r=bounds();if(r==null)return;WindowManager.LayoutParams p=(WindowManager.LayoutParams)view.getLayoutParams();origin[0]=wanted[0]=r.left;origin[1]=wanted[1]=r.top;origin[2]=p.x;origin[3]=p.y;origin[4]=r.width();origin[5]=r.height();ready[0]=true;}
            public void move(float dx,float dy){if(!ready[0]||view==null)return;android.util.DisplayMetrics m=host.screenMetrics();wanted[0]=clamp(origin[0]+(int)dx,dp(8),m.widthPixels-origin[4]-dp(8));wanted[1]=clamp(origin[1]+(int)dy,dp(32),m.heightPixels-origin[5]-dp(40));WindowManager.LayoutParams p=(WindowManager.LayoutParams)view.getLayoutParams();p.x=origin[2]+wanted[0]-origin[0];p.y=origin[3]+wanted[1]-origin[1];safeUpdateLayout(p);}
            public void end(){if(ready[0])remember();}
        });
        view.setOrientation(LinearLayout.VERTICAL);view.setElevation(0);view.setOnClickListener(v->{if(!expanded)toggle(true);});render();
        WindowManager.LayoutParams p=new WindowManager.LayoutParams(dp(64),dp(64),WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE,PixelFormat.TRANSLUCENT);
        p.gravity=Gravity.TOP|Gravity.LEFT;windowTitle="Companion control "+java.util.UUID.randomUUID();p.setTitle(windowTitle);
        p.x=host.screenMetrics().widthPixels-dp(80);p.y=host.screenMetrics().heightPixels/2;
        try{windows.addView(view,p);}catch(RuntimeException ignored){view=null;detail=null;brief=null;return;}
        ControlCapsule shown=view;long epoch=dragEpoch;view.postDelayed(()->{if(view==shown&&dragEpoch==epoch)restore();},180);
    }
    private void render(){
        view.removeAllViews();detail=null;brief=null;view.setGravity(Gravity.CENTER);
        view.setContentDescription(expanded?"手机任务控制，可拖动；展开显示暂停、继续和停止":"手机任务，"+next+"；点击展开，可拖动");
        if(!expanded){
            view.setPadding(dp(4),dp(6),dp(4),dp(6));view.setBackground(new RippleDrawable(ColorStateList.valueOf(0x33444455),shape(INK,0xff52525b,32),null));
            LinearLayout caption=new LinearLayout(host);caption.setGravity(Gravity.CENTER);View dot=new View(host);dot.setBackground(shape(human?0xffa1a1aa:GREEN,0,3));LinearLayout.LayoutParams dotParams=new LinearLayout.LayoutParams(dp(4),dp(4));dotParams.rightMargin=dp(3);caption.addView(dot,dotParams);caption.addView(text(human?"已暂停":"下一步",9,0xffd4d4d8));view.addView(caption,new LinearLayout.LayoutParams(-1,dp(16)));
            brief=text(next,14,Color.WHITE);brief.setTypeface(Typeface.create("sans-serif-medium",Typeface.NORMAL));view.addView(brief,new LinearLayout.LayoutParams(-1,dp(25)));
            return;
        }
        view.setPadding(dp(14),dp(10),dp(14),dp(12));view.setBackground(shape(Color.WHITE,LINE,12));
        LinearLayout top=new LinearLayout(host);top.setGravity(Gravity.CENTER_VERTICAL);view.addView(top,new LinearLayout.LayoutParams(-1,dp(44)));
        TextView title=text(human?"任务已暂停":"手机正在执行",15,INK);title.setGravity(Gravity.CENTER_VERTICAL);title.setTypeface(Typeface.create("sans-serif-medium",Typeface.NORMAL));top.addView(title,new LinearLayout.LayoutParams(0,-1,1));
        Button collapse=button("收起",false,()->toggle(false));top.addView(collapse,new LinearLayout.LayoutParams(dp(62),dp(44)));
        detail=text(status,13,MUTED);detail.setGravity(Gravity.CENTER_VERTICAL);detail.setMaxLines(2);detail.setEllipsize(android.text.TextUtils.TruncateAt.END);view.addView(detail,new LinearLayout.LayoutParams(-1,dp(52)));
        LinearLayout actions=new LinearLayout(host);view.addView(actions,new LinearLayout.LayoutParams(-1,dp(48)));
        Button first=button(human?"继续":"暂停",true,human?resume:takeover);actions.addView(first,new LinearLayout.LayoutParams(0,-1,1));
        Button last=button("停止",false,stop);LinearLayout.LayoutParams right=new LinearLayout.LayoutParams(0,-1,1);right.leftMargin=dp(8);actions.addView(last,right);
    }
    private Button button(String label,boolean primary,Runnable action){
        Button b=new Button(host);b.setText(label);b.setContentDescription(label+"手机任务");b.setAllCaps(false);b.setTextSize(14);b.setTextColor(primary?Color.WHITE:INK);b.setMinWidth(0);b.setMinimumWidth(0);b.setMinHeight(0);b.setMinimumHeight(0);b.setPadding(dp(6),0,dp(6),0);b.setStateListAnimator(null);b.setBackgroundTintList(null);b.setBackground(new RippleDrawable(ColorStateList.valueOf(0x22777788),shape(primary?INK:0xfff8f8fa,primary?0:LINE,8),null));b.setOnClickListener(v->action.run());return b;
    }
    private void toggle(boolean value){
        if(view==null||expanded==value)return;Rect old=bounds();expanded=value;render();android.util.DisplayMetrics m=host.screenMetrics();
        WindowManager.LayoutParams p=(WindowManager.LayoutParams)view.getLayoutParams();int width=expanded?Math.min(dp(300),m.widthPixels-dp(16)):dp(64),height=expanded?dp(176):dp(64);p.width=width;p.height=height;
        if(old!=null){p.x+=clamp(old.right-width,dp(8),m.widthPixels-width-dp(8))-old.left;p.y+=clamp(old.top,dp(32),m.heightPixels-height-dp(40))-old.top;}
        safeUpdateLayout(p);
    }
    void update(String message){status=message==null?"":message;next=CompanionOverlayPolicy.phase(human,status);if(detail!=null)detail.setText(status);if(brief!=null)brief.setText(next);}
    void proposal(String action){if(human)return;next=CompanionOverlayPolicy.action(action);if(brief!=null)brief.setText(next);}
    private SharedPreferences prefs(){return host.getSharedPreferences("companion-overlay",0);}
    private void remember(){Rect r=bounds();if(r==null)return;android.util.DisplayMetrics m=host.screenMetrics();float x=(r.right-dp(80))/(float)Math.max(1,m.widthPixels-dp(88)),y=(r.top-dp(32))/(float)Math.max(1,m.heightPixels-r.height()-dp(72));prefs().edit().putFloat("x",Math.max(0,Math.min(1,x))).putFloat("y",Math.max(0,Math.min(1,y))).apply();}
    void restore(){Rect r=bounds();if(r==null||view==null)return;android.util.DisplayMetrics m=host.screenMetrics();int right=dp(80)+(int)((m.widthPixels-dp(88))*prefs().getFloat("x",1f)),x=clamp(right-r.width(),dp(8),m.widthPixels-r.width()-dp(8)),y=dp(32)+(int)(Math.max(1,m.heightPixels-r.height()-dp(72))*prefs().getFloat("y",.5f));WindowManager.LayoutParams p=(WindowManager.LayoutParams)view.getLayoutParams();p.x+=x-r.left;p.y+=y-r.top;safeUpdateLayout(p);}
    boolean avoid(Rect path)throws Exception{
        if(view==null)return false;Rect r=bounds();if(r==null)throw new Exception("控制浮钮位置正在同步");if(!Rect.intersects(r,path))return false;
        android.util.DisplayMetrics m=host.screenMetrics();int y=r.centerY()<m.heightPixels/2?m.heightPixels-r.height()-dp(40):dp(32);Rect next=new Rect(r.left,y,r.right,y+r.height());if(Rect.intersects(next,path))throw new Exception("控制浮钮暂时遮挡动作路径");avoidingEpoch=dragEpoch;WindowManager.LayoutParams p=(WindowManager.LayoutParams)view.getLayoutParams();p.y+=y-r.top;safeUpdateLayout(p);return true;
    }
    void restoreAfterAvoidance(){if(avoidingEpoch==dragEpoch)restore();avoidingEpoch=-1;}
    void hide(){if(view!=null)try{windows.removeView(view);}catch(Exception ignored){}view=null;detail=null;brief=null;avoidingEpoch=-1;}
}
