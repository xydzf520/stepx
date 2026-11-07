package com.stepx.companion.app;

import android.content.*;
import android.content.pm.*;
import android.net.Uri;
import android.os.Bundle;
import android.util.Base64;
import org.json.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

/** Companion-owned delegation to the embedded Neo runtime in this same APK. */
final class NeoAgent {
    static final String PACKAGE="com.stepx.companion.app";
    static final String AUTHORITY=PACKAGE+".neo.control";
    private static final Uri URI=Uri.parse("content://"+AUTHORITY);
    static boolean installed(Context c){
        ProviderInfo p=c.getPackageManager().resolveContentProvider(AUTHORITY,0);
        return p!=null&&PACKAGE.equals(p.packageName)&&c.getPackageManager().checkSignatures(c.getPackageName(),PACKAGE)==PackageManager.SIGNATURE_MATCH;
    }
    static boolean selected(Context c){return c.getSharedPreferences("gui_backend",0).getBoolean("neo",false);}
    static JSONObject call(Context c,JSONObject q)throws Exception {
        if(!installed(c))throw new IllegalStateException("请先更新带内置 GUI Agent 的 StepX伴侣");
        Bundle extras=new Bundle();extras.putString("request",q.toString());
        Bundle answer=c.getContentResolver().call(URI,"request",null,extras);
        if(answer==null||answer.getString("result")==null)throw new IllegalStateException("Neo Agent 未返回状态");
        JSONObject value=new JSONObject(new String(Base64.decode(answer.getString("result"),Base64.DEFAULT),StandardCharsets.UTF_8));
        if(!value.optBoolean("ok"))throw new IllegalStateException(value.optString("error","Neo Agent 操作失败"));
        return value;
    }
    static synchronized JSONObject enable(Context c)throws Exception {
        JSONObject old=PhoneGuiAgent.status(c);
        GuiAccessibilityService legacy=GuiAccessibilityService.instance;
        if(NeoPolicy.active(old.optString("phase"))||(legacy!=null&&legacy.task.active()))throw new IllegalStateException("请先停止原执行器任务，再切换 Neo Agent");
        JSONObject config=call(c,new JSONObject().put("op","config_info"));
        if(!config.optBoolean("hasKey")&&GuiModelVault.configured(c)){
            call(c,new JSONObject().put("op","config").put("endpoint","https://api.stepfun.com/step_plan/v1")
                .put("model",PhoneGuiAgent.MODEL).put("protocol","openai").put("key",GuiModelVault.read(c)).put("userConfirmed",true));
        }
        if(!c.getSharedPreferences("gui_backend",0).edit().putBoolean("neo",true).commit())throw new IllegalStateException("未能保存执行器选择，请重试");
        return status(c);
    }
    static JSONObject info(Context c)throws Exception {
        if(!installed(c))return new JSONObject().put("ok",true).put("installed",false).put("selected",selected(c));
        return status(c);
    }
    static synchronized JSONObject disable(Context c)throws Exception {
        boolean present=installed(c);
        if(present){JSONObject current=status(c);
            if(NeoPolicy.active(current.optString("phase"))||current.optBoolean("executorBusy"))throw new IllegalStateException("请先停止 Neo Agent 任务再恢复旧执行器");}
        if(!c.getSharedPreferences("gui_backend",0).edit().putBoolean("neo",false).commit())throw new IllegalStateException("未能保存执行器选择，请重试");
        return new JSONObject().put("ok",true).put("selected",false).put("installed",present);
    }
    static synchronized JSONObject executionMode(Context c,String mode)throws Exception {
        if(!mode.equals("background")&&!mode.equals("foreground"))throw new IllegalArgumentException("执行模式无效");
        JSONObject state=call(c,new JSONObject().put("op","status"));
        if(NeoPolicy.active(state.optString("state"))||state.optBoolean("executorBusy"))throw new IllegalStateException("请先结束当前任务，再切换执行方式");
        if(mode.equals("background"))call(c,new JSONObject().put("op","mdi_probe"));
        if(!c.getSharedPreferences("gui_backend",0).edit().putString("executionMode",mode).commit())throw new IllegalStateException("未能保存执行方式");
        return status(c);
    }
    static synchronized JSONObject statusReadonly(Context c)throws Exception {
        JSONObject raw=call(c,new JSONObject().put("op","status"));
        String preferred=c.getSharedPreferences("gui_backend",0).getString("executionMode","foreground");
        raw.put("preferredExecutionMode",preferred);
        JSONObject value=normalize(raw).put("installed",true).put("selected",selected(c));
        // Background readiness normally requires an active MDI probe. A passive
        // observer must leave that fact unknown rather than starting a probe.
        value.put("readinessKnown",!preferred.equals("background")||NeoPolicy.active(raw.optString("state")));
        return value;
    }
    static synchronized JSONObject status(Context c)throws Exception {
        JSONObject raw=call(c,new JSONObject().put("op","status")),config=raw.optJSONObject("config");
        // Adopt the companion's all-app default once for an already-ready legacy setup.
        // Explicit custom scopes and changed/unapproved model recipients are preserved.
        if(selected(c)&&raw.optBoolean("ready")&&!raw.optBoolean("executorBusy")&&!NeoPolicy.active(raw.optString("state"))&&config!=null&&"default".equals(config.optString("scopeMode"))){
            call(c,new JSONObject().put("op","apps").put("mode","all").put("userConfirmed",true));
            raw=call(c,new JSONObject().put("op","status"));
        }
        String preferred=c.getSharedPreferences("gui_backend",0).getString("executionMode","foreground");
        if(preferred.equals("background")&&config!=null&&config.optBoolean("hasKey")&&!NeoPolicy.active(raw.optString("state"))){
            try{JSONObject probe=call(c,new JSONObject().put("op","mdi_probe"));raw.put("ready",probe.optBoolean("transportReady")).put("accessibilityConnected",probe.optBoolean("transportReady"));}
            catch(Exception e){raw.put("ready",false).put("accessibilityConnected",false).put("backendError","后台虚拟屏系统服务未就绪，请安装或修复伴侣服务");}
        }
        raw.put("preferredExecutionMode",preferred);
        JSONObject value=normalize(raw).put("installed",true).put("selected",selected(c));AgentHistory.observe(c,value);return value;
    }
    static JSONObject stepPlanKeyTarget(Context c)throws Exception {
        JSONObject current=status(c),config=current.getJSONObject("config");
        if(NeoPolicy.active(current.optString("phase"))||current.optBoolean("executorBusy"))throw new IllegalStateException("请先停止任务，再修改模型密钥");
        if(config.optBoolean("hasKey")&&!"https://api.stepfun.com/step_plan/v1".equals(config.optString("endpoint")))
            throw new IllegalStateException("当前 Neo 使用自定义模型，请在手机“模型与应用范围”修改密钥；不会覆盖现有服务");
        return config;
    }
    static synchronized JSONObject importStepPlanKey(Context c)throws Exception {
        JSONObject previous=stepPlanKeyTarget(c);
        call(c,new JSONObject().put("op","config").put("endpoint","https://api.stepfun.com/step_plan/v1")
            .put("model",previous.optBoolean("hasKey")?previous.optString("model",PhoneGuiAgent.MODEL):PhoneGuiAgent.MODEL)
            .put("protocol","openai").put("key",GuiModelVault.read(c)).put("userConfirmed",true));
        return status(c);
    }
    static JSONObject normalize(JSONObject raw)throws Exception {
        String state=raw.optString("state","idle"),phase=state;
        if(state.equals("planning"))phase="queued";
        if(state.equals("paused_planning"))phase="paused";
        if(state.equals("stopped"))phase="cancelled";
        JSONObject config=raw.optJSONObject("config");
        boolean verified=NeoPolicy.verified(state,raw.optBoolean("completed"),raw.optBoolean("goal_verified"),raw.optBoolean("terminal"));
        if(state.equals("completed")&&!verified)phase="reported";
        JSONObject result=new JSONObject().put("verified",verified).put("phoneStopped",raw.optBoolean("terminal"));
        result.put("verification",raw.opt("verification"));
        return new JSONObject(raw.toString()).put("ok",true).put("framework","Neo Agent Touch")
            .put("task",raw.optString("requestId")).put("phase",phase).put("progress",raw.optString("detail"))
            .put("state",state.equals("completed")&&!verified?"reported":state).put("completed",verified).put("goal_verified",verified)
            .put("needsVerification",phase.equals("reported")||raw.optBoolean("needsVerification"))
            .put("model",config==null?"":config.optString("model")).put("modelConfigured",config!=null&&config.optBoolean("hasKey"))
            .put("executorConnected",raw.optBoolean("accessibilityConnected",raw.optBoolean("ready")))
            .put("ready",raw.optBoolean("ready")&&config!=null&&config.optBoolean("hasKey"))
            .put("requiresDesktop",false).put("executionModel","on-device-independent-planner").put("result",result)
            .put("nextStepHint","任务在手机独立执行。使用同一 task 查询 status；只有 phase=completed 且 result.verified、result.phoneStopped 均为 true 才可报告完成。reported 表示模型自述完成，尚未通过屏幕验收；needs_input 表示需要用户接管。不要重复发起或另行点击。");
    }
    static synchronized JSONObject request(Context c,int uid,JSONObject q,boolean trusted)throws Exception {
        String op=q.getString("op");
        if(!Arrays.asList("start","status","pause","resume","stop").contains(op))throw new IllegalArgumentException("不支持的任务操作");
        android.content.SharedPreferences owner=c.getSharedPreferences("neo-owner",0);
        String task=owner.getString("task","");
        JSONObject request=new JSONObject().put("op",op);
        if(op.equals("start")){
            String goal=q.getString("goal");if(goal.trim().isEmpty()||goal.length()>4000)throw new IllegalArgumentException("任务目标无效");
            JSONObject current=status(c);
            if(NeoPolicy.active(current.optString("phase"))||current.optBoolean("executorBusy"))throw new IllegalStateException("手机已有任务，请先停止或等待完成");
            String mode=q.optString("executionMode",c.getSharedPreferences("gui_backend",0).getString("executionMode","foreground"));
            if(!mode.equals("foreground")&&!mode.equals("background"))throw new IllegalArgumentException("执行模式无效");
            request.put("executionMode",mode);
            task=UUID.randomUUID().toString().replace("-","");request.put("requestId",task).put("goal",goal).put("session","companion-"+uid);
            // Native MCP cannot supply its own success assertion. Known exact
            // tasks use the same companion-owned acceptance recipe on all routes.
            if(NeoPolicy.systemVersionPreset(goal))request.put("verify",new JSONObject()
                .put("preset","android_version").put("package","com.android.settings")
                .put("texts",new JSONArray().put("关于手机").put("Android 版本").put("系统版本")));
            else if(trusted&&q.optJSONObject("verify")!=null)request.put("verify",q.getJSONObject("verify"));
            // Persist the claim before starting: a process death after Binder acceptance
            // must not leave a running task without a recoverable calling owner.
            if(!owner.edit().putString("task",task).putInt("uid",uid).commit())throw new IllegalStateException("未能保存任务归属，尚未启动");
            JSONObject value=call(c,request);
            if(!task.equals(value.optString("requestId")))throw new IllegalStateException("执行器返回了不同任务，不能确认本次提交");
            JSONObject normalized=normalize(value);AgentHistory.observe(c,normalized);return normalized;
        }
        String requested=q.optString("task");
        if(!trusted&&(task.isEmpty()||uid!=owner.getInt("uid",-1)||!task.equals(requested)))throw new SecurityException("任务不属于当前调用方");
        if(!requested.isEmpty()&&!requested.equals(task))throw new IllegalStateException("任务标识已失效或不匹配");
        if(!op.equals("status")){
            if(task.isEmpty())throw new IllegalStateException("没有可控制的任务");
            request.put("requestId",task);
            if(op.equals("resume")){
                if(!trusted)throw new IllegalStateException("请从伴侣或 desktop 确认继续，Amoo 不能代替用户解除接管");
                request.put("userConfirmed",true);
            }
        }
        JSONObject value=call(c,request);
        if(!trusted&&!NeoPolicy.owns(uid,owner.getInt("uid",-1),task,requested,value.optString("requestId")))throw new SecurityException("原任务已被替换，未返回其他会话内容");
        JSONObject normalized=normalize(value);AgentHistory.observe(c,normalized);return normalized;
    }
    static void openSettings(Context c){
        if(!installed(c))throw new IllegalStateException("请先从 desktop 小工具更新 StepX伴侣");
        Intent launch=new Intent(c,NeoSettingsActivity.class);
        c.startActivity(launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
    }
}
