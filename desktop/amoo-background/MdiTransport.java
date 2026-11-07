package stepx;

import android.content.*;
import android.os.*;
import java.io.IOException;
import java.util.*;
import java.util.concurrent.*;
import org.json.*;

/** Original, permission-protected MCP Binder. No socket, shell or ambiguous retries. */
final class MdiTransport implements AutoCloseable {
    private static final String SERVICE="com.stepos.mcp.sdk.IMCPBinderService", CALLBACK="com.stepos.mcp.sdk.IMCPBinderCallback";
    static final String DEFAULT_PKG="com.stepos.ability", DEFAULT_CLS="com.stepos.taskrobot.ability.TaskRobotMCPService";
    private final Context context;
    private final ComponentName targetComponent;
    private final String session="stepx-mdi-"+UUID.randomUUID();
    private final CompletableFuture<IBinder> connected=new CompletableFuture<>();
    private final ConcurrentHashMap<String,CompletableFuture<JSONObject>> pending=new ConcurrentHashMap<>();
    private IBinder remote;private boolean bound;private int nextId;
    private final ServiceConnection connection=new ServiceConnection(){
        public void onServiceConnected(ComponentName name,IBinder service){connected.complete(service);}
        public void onServiceDisconnected(ComponentName name){fail();}
        public void onNullBinding(ComponentName name){connected.completeExceptionally(new IOException("原厂 MDI 未提供 MCP 服务"));}
        public void onBindingDied(ComponentName name){fail();}
    };
    private final Binder callback=new Binder(){
        protected boolean onTransact(int code,Parcel data,Parcel reply,int flags)throws RemoteException{
            if(code==INTERFACE_TRANSACTION){if(reply!=null)reply.writeString(CALLBACK);return true;}
            if(code!=1)return false;data.enforceInterface(CALLBACK);String raw=data.readString();
            try{if(raw==null||raw.length()>700000)throw new IOException("原厂 MDI 返回过大");
                JSONObject value=new JSONObject(raw);CompletableFuture<JSONObject> f=pending.remove(value.optString("id"));if(f!=null)f.complete(value);
            }catch(Exception e){fail();}return true;
        }
    };
    private static String verifiedApk="";
    private static synchronized void verifyFirmware(Context context)throws Exception {
        if(!"StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys".equals(Build.FINGERPRINT)&&!"StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/12:user/release-keys".equals(Build.FINGERPRINT))throw new IOException("固件已变化，MDI MCP 需要重新评估；未创建虚拟屏");
        android.content.pm.PackageInfo info=context.getPackageManager().getPackageInfo("com.stepos.ability",0);
        long vc=info.getLongVersionCode();
        if(vc!=101002&&vc!=102003)throw new IOException("MDI 服务已更新，请重新验证兼容性");
        String path=info.applicationInfo.sourceDir;
        if(path.equals(verifiedApk))return;
        java.security.MessageDigest hash=java.security.MessageDigest.getInstance("SHA-256");
        try(java.io.InputStream in=new java.io.FileInputStream(path)){byte[] b=new byte[65536];int n;while((n=in.read(b))!=-1)hash.update(b,0,n);}
        StringBuilder hex=new StringBuilder();for(byte b:hash.digest())hex.append(String.format(java.util.Locale.ROOT,"%02x",b&255));
        String expectedSha=vc==102003?"a7dbeaa721c13836bbb3990026af6c3d66bf164afc4c73f70bd453aa119fbca8":"c9f56f3078df96d8d726422b1c622a1c19fe8c7dec8cadd8537d9a6ecf513ac4";
        if(!expectedSha.equals(hex.toString()))throw new IOException("MDI 接口资源已变化，请重新评估后再启用");
        verifiedApk=path;
    }
    MdiTransport(Context c)throws Exception {
        this(c,new ComponentName(DEFAULT_PKG,DEFAULT_CLS));
    }
    MdiTransport(Context c,ComponentName component)throws Exception {
        context=c;targetComponent=component!=null?component:new ComponentName(DEFAULT_PKG,DEFAULT_CLS);
        verifyFirmware(c);
        try{
            Intent intent=new Intent("com.stepos.mcp.sdk.TOOLS_SERVICE").setComponent(targetComponent);
            bound=context.bindService(intent,connection,Context.BIND_AUTO_CREATE);
            if(!bound){
                try{context.unbindService(connection);}catch(Exception ignored){}
                ComponentName fallback=fallbackComponent(targetComponent);
                if(fallback!=null)bound=context.bindService(new Intent("com.stepos.mcp.sdk.TOOLS_SERVICE").setComponent(fallback),connection,Context.BIND_AUTO_CREATE);
            }
            if(!bound)throw new IOException("原厂 MDI 服务未安装或权限不足");remote=connected.get(8,TimeUnit.SECONDS);
            transact(1,null);request("initialize",new JSONObject().put("protocolVersion","2024-11-05").put("capabilities",new JSONObject()).put("clientInfo",new JSONObject().put("name","StepX Companion").put("version","1")));
            transact(3,new JSONObject().put("jsonrpc","2.0").put("method","notifications/initialized").toString());
        }catch(Exception e){close();throw e;}
    }
    private static ComponentName fallbackComponent(ComponentName primary){
        if(primary==null)return null;
        String pkg=primary.getPackageName(),cls=primary.getClassName();
        if("com.stepos.settings".equals(pkg)&&"com.stepos.settings.mcp.SettingsMcpService".equals(cls))
            return new ComponentName("com.android.settings","com.android.settings.stepos.mcp.McpService");
        if("com.android.dialer".equals(pkg)&&"com.stepos.dialer.mcp.DialerMcpService".equals(cls))
            return new ComponentName("com.android.dialer","com.stepos.mcp.service.AbilityService");
        if("com.android.soundrecorder".equals(pkg)&&"com.stepos.soundrecorder.mcp.SoundRecorderMcpService".equals(cls))
            return new ComponentName("com.android.soundrecorder","com.android.soundrecorder.atomicabilitymcp.RecordManagerMcpService");
        return null;
    }
    void probe()throws Exception {
        JSONArray tools=request("tools/list",new JSONObject()).getJSONArray("tools");boolean found=false;
        for(int i=0;i<tools.length();i++)if("mdi_task_create".equals(tools.getJSONObject(i).optString("name")))found=true;
        if(!found)throw new IOException("当前固件没有 MDI 工具");
    }
    static final String ALLOWED_TOOL = "mdi_(task_create|task_finish|open_app|move_app|top_app|click|tap|long_click|swipe|fling|scale|set_text|key_event|back|home|wait|capture|snapshot|ime_enable)";
    private static final Set<String> COMMON_ABILITY_TOOLS = new HashSet<>(Arrays.asList(
        "set_back_screen_switch",
        "set_back_screen_enabled",
        "get_back_screen_enabled",
        "set_back_screen_frame",
        "clear_back_screen_frame",
        "set_back_screen_basic_type",
        "get_back_screen_basic_type",
        "start_back_screen_countdown",
        "cancel_back_screen_countdown",
        "get_back_screen_display_info",
        "get_back_screen_display_mode",
        "set_back_screen_display_mode",
        "get_back_screen_auto_brightness",
        "set_back_screen_auto_brightness",
        "get_back_screen_brightness",
        "set_back_screen_brightness",
        "get_back_screen_camera_link",
        "set_back_screen_camera_link",
        "get_back_screen_auto_rotate",
        "set_back_screen_auto_rotate",
        "wake_up_back_screen",
        "sleep_back_screen",
        "start_back_screen_nap",
        "get_current_location",
        "speak_text",
        "stop_speak_text",
        "start_vibration",
        "stop_vibration",
        "open_app_by_package",
        "open_app_by_name",
        "open_url_with_browser",
        "common_ability_clipboard_text",
        "common_ability_get_screen_size",
        "common_ability_get_app_list",
        "common_ability_get_app_count"
    ));
    private static final Set<String> GALLERY_TOOLS = new HashSet<>(Arrays.asList(
        "gallery_local_search",
        "gallery_intelligent_search",
        "gallery_aicut_intelligent_search",
        "gallery_ai_erase_image",
        "gallery_ai_extend_image",
        "gallery_ai_style_image",
        "gallery_get_latest_media",
        "gallery_get_media_by_time_range",
        "gallery_get_photo_info",
        "gallery_get_video_info",
        "gallery_get_trash_media_list",
        "gallery_restore_media_from_trash"
    ));
    private static final Set<String> ALARM_TOOLS = new HashSet<>(Arrays.asList(
        "alarm_clock_add",
        "alarm_clock_query",
        "alarm_clock_edit",
        "alarm_clock_delete",
        "alarm_clock_timer_add",
        "alarm_clock_delete_all",
        "alarm_clock_disable",
        "alarm_clock_disable_one",
        "alarm_clock_snooze",
        "alarm_clock_get_enable",
        "alarm_clock_get_current_time_zone",
        "alarm_clock_get_current_time_by_city"
    ));
    private static final Set<String> SETTINGS_TOOLS = new HashSet<>(Arrays.asList(
        "bluetooth_Switch",
        "wifi_Switch",
        "hotspot_Switch",
        "nfc_Switch",
        "dark_ui_mode",
        "eye_comfort_mode",
        "do_not_disturb_Switch",
        "power_Usage_Summary",
        "storage_Dashboard",
        "settings_wifi_switch",
        "settings_bluetooth_switch",
        "settings_hotspot_switch",
        "settings_airplane_mode_switch",
        "settings_data_switch",
        "settings_vibration_switch",
        "settings_dark_mode_switch",
        "settings_disturb_mode_switch",
        "settings_adjust_sound",
        "settings_adjust_brightness",
        "settings_eye_protection_mode_switch",
        "settings_battery_saver_switch",
        "settings_get_wifi_state",
        "settings_get_bluetooth_state",
        "settings_get_hotspot_state",
        "settings_get_dark_mode_state",
        "settings_get_current_battery_level",
        "settings_open_language",
        "settings_open_feedback"
    ));
    private static final Set<String> DIALER_TOOLS = new HashSet<>(Arrays.asList(
        "dialer_get_call_logs",
        "dialer_set_auto_record",
        "dialer_query_calllog",
        "dialer_query_calllog_of_contact",
        "dialer_query_calllog_by_type",
        "dialer_query_calllog_by_number",
        "dialer_check_last_caller",
        "dialer_contacts_query",
        "dialer_settings_open_auto_record_screen"
    ));
    private static final Set<String> SOUNDRECORDER_TOOLS = new HashSet<>(Arrays.asList(
        "soundrecorder_start_record",
        "soundrecorder_stop_record",
        "soundrecorder_get_record_list",
        "lite_recorder_open_recording",
        "lite_recorder_open_home",
        "lite_recorder_stop_recording",
        "lite_recorder_pause_recording",
        "lite_recorder_resume_recording"
    ));
    private static final Set<String> FILEMANAGER_TOOLS = new HashSet<>(Arrays.asList(
        "filemanager_search_files_by_keyword",
        "filemanager_get_document_file_list",
        "filemanager_get_voice_file_list",
        "filemanager_search_file",
        "filemanager_get_file_path",
        "filemanager_get_file_size",
        "filemanager_get_file_detail",
        "filemanager_get_download_file_list",
        "filemanager_get_weixin_file_list",
        "filemanager_get_screenshot_file_list",
        "filemanager_get_screen_recording_file_list",
        "filemanager_get_garbage_file_list",
        "filemanager_clean_junk"
    ));
    private static final Set<String> AILAUNCHER_TOOLS = new HashSet<>(Arrays.asList(
        "ailauncher_load_tripartite_common_card",
        "ailauncher_launch_alipay_cashier",
        "assistant_change_voice_id",
        "assistant_get_voice_list",
        "assistant_show_risk_statement"
    ));
    static boolean isMdiTool(String method) {
        return method != null && method.matches(ALLOWED_TOOL);
    }
    static String[] resolveServiceTarget(String toolName) {
        if (toolName == null || !toolName.matches("[a-zA-Z][a-zA-Z0-9_]{1,63}")) return null;
        if (toolName.startsWith("mdi_")) {
            return isMdiTool(toolName) ? new String[]{DEFAULT_PKG, DEFAULT_CLS} : null;
        }
        if (COMMON_ABILITY_TOOLS.contains(toolName)
                || toolName.startsWith("set_back_screen_")
                || toolName.startsWith("get_back_screen_")
                || toolName.startsWith("common_ability_")) {
            return new String[]{"com.stepos.common.ability.proxy", "com.stepos.common.ability.biz.CommonAbilityService"};
        }
        if (GALLERY_TOOLS.contains(toolName) || toolName.startsWith("gallery_")) {
            return new String[]{"com.stepos.media.gallery", "com.stepos.gallery.baselib.mcp.GalleryMCPService"};
        }
        if (ALARM_TOOLS.contains(toolName) || toolName.startsWith("alarm_clock_")) {
            return new String[]{"com.android.alarmclock", "com.stepos.alarmclock.mcp.AlarmMCPService"};
        }
        if (SETTINGS_TOOLS.contains(toolName) || toolName.startsWith("settings_")) {
            return new String[]{"com.stepos.settings", "com.stepos.settings.mcp.SettingsMcpService"};
        }
        if (DIALER_TOOLS.contains(toolName) || toolName.startsWith("dialer_")) {
            return new String[]{"com.android.dialer", "com.stepos.dialer.mcp.DialerMcpService"};
        }
        if (SOUNDRECORDER_TOOLS.contains(toolName)
                || toolName.startsWith("soundrecorder_")
                || toolName.startsWith("lite_recorder_")) {
            return new String[]{"com.android.soundrecorder", "com.stepos.soundrecorder.mcp.SoundRecorderMcpService"};
        }
        if (FILEMANAGER_TOOLS.contains(toolName) || toolName.startsWith("filemanager_")) {
            return new String[]{"com.stepos.filemanager", "com.stepos.filemanager.mcp.FileManagerMcpService"};
        }
        if (AILAUNCHER_TOOLS.contains(toolName) || toolName.startsWith("ailauncher_")) {
            return new String[]{"com.stepos.ailauncher", "com.stepos.ailauncher.agent.ability.VoiceAbilityService"};
        }
        return null;
    }
    static ComponentName resolveServiceComponent(String toolName) {
        String[] target = resolveServiceTarget(toolName);
        return target == null ? null : new ComponentName(target[0], target[1]);
    }
    static boolean isAllowedTool(String method) {
        return resolveServiceTarget(method) != null;
    }
    Object call(String method,JSONObject args)throws Exception {
        if(!isAllowedTool(method))throw new IllegalArgumentException("MDI 操作不支持");
        if(args==null)args=new JSONObject();
        if(isMdiTool(method)&&!args.optString("taskName").startsWith("stepx-mdi-"))throw new IllegalArgumentException("缺少独立任务标识");
        ComponentName required=resolveServiceComponent(method);
        if(required!=null&&!required.equals(targetComponent)){
            try(MdiTransport routed=new MdiTransport(context,required)){return routed.call(method,args);}
        }
        JSONObject result=request("tools/call",new JSONObject().put("name",method).put("arguments",args));
        JSONArray contents=result.optJSONArray("content");StringBuilder text=new StringBuilder();
        if(contents!=null)for(int i=0;i<contents.length();i++){
            JSONObject item=contents.getJSONObject(i);if("text".equals(item.optString("type")))text.append(item.optString("text"));
            if("image".equals(item.optString("type")))return new JSONObject().put("format","png").put("base64",item.getString("data"));
        }
        if(result.optBoolean("isError")&&method.equals("mdi_task_finish")){
            try{if(new JSONObject(text.toString()).optInt("code")==20102)return "already released";}catch(JSONException ignored){}
        }
        if(result.optBoolean("isError"))throw new IOException("原厂 MDI 未完成 "+method+"："+text.substring(0,Math.min(180,text.length())));
        String trimmed=text.toString().trim();
        if(trimmed.startsWith("{")){try{return new JSONObject(trimmed);}catch(JSONException ignored){}}
        if(trimmed.startsWith("[")){try{return new JSONArray(trimmed);}catch(JSONException ignored){}}
        return text.toString();
    }
    private JSONObject request(String method,JSONObject params)throws Exception {
        String id=Integer.toString(++nextId);CompletableFuture<JSONObject> f=new CompletableFuture<>();pending.put(id,f);
        try{transact(3,new JSONObject().put("jsonrpc","2.0").put("id",id).put("method",method).put("params",params).toString());
            JSONObject value=f.get(20,TimeUnit.SECONDS);if(value.has("error"))throw new IOException("原厂 MCP 拒绝请求："+value.getJSONObject("error").optString("message"));return value.getJSONObject("result");
        }catch(TimeoutException timeout){throw new IOException("原厂 MDI 请求超时（"+method+"），尚未确认完成",timeout);}
        finally{pending.remove(id);}
    }
    private void transact(int code,String body)throws Exception {
        Parcel p=Parcel.obtain();try{p.writeInterfaceToken(SERVICE);p.writeString(session);if(code==1)p.writeStrongBinder(callback);if(code==3)p.writeString(body);
            if(!remote.transact(code,p,null,IBinder.FLAG_ONEWAY))throw new IOException("原厂 MCP 协议不兼容");
        }finally{p.recycle();}
    }
    private void fail(){IOException e=new IOException("原厂 MDI 服务连接中断");connected.completeExceptionally(e);for(CompletableFuture<JSONObject> f:pending.values())f.completeExceptionally(e);}
    public void close(){if(remote!=null)try{transact(2,null);}catch(Exception ignored){}if(bound){bound=false;try{context.unbindService(connection);}catch(Exception ignored){}}fail();}
    private static void check(boolean cond, String msg) { if (!cond) throw new AssertionError(msg); }
    public static void main(String[] args) {
        check(isMdiTool("mdi_click") && isAllowedTool("mdi_click"), "mdi_click must be MDI and allowed");
        check(!isMdiTool("mdi_unknown") && !isAllowedTool("mdi_unknown"), "unknown mdi_* tool must be rejected");
        check(Arrays.equals(resolveServiceTarget("mdi_click"), new String[]{DEFAULT_PKG, DEFAULT_CLS}), "mdi_click must target TaskRobotMCPService");
        check(!isMdiTool("set_back_screen_frame") && isAllowedTool("set_back_screen_frame"), "set_back_screen_frame must be allowed non-MDI tool");
        check(Arrays.equals(resolveServiceTarget("set_back_screen_frame"), new String[]{"com.stepos.common.ability.proxy", "com.stepos.common.ability.biz.CommonAbilityService"}), "CommonAbilityService routing");
        check(Arrays.equals(resolveServiceTarget("gallery_local_search"), new String[]{"com.stepos.media.gallery", "com.stepos.gallery.baselib.mcp.GalleryMCPService"}), "GalleryMCPService routing");
        check(Arrays.equals(resolveServiceTarget("gallery_create_album"), new String[]{"com.stepos.media.gallery", "com.stepos.gallery.baselib.mcp.GalleryMCPService"}), "GalleryMCPService prefix routing");
        check(Arrays.equals(resolveServiceTarget("alarm_clock_add"), new String[]{"com.android.alarmclock", "com.stepos.alarmclock.mcp.AlarmMCPService"}), "AlarmMCPService routing");
        check(Arrays.equals(resolveServiceTarget("wifi_Switch"), new String[]{"com.stepos.settings", "com.stepos.settings.mcp.SettingsMcpService"}), "SettingsMcpService routing");
        check(Arrays.equals(resolveServiceTarget("settings_get_airplane_mode_state"), new String[]{"com.stepos.settings", "com.stepos.settings.mcp.SettingsMcpService"}), "SettingsMcpService prefix routing");
        check(Arrays.equals(resolveServiceTarget("dialer_get_call_logs"), new String[]{"com.android.dialer", "com.stepos.dialer.mcp.DialerMcpService"}), "DialerMcpService routing");
        check(Arrays.equals(resolveServiceTarget("soundrecorder_start_record"), new String[]{"com.android.soundrecorder", "com.stepos.soundrecorder.mcp.SoundRecorderMcpService"}), "SoundRecorderMcpService routing");
        check(Arrays.equals(resolveServiceTarget("filemanager_search_files_by_keyword"), new String[]{"com.stepos.filemanager", "com.stepos.filemanager.mcp.FileManagerMcpService"}), "FileManagerMcpService routing");
        check(Arrays.equals(resolveServiceTarget("assistant_get_voice_list"), new String[]{"com.stepos.ailauncher", "com.stepos.ailauncher.agent.ability.VoiceAbilityService"}), "VoiceAbilityService routing");
        check(!isAllowedTool("rm_rf") && resolveServiceTarget("rm_rf") == null, "unknown tool must be rejected");
        check(!isAllowedTool("gallery_../bad"), "invalid characters in tool name must be rejected");
        System.out.println("MdiTransport multi-service routing test: passed");
    }
}
