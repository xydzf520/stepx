package com.stepx.companion;

import android.Manifest;
import android.accessibilityservice.AccessibilityServiceInfo;
import android.app.Activity;
import android.app.KeyguardManager;
import android.app.PendingIntent;
import android.app.UiAutomation;
import android.app.usage.UsageEvents;
import android.app.usage.UsageStats;
import android.app.usage.UsageStatsManager;
import android.content.BroadcastReceiver;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import android.graphics.Rect;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.Looper;
import android.provider.Settings;
import android.telephony.SmsManager;
import android.telephony.SubscriptionInfo;
import android.telephony.SubscriptionManager;
import android.view.accessibility.AccessibilityNodeInfo;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.*;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

/** Small Android API adapter, run as the already-authorized ADB shell UID. */
public final class Bridge {
    static final String PKG = "com.stepos.ailauncher";
    static final String ROOT = "/data/local/tmp/stepx-companion";
    static Context context;
    static HandlerThread callbacks;
    static UiAutomation automation;

    private static synchronized Handler callbacksHandler() {
        if (callbacks == null) {
            callbacks = new HandlerThread("stepx-bridge");
            callbacks.start();
        }
        return new Handler(callbacks.getLooper());
    }

    public static void main(String[] args) {
        try {
            Looper.prepareMainLooper();
            Class<?> at = Class.forName("android.app.ActivityThread");
            Object thread = at.getMethod("systemMain").invoke(null);
            Context system = (Context) at.getMethod("getSystemContext").invoke(thread);
            context = system.createPackageContext("com.android.shell", 0);
            if (args != null && args.length == 2 && "--call-audio".equals(args[0])) {
                // stdout is binary PCM in this mode. Diagnostics must never enter it.
                try { CallAudioTools.run(args[1]); }
                catch (Throwable error) {
                    while (error.getCause() != null) error = error.getCause();
                    System.err.println("STEPX_CALL_AUDIO_ERROR:" + error.getMessage());
                }
                return;
            }
            if (args != null && args.length > 0 && "--backscreen-loop".equals(args[0])) {
                BackScreenMcpTools.runLoop(args);
                return;
            }
            String line = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8)).readLine();
            if (line == null || line.length() > 32768) throw new IllegalArgumentException("请求为空或过长");
            JSONObject request = new JSONObject(line);
            JSONObject result = dispatch(request.getString("op"), request);
            result.put("ok", true);
            System.out.println("STEPX_RESULT:" + result);
        } catch (Throwable error) {
            while (error.getCause() != null) error = error.getCause();
            try { System.out.println("STEPX_RESULT:" + new JSONObject().put("ok", false)
                .put("error", error.getMessage() == null ? error.getClass().getSimpleName() : error.getMessage())); }
            catch (Throwable ignored) {}
        } finally {
            if (automation != null) try { UiAutomation.class.getMethod("disconnect").invoke(automation); } catch (Throwable ignored) {}
            if (callbacks != null) callbacks.quitSafely();
            System.exit(0);
        }
    }

    static JSONObject dispatch(String op, JSONObject request) throws Exception {
        switch (op) {
            case "batch": return batch(request);
            case "apps_list": return AppTools.list(request);
            case "app_info": return AppTools.info(request);
            case "phone_accounts": return DialTools.inspectAccounts();
            case "phone_prepare": return DialTools.prepare(request);
            case "phone_call": return DialTools.call(request);
            case "phone_control": return DialTools.control(request);
            case "phone_audio_capabilities": return CallAudioTools.inspect();
            case "storage_scan": return StorageTools.scan();
            case "storage_file_operation": return FileTools.run(request);
            case "storage_clear_cache": return FileTools.clearCache();
            case "read_sms": return readSms(request);
            case "read_calendar": return readCalendarPage(request);
            // Exact, bounded readback for MCP-created alarms; does not truncate
            // silently to the 20-row home overview and never writes a provider.
            case "read_alarms": return queryRows("content://com.android.alarmclock/alarms",
                new String[]{"_id", "hour", "minutes", "daysofweek", "enabled", "vibrate", "label"},
                "_id ASC", null, null, 1000);
            case "read_calls": return queryRows("content://call_log/calls", new String[]{"number", "name", "type", "date", "duration"}, "date DESC, _id DESC");
            case "read_contacts": return readContacts();
            case "create_contact": return createContact(request);
            case "capabilities": return capabilities();
            case "amoo_context": return amooContext();
            case "amoo_snapshot": return snapshot(request.optBoolean("debug", false));
            case "amoo_sync": return syncAmoo(request.optString("select_title", ""), request.optInt("select_index", -1));
            case "amoo_new_chat": return newChatAmoo();
            case "amoo_prepare": return prepareAmoo(request.getString("text"));
            case "amoo_send": return sendAmoo(request.getString("text"));
            case "sms_prepare": return prepareSms(request);
            case "sms_send": return sendSms(request);
            case "sms_status": return readStatus(request.getString("id"));
            case "clipboard_get": return clipboardGet();
            case "clipboard_set": return clipboardSet(request.optString("text", ""));
            case "stepos_usage":
            case "ai_suite": return steposUsage();
            case "read_recordings": return new JSONObject().put("rows", queryRecentRecordings(new JSONArray()));
            case "calendar_create": return calendarCreate(request);
            case "calendar_delete": return calendarDelete(request);
            case "alarm_create": return alarmCreate(request);
            case "alarm_toggle": return alarmToggle(request);
            case "alarm_delete": return alarmDelete(request);
            case "backscreen_mcp_status": return BackScreenMcpTools.status();
            case "backscreen_mcp_stop": return BackScreenMcpTools.stop();
            default: throw new IllegalArgumentException("未知操作");
        }
    }

    static JSONObject batch(JSONObject request) throws Exception {
        JSONArray ops = request.optJSONArray("ops");
        if (ops == null) throw new IllegalArgumentException("缺少批处理操作列表");
        if (ops.length() > 32) throw new IllegalArgumentException("批处理操作过多");
        JSONObject results = new JSONObject();
        for (int i = 0; i < ops.length(); i++) {
            JSONObject item = ops.getJSONObject(i);
            String subOp = item.getString("op");
            if ("batch".equals(subOp)) throw new IllegalArgumentException("不支持嵌套批处理");
            String id = item.optString("id", "").trim();
            if (id.isEmpty()) id = subOp;
            try {
                JSONObject res = dispatch(subOp, item);
                res.put("ok", true);
                results.put(id, res);
            } catch (Throwable subErr) {
                while (subErr.getCause() != null) subErr = subErr.getCause();
                String msg = subErr.getMessage() == null ? subErr.getClass().getSimpleName() : subErr.getMessage();
                results.put(id, new JSONObject().put("ok", false).put("error", msg));
            }
        }
        return new JSONObject().put("results", results);
    }

    // Cursor values are JSON encoded individually: message text can contain newlines,
    // commas and even strings identical to the shell `content query` row delimiters.
    static JSONObject queryRows(String uri, String[] columns, String order) throws Exception {
        return queryRows(uri, columns, order, null, null);
    }

    static JSONObject queryRows(String uri, String[] columns, String order, String selection) throws Exception {
        return queryRows(uri, columns, order, selection, null);
    }

    static JSONObject queryRows(String uri, String[] columns, String order, String[] args) throws Exception {
        return queryRows(uri, columns, order, null, args);
    }

    static JSONObject queryRows(String uri, String[] columns, String order, String selection, String[] args) throws Exception {
        return queryRows(uri, columns, order, selection, args, 0);
    }

    static JSONObject queryRows(String uri, String[] columns, String order, String selection, String[] args, int limit) throws Exception {
        // app_process has no registered ApplicationThread. Use the external provider
        // handle, as Android's `content` shell command does, with shell attribution.
        Uri target = Uri.parse(uri);
        Class<?> am = Class.forName("android.app.ActivityManager");
        Class<?> iam = Class.forName("android.app.IActivityManager");
        Object service = am.getMethod("getService").invoke(null);
        int user = (Integer) am.getMethod("getCurrentUser").invoke(null);
        android.os.IBinder token = new android.os.Binder();
        Object holder = iam.getMethod("getContentProviderExternal", String.class, int.class,
            android.os.IBinder.class, String.class).invoke(service, target.getAuthority(), user, token, "*cmd*");
        if (holder == null) throw new IOException("手机数据提供程序不可用");
        try {
            Object provider = holder.getClass().getField("provider").get(holder);
            Class<?> ipc = Class.forName("android.content.IContentProvider");
            Class<?> signal = Class.forName("android.os.ICancellationSignal");
            android.content.AttributionSource attribution = new android.content.AttributionSource.Builder(android.os.Process.myUid())
                .setPackageName("com.android.shell").build();
            Bundle extras = new Bundle();
            if (order != null) extras.putString(android.content.ContentResolver.QUERY_ARG_SQL_SORT_ORDER, order);
            if (selection != null && !selection.isEmpty()) extras.putString(android.content.ContentResolver.QUERY_ARG_SQL_SELECTION, selection);
            if (args != null) extras.putStringArray(android.content.ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS, args);
            JSONArray rows = new JSONArray();
            boolean hasMore = false;
            try (Cursor cursor = (Cursor) ipc.getMethod("query", android.content.AttributionSource.class,
                Uri.class, String[].class, Bundle.class, signal).invoke(provider, attribution, target, columns, extras, null)) {
                if (cursor == null) throw new IOException("手机数据提供程序未返回结果");
                int[] colIndices = new int[columns.length];
                for (int i = 0; i < columns.length; i++) {
                    colIndices[i] = cursor.getColumnIndexOrThrow(columns[i]);
                }
                while (cursor.moveToNext()) {
                    if (limit > 0 && rows.length() >= limit) {
                        hasMore = true;
                        break;
                    }
                    JSONObject row = new JSONObject();
                    for (int i = 0; i < columns.length; i++) {
                        int index = colIndices[i];
                        row.put(columns[i], cursor.isNull(index) ? JSONObject.NULL : cursor.getString(index));
                    }
                    rows.put(row);
                }
            }
            return new JSONObject().put("rows", rows).put("has_more", hasMore);
        } finally {
            iam.getMethod("removeContentProviderExternalAsUser", String.class, android.os.IBinder.class, int.class)
                .invoke(service, target.getAuthority(), token, user);
        }
    }

    static JSONObject readContacts() throws Exception {
        JSONArray merged = new JSONArray();
        Set<String> seen = new HashSet<>();
        try {
            JSONArray phoneRows = queryRows(
                "content://com.android.contacts/data/phones",
                new String[]{"display_name", "data1"},
                "display_name ASC"
            ).optJSONArray("rows");
            if (phoneRows != null) {
                for (int i = 0; i < phoneRows.length(); i++) {
                    JSONObject r = phoneRows.optJSONObject(i);
                    if (r == null) continue;
                    String name = r.optString("display_name", "").trim();
                    String num = r.optString("data1", "").trim();
                    if (num.isEmpty() || "null".equalsIgnoreCase(num)) continue;
                    if ("null".equalsIgnoreCase(name)) name = "";
                    String key = num.replaceAll("[^0-9+]", "");
                    if (key.isEmpty()) key = num;
                    if (seen.add(key)) {
                        merged.put(new JSONObject()
                            .put("display_name", name.isEmpty() ? num : name)
                            .put("data1", num));
                    }
                }
            }
        } catch (Throwable ignored) {
        }
        String[] simUris = new String[]{
            "content://icc/adn",
            "content://icc/adn/subId/1",
            "content://icc/adn/subId/2",
            "content://icc/sdn"
        };
        for (String simUri : simUris) {
            try {
                JSONArray simRows = queryRows(
                    simUri,
                    new String[]{"name", "number"},
                    null
                ).optJSONArray("rows");
                if (simRows == null) continue;
                for (int i = 0; i < simRows.length(); i++) {
                    JSONObject r = simRows.optJSONObject(i);
                    if (r == null) continue;
                    String name = r.optString("name", "").trim();
                    String num = r.optString("number", "").trim();
                    if (num.isEmpty() || "null".equalsIgnoreCase(num)) continue;
                    if ("null".equalsIgnoreCase(name)) name = "";
                    String key = num.replaceAll("[^0-9+]", "");
                    if (key.isEmpty()) key = num;
                    if (seen.add(key)) {
                        merged.put(new JSONObject()
                            .put("display_name", name.isEmpty() ? num : name)
                            .put("data1", num));
                    }
                }
            } catch (Throwable ignored) {
            }
        }
        return new JSONObject().put("rows", merged).put("has_more", false);
    }

    static JSONObject createContact(JSONObject req) throws Exception {
        String name = req.optString("name", "").trim();
        String number = req.optString("number", "").trim();
        if (name.isEmpty() || number.isEmpty()) {
            throw new IllegalArgumentException("请填写联系人姓名与电话号码");
        }
        String normTarget = number.replaceAll("[^0-9+]", "");
        if (normTarget.isEmpty()) normTarget = number;

        // 1. Check if a phone_v2 row with this number already exists on a non-deleted raw_contact
        long existingRawContactId = 0L;
        try {
            JSONArray phoneRows = queryRows(
                "content://com.android.contacts/data/phones",
                new String[]{"raw_contact_id", "data1"},
                "_id DESC"
            ).optJSONArray("rows");
            if (phoneRows != null) {
                for (int i = 0; i < phoneRows.length(); i++) {
                    JSONObject r = phoneRows.optJSONObject(i);
                    if (r == null) continue;
                    String num = r.optString("data1", "").trim();
                    String norm = num.replaceAll("[^0-9+]", "");
                    if (norm.isEmpty()) norm = num;
                    if (norm.equals(normTarget)) {
                        existingRawContactId = r.optLong("raw_contact_id", 0L);
                        if (existingRawContactId > 0L) break;
                    }
                }
            }
        } catch (Throwable ignored) {
        }

        if (existingRawContactId > 0L) {
            ContentValues nameValues = new ContentValues();
            nameValues.put("data1", name);
            nameValues.put("data2", name);
            int updated = updateRow(
                "content://com.android.contacts/data",
                nameValues,
                "raw_contact_id=" + existingRawContactId + " AND mimetype='vnd.android.cursor.item/name'"
            );
            if (updated <= 0) {
                ContentValues insertName = new ContentValues();
                insertName.put("raw_contact_id", existingRawContactId);
                insertName.put("mimetype", "vnd.android.cursor.item/name");
                insertName.put("data1", name);
                insertName.put("data2", name);
                insertRow("content://com.android.contacts/data", insertName);
            }
            ContentValues phoneValues = new ContentValues();
            phoneValues.put("data1", number);
            updateRow(
                "content://com.android.contacts/data",
                phoneValues,
                "raw_contact_id=" + existingRawContactId + " AND mimetype='vnd.android.cursor.item/phone_v2'"
            );
            return new JSONObject()
                .put("saved", true)
                .put("updated", true)
                .put("raw_contact_id", existingRawContactId)
                .put("contacts", readContacts().optJSONArray("rows"));
        }

        // 2. Insert a brand new raw_contact + StructuredName + Phone row
        ContentValues rawValues = new ContentValues();
        rawValues.putNull("account_type");
        rawValues.putNull("account_name");
        Uri rawUri = insertRow("content://com.android.contacts/raw_contacts", rawValues);
        long rawContactId = 0L;
        if (rawUri != null && rawUri.getLastPathSegment() != null) {
            try {
                rawContactId = Long.parseLong(rawUri.getLastPathSegment());
            } catch (NumberFormatException ignored) {
            }
        }
        if (rawContactId <= 0L) {
            JSONArray rawRows = queryRows(
                "content://com.android.contacts/raw_contacts",
                new String[]{"_id"},
                "_id DESC",
                "deleted=0",
                null,
                1
            ).optJSONArray("rows");
            if (rawRows != null && rawRows.length() > 0) {
                rawContactId = rawRows.getJSONObject(0).optLong("_id", 0L);
            }
        }
        if (rawContactId <= 0L) {
            throw new IOException("无法创建手机通讯录记录");
        }

        ContentValues nameValues = new ContentValues();
        nameValues.put("raw_contact_id", rawContactId);
        nameValues.put("mimetype", "vnd.android.cursor.item/name");
        nameValues.put("data1", name);
        nameValues.put("data2", name);
        insertRow("content://com.android.contacts/data", nameValues);

        ContentValues phoneValues = new ContentValues();
        phoneValues.put("raw_contact_id", rawContactId);
        phoneValues.put("mimetype", "vnd.android.cursor.item/phone_v2");
        phoneValues.put("data1", number);
        phoneValues.put("data2", 2);
        insertRow("content://com.android.contacts/data", phoneValues);

        return new JSONObject()
            .put("saved", true)
            .put("raw_contact_id", rawContactId)
            .put("contacts", readContacts().optJSONArray("rows"));
    }

    static JSONObject readSms(JSONObject request) throws Exception {
        String[] columns = new String[]{"_id", "address", "body", "date", "type", "read", "sub_id"};
        if (!request.has("after_date_ms") && !request.has("limit")) {
            return queryRows("content://sms", columns, "date DESC, _id DESC");
        }
        long date = Math.max(0L, request.optLong("after_date_ms", 0L));
        long id = Math.max(0L, request.optLong("after_id", 0L));
        int limit = ContextReadPolicy.pageLimit(request.optInt("limit", 500));
        JSONObject result = queryRows("content://sms", columns, "date ASC, _id ASC",
            "date > ? OR (date = ? AND _id > ?)",
            new String[]{String.valueOf(date), String.valueOf(date), String.valueOf(id)}, limit);
        return result.put("incremental", true).put("capture_ms", System.currentTimeMillis());
    }

    static JSONObject readCalendarPage(JSONObject request) throws Exception {
        long now = System.currentTimeMillis();
        long start = request.optLong("start_ms", ContextReadPolicy.calendarStart(now));
        long end = request.optLong("end_ms", ContextReadPolicy.calendarEnd(now));
        ContextReadPolicy.checkCalendarWindow(start, end);
        long after = Math.max(0L, request.optLong("after_begin_ms", 0L));
        long afterId = Math.max(0L, request.optLong("after_id", 0L));
        String uri = "content://com.android.calendar/instances/when/" + start + "/" + end;
        JSONObject result = queryRows(uri,
            new String[]{"_id", "event_id", "title", "description", "eventLocation", "begin", "end", "allDay", "rrule", "rdate"},
            // CalendarProvider joins Instances and Events, both with an _id column.
            // Qualify the paging key in SQL; the projected result remains named _id.
            "begin ASC, Instances._id ASC", "begin > ? OR (begin = ? AND Instances._id > ?)",
            new String[]{String.valueOf(after), String.valueOf(after), String.valueOf(afterId)},
            ContextReadPolicy.pageLimit(request.optInt("limit", 500)));
        return result.put("start_ms", start).put("end_ms", end).put("incremental", true);
    }

    // Same public OpenId provider used by Amoo. No account token or private DB.
    static JSONObject amooContext() throws Exception {
        JSONArray rows = queryRows("content://com.stepos.secureservice.openidsdk/",
            new String[]{"value", "code"}, null, new String[]{"oaid"}).getJSONArray("rows");
        // The native SDK consumes a nonempty value even when provider code != 1000
        // (StepOS reports 1). The value, not that vendor-specific code, is the ID.
        if (rows.length() != 1)
            throw new IOException("无法读取当前手机的 Amoo 设备标识，消息未发送");
        String id = rows.getJSONObject(0).optString("value", "");
        if (id.isEmpty()) throw new IOException("Amoo 设备标识为空，消息未发送");
        boolean locked = context.getSystemService(KeyguardManager.class).isKeyguardLocked();
        return new JSONObject().put("device_id", id)
            .put("device_model", android.os.Build.MODEL)
            .put("os_version", android.os.Build.VERSION.RELEASE)
            .put("app_version", context.getPackageManager().getPackageInfo(PKG, 0).versionName)
            .put("rawScreenLockState", locked ? "0" : "1")
            .put("screenLockState", locked ? "0" : "1");
    }

    static JSONObject capabilities() throws Exception {
        // app_process does not run SystemServer's telephony bootstrap.
        Class<?> initializer = Class.forName("android.telephony.TelephonyFrameworkInitializer");
        if (initializer.getMethod("getTelephonyServiceManager").invoke(null) == null) {
            Class<?> manager = Class.forName("android.os.TelephonyServiceManager");
            initializer.getMethod("setTelephonyServiceManager", manager)
                .invoke(null, manager.getConstructor().newInstance());
        }
        JSONObject out = new JSONObject();
        out.put("send_sms", context.checkSelfPermission(Manifest.permission.SEND_SMS) == PackageManager.PERMISSION_GRANTED);
        out.put("locked", context.getSystemService(KeyguardManager.class).isKeyguardLocked());
        out.put("default_subscription", SubscriptionManager.getDefaultSmsSubscriptionId());
        JSONArray sims = new JSONArray();
        List<SubscriptionInfo> list = context.getSystemService(SubscriptionManager.class).getActiveSubscriptionInfoList();
        if (list != null) for (SubscriptionInfo sim : list) sims.put(new JSONObject()
            .put("id", sim.getSubscriptionId()).put("slot", sim.getSimSlotIndex() + 1)
            .put("label", String.valueOf(sim.getDisplayName())));
        return out.put("subscriptions", sims).put("protocol", 1);
    }

    static void connectUi() throws Exception {
        if (automation != null) return;
        Class<?> connectionType = Class.forName("android.app.IUiAutomationConnection");
        Object connection = Class.forName("android.app.UiAutomationConnection").getConstructor().newInstance();
        automation = (UiAutomation) UiAutomation.class.getConstructor(Looper.class, connectionType)
            .newInstance(callbacksHandler().getLooper(), connection);
        UiAutomation.class.getMethod("connect", int.class).invoke(automation, 1);
        AccessibilityServiceInfo info = automation.getServiceInfo();
        info.flags |= AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS | AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS;
        automation.setServiceInfo(info);
    }

    static AccessibilityNodeInfo root() throws Exception {
        connectUi();
        for (int i = 0; i < 12; i++) {
            AccessibilityNodeInfo root = automation.getRootInActiveWindow();
            if (root != null && PKG.contentEquals(root.getPackageName())) return root;
            Thread.sleep(100);
        }
        throw new IllegalStateException("手机当前不在 Amoo 对话页，请点击连接 Amoo 后重试");
    }

    static AccessibilityNodeInfo find(AccessibilityNodeInfo root, String id) {
        List<AccessibilityNodeInfo> nodes = root.findAccessibilityNodeInfosByViewId(PKG + ":id/" + id);
        for (AccessibilityNodeInfo node : nodes) if (node.isVisibleToUser()) return node;
        return null;
    }

    static void collect(AccessibilityNodeInfo node, JSONArray result, int depth) throws Exception {
        if (node == null || depth > 35 || result.length() >= 1000 || node.isPassword() || !node.isVisibleToUser()) return;
        String id = node.getViewIdResourceName();
        CharSequence text = node.getText(), desc = node.getContentDescription();
        if (id != null || text != null || desc != null) result.put(new JSONObject()
            .put("id", id == null ? "" : id).put("text", text == null ? "" : text.toString())
            .put("description", desc == null ? "" : desc.toString())
            .put("class", String.valueOf(node.getClassName())).put("clickable", node.isClickable())
            .put("enabled", node.isEnabled()).put("depth", depth));
        for (int i = 0; i < node.getChildCount(); i++) collect(node.getChild(i), result, depth + 1);
    }

    static JSONObject snapshot(boolean debug) throws Exception {
        AccessibilityNodeInfo root = root();
        AccessibilityNodeInfo closeBtn = find(root, "btn_close");
        if (closeBtn != null && closeBtn.isClickable()) {
            closeBtn.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            Thread.sleep(250);
            root = root();
        }
        AccessibilityNodeInfo list = find(root, "chat_list");
        if (list == null) throw new IllegalStateException("未找到 Amoo 消息列表，请打开聊天页");
        List<AccessibilityNodeInfo> children = new ArrayList<>();
        for (int i = 0; i < list.getChildCount(); i++) {
            AccessibilityNodeInfo c = list.getChild(i);
            if (c != null && c.isVisibleToUser()) children.add(c);
        }
        children.sort((a, b) -> {
            Rect ra = new Rect(), rb = new Rect();
            a.getBoundsInScreen(ra);
            b.getBoundsInScreen(rb);
            return Integer.compare(ra.top, rb.top);
        });
        JSONArray rows = new JSONArray();
        for (AccessibilityNodeInfo child : children) {
            JSONArray nodes = new JSONArray(); collect(child, nodes, 0);
            if (nodes.length() > 0) rows.put(nodes);
        }
        JSONObject result = new JSONObject().put("rows", rows);
        if (debug) { JSONArray nodes = new JSONArray(); collect(root, nodes, 0); result.put("nodes", nodes); }
        return result;
    }

    static JSONObject syncAmoo(String selectTitle, int selectIndex) throws Exception {
        AccessibilityNodeInfo root = root();
        AccessibilityNodeInfo closeBtn = find(root, "btn_close");
        if (closeBtn != null && closeBtn.isClickable()) {
            closeBtn.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            Thread.sleep(250);
            root = root();
        }
        AccessibilityNodeInfo history = find(root, "rv_chat_history");
        if (history == null) {
            AccessibilityNodeInfo menu = find(root, "iv_menu");
            if (menu != null && menu.isClickable()) {
                menu.performAction(AccessibilityNodeInfo.ACTION_CLICK);
                for (int i = 0; i < 8; i++) {
                    Thread.sleep(100);
                    root = root();
                    history = find(root, "rv_chat_history");
                    if (history != null) break;
                }
            }
        }
        JSONArray sessions = new JSONArray();
        String chosenTitle = "";
        if (history != null) {
            String group = "今天";
            List<AccessibilityNodeInfo> clickTargets = new ArrayList<>();
            List<String> titles = new ArrayList<>();
            for (int i = 0; i < history.getChildCount(); i++) {
                AccessibilityNodeInfo child = history.getChild(i);
                if (child == null || !child.isVisibleToUser()) continue;
                String id = child.getViewIdResourceName();
                CharSequence directText = child.getText();
                if ("android:id/text1".equals(id) && directText != null && directText.length() > 0) {
                    group = directText.toString().trim();
                    continue;
                }
                List<AccessibilityNodeInfo> items = child.findAccessibilityNodeInfosByViewId(PKG + ":id/ll_drawer_item_content");
                if (items == null || items.isEmpty()) continue;
                AccessibilityNodeInfo itemNode = items.get(0);
                List<AccessibilityNodeInfo> titleNodes = itemNode.findAccessibilityNodeInfosByViewId("android:id/text1");
                if (titleNodes == null || titleNodes.isEmpty()) continue;
                CharSequence t = titleNodes.get(0).getText();
                String title = t == null ? "" : t.toString().trim();
                if (title.isEmpty()) continue;
                List<AccessibilityNodeInfo> statusIcons = itemNode.findAccessibilityNodeInfosByViewId(PKG + ":id/iv_session_status");
                boolean hasStatusIcon = statusIcons != null && !statusIcons.isEmpty() && statusIcons.get(0).isVisibleToUser();
                sessions.put(new JSONObject()
                    .put("title", title)
                    .put("group", group)
                    .put("has_status_icon", hasStatusIcon));
                clickTargets.add(itemNode);
                titles.add(title);
            }
            int targetIdx = -1;
            if (selectTitle != null && !selectTitle.trim().isEmpty()) {
                String wanted = selectTitle.trim();
                for (int i = 0; i < titles.size(); i++) {
                    if (titles.get(i).equals(wanted)) { targetIdx = i; break; }
                }
            }
            if (targetIdx < 0 && selectIndex >= 0 && selectIndex < clickTargets.size()) {
                targetIdx = selectIndex;
            }
            if (targetIdx < 0 && !clickTargets.isEmpty()) {
                targetIdx = 0;
            }
            if (targetIdx >= 0 && targetIdx < clickTargets.size()) {
                chosenTitle = titles.get(targetIdx);
                clickTargets.get(targetIdx).performAction(AccessibilityNodeInfo.ACTION_CLICK);
                Thread.sleep(450);
            }
        }
        JSONObject snap = snapshot(false);
        snap.put("sessions", sessions);
        snap.put("selected_title", chosenTitle);
        return snap;
    }

    static JSONObject newChatAmoo() throws Exception {
        AccessibilityNodeInfo root = root();
        AccessibilityNodeInfo drawerNew = find(root, "btn_slide_panel_new_chat");
        if (drawerNew != null && drawerNew.isClickable()) {
            drawerNew.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            Thread.sleep(350);
            return snapshot(false);
        }
        AccessibilityNodeInfo topNew = find(root, "btn_new_session");
        if (topNew != null && topNew.isClickable()) {
            topNew.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            Thread.sleep(350);
            return snapshot(false);
        }
        return snapshot(false);
    }

    static JSONObject prepareAmoo(String text) throws Exception {
        if (text.trim().isEmpty() || text.length() > 8000) throw new IllegalArgumentException("问题不能为空，最多 8000 字");
        AccessibilityNodeInfo root = root();
        AccessibilityNodeInfo history = find(root, "rv_chat_history");
        if (history != null) {
            List<AccessibilityNodeInfo> items = history.findAccessibilityNodeInfosByViewId(PKG + ":id/ll_drawer_item_content");
            if (items != null && !items.isEmpty()) {
                items.get(0).performAction(AccessibilityNodeInfo.ACTION_CLICK);
                Thread.sleep(350);
                root = root();
            }
        }
        AccessibilityNodeInfo input = find(root, "et_input_bar");
        if (input == null || !input.isEditable()) throw new IllegalStateException("未找到可编辑的 Amoo 输入框");
        Bundle arguments = new Bundle(); arguments.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text);
        if (!input.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, arguments)) throw new IllegalStateException("Amoo 未接受输入，请重试");
        Thread.sleep(250);
        AccessibilityNodeInfo updated = find(root(), "et_input_bar");
        if (updated == null || !text.contentEquals(updated.getText())) throw new IllegalStateException("Amoo 输入核对失败，未发送");
        return snapshot(true).put("prepared", true);
    }

    static JSONObject sendAmoo(String text) throws Exception {
        prepareAmoo(text);
        // Resource name verified against the installed StepX build before enabling sending.
        AccessibilityNodeInfo send = find(root(), "btn_send");
        if (send == null || !send.isEnabled() || !send.isClickable()) throw new IllegalStateException("未找到可用的 Amoo 发送按钮，草稿保留在手机中");
        if (!send.performAction(AccessibilityNodeInfo.ACTION_CLICK)) throw new IllegalStateException("Amoo 未接受发送操作，未自动重试");
        return new JSONObject().put("submitted", true);
    }

    static int subscription(JSONObject request, JSONObject caps) throws Exception {
        JSONArray sims = caps.getJSONArray("subscriptions");
        int selected = request.optInt("subscription", -1);
        if (selected < 0) selected = SubscriptionManager.getDefaultSmsSubscriptionId();
        if (selected < 0 && sims.length() == 1) selected = sims.getJSONObject(0).getInt("id");
        for (int i = 0; i < sims.length(); i++) if (sims.getJSONObject(i).getInt("id") == selected) return selected;
        throw new IllegalStateException("请选择可用的发信 SIM 卡");
    }

    static SmsManager smsManager(int subscription) {
        return context.getSystemService(SmsManager.class).createForSubscriptionId(subscription);
    }

    static JSONObject prepareSms(JSONObject request) throws Exception {
        String number = request.getString("number").trim(), body = request.getString("body");
        if (!number.matches("\\+?[0-9][0-9 ()-]{1,39}")) throw new IllegalArgumentException("收件人号码无效");
        if (body.trim().isEmpty() || body.length() > 2000) throw new IllegalArgumentException("短信正文不能为空，最多 2000 字");
        JSONObject caps = capabilities();
        if (!caps.getBoolean("send_sms")) throw new SecurityException("手机未授予调试通道 SEND_SMS 权限");
        int sub = subscription(request, caps);
        String label = "SIM " + sub;
        JSONArray sims = caps.optJSONArray("subscriptions");
        if (sims != null) {
            for (int i = 0; i < sims.length(); i++) {
                JSONObject sim = sims.getJSONObject(i);
                if (sim.optInt("id", -1) == sub) {
                    String l = sim.optString("label", "").trim();
                    if (!l.isEmpty()) label = l;
                    break;
                }
            }
        }
        int parts = smsManager(sub).divideMessage(body).size();
        return new JSONObject().put("subscription", sub).put("subscription_label", label).put("parts", parts);
    }

    static File statusFile(String id) {
        if (!id.matches("[a-f0-9]{32}")) throw new IllegalArgumentException("发送请求标识无效");
        File dir = new File(ROOT); dir.mkdirs(); dir.setReadable(false, false); dir.setWritable(false, false); dir.setExecutable(false, false);
        dir.setReadable(true, true); dir.setWritable(true, true); dir.setExecutable(true, true);
        return new File(dir, "sms-" + id + ".json");
    }

    static synchronized void writeStatus(File file, JSONObject value) throws Exception {
        File pending = new File(file.getPath() + ".pending");
        try (FileOutputStream out = new FileOutputStream(pending)) { out.write(value.toString().getBytes(StandardCharsets.UTF_8)); out.getFD().sync(); }
        pending.setReadable(false, false); pending.setWritable(false, false); pending.setReadable(true, true); pending.setWritable(true, true);
        if (!pending.renameTo(file)) throw new IOException("不能保存发送状态");
    }

    static JSONObject readStatus(String id) throws Exception {
        File file = statusFile(id);
        if (!file.exists()) return new JSONObject().put("id", id).put("state", "not_found");
        if (file.length() == 0) return new JSONObject().put("id", id).put("state", "unknown");
        return new JSONObject(new String(Files.readAllBytes(file.toPath()), StandardCharsets.UTF_8));
    }

    static JSONObject sendSms(JSONObject request) throws Exception {
        JSONObject plan = prepareSms(request);
        String id = request.getString("id"); File file = statusFile(id);
        if (!file.createNewFile()) return readStatus(id); // Never retransmit an existing request.
        final JSONObject state = new JSONObject().put("id", id).put("state", "sending")
            .put("parts", plan.getInt("parts")).put("completed", 0).put("updated_ms", System.currentTimeMillis());
        writeStatus(file, state);
        ArrayList<String> parts = smsManager(plan.getInt("subscription")).divideMessage(request.getString("body"));
        CountDownLatch latch = new CountDownLatch(parts.size()); AtomicInteger completed = new AtomicInteger();
        final Set<Integer> received = new HashSet<>();
        String action = "com.stepx.companion.SMS_SENT_" + id;
        BroadcastReceiver receiver = new BroadcastReceiver() {
            public void onReceive(Context unused, Intent intent) {
                synchronized (state) {
                    try {
                        int part = intent.getIntExtra("part", -1);
                        if (part < 0 || !received.add(part)) return;
                        int result = getResultCode();
                        if (result != Activity.RESULT_OK) state.put("error_code", result);
                        int count = completed.incrementAndGet(); state.put("completed", count);
                        if (count == parts.size()) state.put("state", state.has("error_code") ? "failed" : "sent");
                        state.put("updated_ms", System.currentTimeMillis()); writeStatus(file, state);
                    } catch (Exception ignored) {} finally { latch.countDown(); }
                }
            }
        };
        context.registerReceiver(receiver, new IntentFilter(action), null, callbacksHandler(), Context.RECEIVER_NOT_EXPORTED);
        try {
            ArrayList<PendingIntent> intents = new ArrayList<>();
            for (int i = 0; i < parts.size(); i++) intents.add(PendingIntent.getBroadcast(context, i,
                new Intent(action).setPackage("com.android.shell").putExtra("part", i), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_ONE_SHOT));
            smsManager(plan.getInt("subscription")).sendMultipartTextMessage(request.getString("number"), null, parts, intents, null);
            if (!latch.await(45, TimeUnit.SECONDS)) synchronized (state) {
                state.put("state", "unknown"); state.put("updated_ms", System.currentTimeMillis()); writeStatus(file, state);
            }
        } catch (Throwable error) {
            synchronized (state) { state.put("state", "unknown").put("error", error.getClass().getSimpleName()); writeStatus(file, state); }
        } finally { context.unregisterReceiver(receiver); }
        return readStatus(id);
    }

    static JSONObject clipboardGet() throws Exception {
        String text = "";
        try {
            ClipboardManager cm = (ClipboardManager) context.getSystemService(Context.CLIPBOARD_SERVICE);
            if (cm != null && cm.hasPrimaryClip()) {
                ClipData clip = cm.getPrimaryClip();
                if (clip != null && clip.getItemCount() > 0 && clip.getItemAt(0) != null) {
                    CharSequence cs = clip.getItemAt(0).coerceToText(context);
                    if (cs != null) text = cs.toString();
                }
            }
        } catch (Throwable ignored) {
            try {
                Class<?> sm = Class.forName("android.os.ServiceManager");
                Object binder = sm.getMethod("getService", String.class).invoke(null, "clipboard");
                Class<?> stub = Class.forName("android.content.IClipboard$Stub");
                Object icb = stub.getMethod("asInterface", Class.forName("android.os.IBinder")).invoke(null, binder);
                for (java.lang.reflect.Method m : icb.getClass().getMethods()) {
                    if (!m.getName().equals("getPrimaryClip")) continue;
                    Class<?>[] pts = m.getParameterTypes();
                    Object clipObj = null;
                    if (pts.length == 4) {
                        clipObj = m.invoke(icb, "com.android.shell", null, 0, 0);
                    } else if (pts.length == 3) {
                        clipObj = m.invoke(icb, "com.android.shell", null, 0);
                    } else if (pts.length == 2) {
                        clipObj = m.invoke(icb, "com.android.shell", 0);
                    }
                    if (clipObj instanceof ClipData) {
                        ClipData clip = (ClipData) clipObj;
                        if (clip.getItemCount() > 0 && clip.getItemAt(0) != null) {
                            CharSequence cs = clip.getItemAt(0).coerceToText(context);
                            if (cs != null) text = cs.toString();
                        }
                        break;
                    }
                }
            } catch (Throwable ignored2) {}
        }
        return new JSONObject().put("text", text);
    }

    static JSONObject clipboardSet(String text) throws Exception {
        if (text == null) text = "";
        ClipData clip = ClipData.newPlainText("StepX", text);
        boolean written = false;
        try {
            ClipboardManager cm = (ClipboardManager) context.getSystemService(Context.CLIPBOARD_SERVICE);
            if (cm != null) {
                cm.setPrimaryClip(clip);
                written = true;
            }
        } catch (Throwable ignored) {}
        if (!written) {
            Class<?> sm = Class.forName("android.os.ServiceManager");
            Object binder = sm.getMethod("getService", String.class).invoke(null, "clipboard");
            Class<?> stub = Class.forName("android.content.IClipboard$Stub");
            Object icb = stub.getMethod("asInterface", Class.forName("android.os.IBinder")).invoke(null, binder);
            for (java.lang.reflect.Method m : icb.getClass().getMethods()) {
                if (!m.getName().equals("setPrimaryClip")) continue;
                Class<?>[] pts = m.getParameterTypes();
                if (pts.length == 5) {
                    m.invoke(icb, clip, "com.android.shell", null, 0, 0);
                    written = true;
                    break;
                } else if (pts.length == 4) {
                    m.invoke(icb, clip, "com.android.shell", null, 0);
                    written = true;
                    break;
                } else if (pts.length == 3) {
                    m.invoke(icb, clip, "com.android.shell", 0);
                    written = true;
                    break;
                }
            }
        }
        return new JSONObject().put("written", written).put("text", text);
    }

    static JSONObject steposUsage() throws Exception {
        JSONArray readErrors = new JSONArray();
        UsageStatsManager usm = (UsageStatsManager) context.getSystemService(Context.USAGE_STATS_SERVICE);
        PackageManager pm = context.getPackageManager();
        String tzId = "Asia/Shanghai";
        try {
            Class<?> sp = Class.forName("android.os.SystemProperties");
            String propTz = (String) sp.getMethod("get", String.class, String.class).invoke(null, "persist.sys.timezone", "");
            if (propTz != null && !propTz.trim().isEmpty()
                    && !"GMT".equalsIgnoreCase(propTz.trim())
                    && !"UTC".equalsIgnoreCase(propTz.trim())) {
                tzId = propTz.trim();
            }
        } catch (Throwable ignored) {}
        Calendar cal = Calendar.getInstance(TimeZone.getTimeZone(tzId));
        cal.set(Calendar.HOUR_OF_DAY, 0);
        cal.set(Calendar.MINUTE, 0);
        cal.set(Calendar.SECOND, 0);
        cal.set(Calendar.MILLISECOND, 0);
        long startOfDay = cal.getTimeInMillis();
        long now = System.currentTimeMillis();

        Set<String> ignoredPkgs = new HashSet<>(Arrays.asList(
            "android",
            "com.android.systemui",
            "com.stepos.launcher",
            "com.android.launcher3",
            "com.youdao.hardware.panda",
            "com.android.shell",
            "com.google.android.inputmethod.latin",
            "com.iflytek.inputmethod.stepos",
            "com.baidu.input",
            "org.fcitx.fcitx5.android",
            "com.android.permissioncontroller",
            "com.google.android.permissioncontroller"
        ));

        Map<String, Long> eventMsByPkg = new HashMap<>();
        int unlockCount = 0;
        if (usm == null) readErrors.put("用机统计服务不可用");
        if (usm != null) {
            try {
                UsageEvents events = usm.queryEvents(startOfDay, now);
                if (events == null) readErrors.put("用机事件暂不可读取");
                UsageEvents.Event ev = new UsageEvents.Event();
                String activePkg = null;
                String activeClass = null;
                long activeStart = 0L;
                while (events != null && events.hasNextEvent()) {
                    events.getNextEvent(ev);
                    int type = ev.getEventType();
                    long ts = Math.max(startOfDay, Math.min(now, ev.getTimeStamp()));
                    String evPkg = ev.getPackageName();
                    String evCls = ev.getClassName();
                    if (type == 18) { // KEYGUARD_HIDDEN
                        unlockCount++;
                    } else if (type == 1) { // ACTIVITY_RESUMED / MOVE_TO_FOREGROUND
                        if (activePkg != null && activeStart > 0L && ts > activeStart) {
                            long delta = ts - activeStart;
                            if (delta <= 12 * 3600_000L && !ignoredPkgs.contains(activePkg)) {
                                eventMsByPkg.put(activePkg, eventMsByPkg.getOrDefault(activePkg, 0L) + delta);
                            }
                        }
                        activePkg = evPkg;
                        activeClass = evCls;
                        activeStart = ts;
                    } else if (type == 2) { // ACTIVITY_PAUSED / MOVE_TO_BACKGROUND
                        if (activePkg != null && activePkg.equals(evPkg)
                                && (activeClass == null || evCls == null || activeClass.equals(evCls))) {
                            if (activeStart > 0L && ts > activeStart) {
                                long delta = ts - activeStart;
                                if (delta <= 12 * 3600_000L && !ignoredPkgs.contains(activePkg)) {
                                    eventMsByPkg.put(activePkg, eventMsByPkg.getOrDefault(activePkg, 0L) + delta);
                                }
                            }
                            activePkg = null;
                            activeClass = null;
                            activeStart = 0L;
                        }
                    } else if (type == 16 || type == 17 || type == 26) { // SCREEN_NON_INTERACTIVE, KEYGUARD_SHOWN, DEVICE_SHUTDOWN
                        if (activePkg != null && activeStart > 0L && ts > activeStart) {
                            long delta = ts - activeStart;
                            if (delta <= 12 * 3600_000L && !ignoredPkgs.contains(activePkg)) {
                                eventMsByPkg.put(activePkg, eventMsByPkg.getOrDefault(activePkg, 0L) + delta);
                            }
                        }
                        activePkg = null;
                        activeClass = null;
                        activeStart = 0L;
                    }
                }
                if (activePkg != null && activeStart > 0L && now > activeStart) {
                    long delta = now - activeStart;
                    if (delta <= 2 * 3600_000L && !ignoredPkgs.contains(activePkg)) {
                        eventMsByPkg.put(activePkg, eventMsByPkg.getOrDefault(activePkg, 0L) + delta);
                    }
                }
            } catch (Throwable failure) { readErrors.put("用机事件读取失败"); }
        }

        Map<String, Long> bucketMsByPkg = new HashMap<>();
        Map<String, Long> todayBucketSumByPkg = new HashMap<>();
        Set<String> seenTodayBuckets = new HashSet<>();
        if (usm != null) {
            try {
                List<UsageStats> daily = usm.queryUsageStats(UsageStatsManager.INTERVAL_DAILY, startOfDay, now);
                if (daily == null) readErrors.put("用机统计暂不可读取");
                if (daily != null) {
                    for (UsageStats st : daily) {
                        if (st == null) continue;
                        String pkg = st.getPackageName();
                        if (pkg == null || ignoredPkgs.contains(pkg)) continue;
                        if (st.getLastTimeUsed() < startOfDay || st.getFirstTimeStamp() < startOfDay - 12 * 3600_000L) continue;
                        long fg = st.getTotalTimeInForeground();
                        if (fg <= 0L) continue;
                        bucketMsByPkg.put(pkg, Math.max(bucketMsByPkg.getOrDefault(pkg, 0L), fg));
                        if (st.getFirstTimeStamp() >= startOfDay) {
                            String bucketKey = pkg + ":" + st.getFirstTimeStamp() + ":" + fg;
                            if (seenTodayBuckets.add(bucketKey)) {
                                todayBucketSumByPkg.put(pkg, todayBucketSumByPkg.getOrDefault(pkg, 0L) + fg);
                            }
                        }
                    }
                }
            } catch (Throwable failure) { readErrors.put("用机统计读取失败"); }
        }

        Set<String> allPkgs = new HashSet<>();
        allPkgs.addAll(eventMsByPkg.keySet());
        allPkgs.addAll(bucketMsByPkg.keySet());
        allPkgs.addAll(todayBucketSumByPkg.keySet());

        Map<String, Long> fgMsByPkg = new HashMap<>();
        for (String pkg : allPkgs) {
            if (pkg == null || ignoredPkgs.contains(pkg)) continue;
            long evMs = eventMsByPkg.getOrDefault(pkg, 0L);
            long bkMs = Math.max(bucketMsByPkg.getOrDefault(pkg, 0L), todayBucketSumByPkg.getOrDefault(pkg, 0L));
            long fg = Math.max(evMs, bkMs);
            if (fg >= 30_000L) {
                fgMsByPkg.put(pkg, fg);
            }
        }

        List<Map.Entry<String, Long>> sorted = new ArrayList<>(fgMsByPkg.entrySet());
        sorted.sort((a, b) -> Long.compare(b.getValue(), a.getValue()));

        long totalMs = 0L;
        for (Map.Entry<String, Long> e : sorted) {
            totalMs += e.getValue();
        }
        int totalMinutes = (int) Math.max(0L, (totalMs + 30_000L) / 60_000L);

        JSONArray topApps = new JSONArray();
        for (Map.Entry<String, Long> e : sorted) {
            if (topApps.length() >= 6) break;
            String pkg = e.getKey();
            int mins = (int) Math.max(1L, (e.getValue() + 30_000L) / 60_000L);
            String label = pkg;
            try {
                ApplicationInfo ai = pm.getApplicationInfo(pkg, 0);
                CharSequence cs = pm.getApplicationLabel(ai);
                if (cs != null && cs.length() > 0) {
                    label = cs.toString().trim();
                }
            } catch (Throwable ignored) {}
            if ("com.stepx.companion.app".equals(pkg)) label = "StepX伴侣";
            else if ("com.stepos.update".equals(pkg)) label = "系统更新";
            else if ("com.twitter.android".equals(pkg)) label = "X";
            else if ("mark.via".equals(pkg)) label = "Via 浏览器";
            else if ("com.google.android.googlequicksearchbox".equals(pkg)) label = "Google";
            else if ("com.android.mms".equals(pkg)) label = "短信";
            else if (label.equals(pkg) || label.contains(".")) {
                if (pkg.contains("ailauncher") || pkg.contains("habit")) label = "Amoo 助手";
                else if (pkg.contains("mm")) label = "微信";
                else if (pkg.contains("chrome")) label = "Chrome 浏览器";
                else if (pkg.contains("vending")) label = "Google Play 商店";
                else if (pkg.contains("gm")) label = "Gmail";
                else if (pkg.contains("bard")) label = "Gemini";
                else if (pkg.contains("settings")) label = "系统设置";
                else if (pkg.contains("gallery")) label = "相册";
                else if (pkg.contains("calendar")) label = "日历";
                else if (pkg.contains("alarmclock")) label = "时钟";
            }
            topApps.put(new JSONObject()
                .put("name", label)
                .put("package", pkg)
                .put("minutes", mins));
        }

        String clipText = "";
        try {
            clipText = clipboardGet().optString("text", "");
        } catch (Throwable failure) { readErrors.put("剪贴板读取失败"); }

        JSONArray recentCalls = queryRecentCalls(readErrors);
        JSONArray recentRecordings = queryRecentRecordings(readErrors);

        return new JSONObject()
            .put("screen_time_minutes", totalMinutes)
            .put("unlock_count", unlockCount)
            .put("clipboard", clipText)
            .put("top_apps", topApps)
            .put("calendar_events", queryCalendarEvents(readErrors))
            .put("alarms", queryAlarms(readErrors))
            .put("controls", querySystemControls(readErrors))
            .put("recent_calls", recentCalls)
            .put("recent_recordings", recentRecordings)
            .put("voice_recordings", recentRecordings)
            .put("read_errors", readErrors);
    }

    static JSONObject aiSuite() throws Exception {
        return steposUsage();
    }

    static JSONArray queryRecentCalls() { return queryRecentCalls(null); }

    static JSONArray queryRecentCalls(JSONArray readErrors) {
        JSONArray out = new JSONArray();
        try {
            if (context != null && context.checkSelfPermission(Manifest.permission.READ_CALL_LOG) != PackageManager.PERMISSION_GRANTED) {
                return out;
            }
            JSONArray rows = queryRows(
                android.provider.CallLog.Calls.CONTENT_URI.toString(),
                new String[]{"number", "name", "type", "date", "duration"},
                "date DESC, _id DESC",
                null,
                null,
                ContextReadPolicy.RECENT_CALLS_LIMIT
            ).optJSONArray("rows");
            if (rows == null) return out;
            for (int i = 0; i < rows.length() && out.length() < ContextReadPolicy.RECENT_CALLS_LIMIT; i++) {
                JSONObject r = rows.getJSONObject(i);
                String number = r.optString("number", "").trim();
                String name = r.optString("name", "").trim();
                if ("null".equalsIgnoreCase(name)) name = "";
                out.put(new JSONObject()
                    .put("number", number)
                    .put("name", name)
                    .put("type", r.optInt("type", 0))
                    .put("date", r.optLong("date", 0L))
                    .put("duration", r.optLong("duration", 0L)));
            }
        } catch (SecurityException se) {
            if (readErrors != null) readErrors.put("通话记录读取权限不足");
        } catch (Throwable failure) {
            Throwable cause = failure;
            while (cause.getCause() != null) cause = cause.getCause();
            if (cause instanceof SecurityException) {
                if (readErrors != null) readErrors.put("通话记录读取权限不足");
            }
        }
        return out;
    }

    static JSONArray queryRecentRecordings() { return queryRecentRecordings(null); }

    static JSONArray queryRecentRecordings(JSONArray readErrors) {
        JSONArray out = new JSONArray();
        Set<String> seenPaths = new HashSet<>();
        try {
            List<File> files = new ArrayList<>();
            for (String rel : new String[]{"/sdcard/Recordings", "/sdcard/SoundRecorder", "/sdcard/Sounds", "/sdcard/Music/Recordings"}) {
                File dir = new File(rel);
                if (!dir.isDirectory() || Files.isSymbolicLink(dir.toPath())) continue;
                collectRecordingFiles(dir, 0, files);
            }
            files.sort((a, b) -> Long.compare(b.lastModified(), a.lastModified()));
            for (File f : files) {
                if (out.length() >= ContextReadPolicy.RECENT_RECORDINGS_LIMIT) break;
                String path = f.getAbsolutePath();
                if (!seenPaths.add(path)) continue;
                out.put(new JSONObject()
                    .put("name", f.getName())
                    .put("path", path)
                    .put("size", f.length())
                    .put("bytes", f.length())
                    .put("date", f.lastModified())
                    .put("modified_ms", f.lastModified())
                    .put("duration", 0L));
            }
        } catch (SecurityException se) {
            if (readErrors != null) readErrors.put("录音文件目录读取权限不足");
        } catch (Throwable ignored) {}

        if (out.length() < ContextReadPolicy.RECENT_RECORDINGS_LIMIT) {
            try {
                boolean hasAudioPerm = context == null
                    || context.checkSelfPermission("android.permission.READ_MEDIA_AUDIO") == PackageManager.PERMISSION_GRANTED
                    || context.checkSelfPermission(Manifest.permission.READ_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED;
                if (hasAudioPerm) {
                    JSONArray rows = queryRows(
                        android.provider.MediaStore.Audio.Media.EXTERNAL_CONTENT_URI.toString(),
                        new String[]{"_display_name", "_data", "_size", "date_modified", "duration"},
                        "date_modified DESC",
                        "_data LIKE ? OR _data LIKE ? OR _data LIKE ?",
                        new String[]{"%/Recordings/%", "%/SoundRecorder/%", "%/Sounds/%"},
                        ContextReadPolicy.RECENT_RECORDINGS_LIMIT
                    ).optJSONArray("rows");
                    if (rows != null) {
                        for (int i = 0; i < rows.length() && out.length() < ContextReadPolicy.RECENT_RECORDINGS_LIMIT; i++) {
                            JSONObject r = rows.getJSONObject(i);
                            String path = r.optString("_data", "").trim();
                            if (path.isEmpty() || !seenPaths.add(path)) continue;
                            String name = r.optString("_display_name", "").trim();
                            if (name.isEmpty()) name = new File(path).getName();
                            long size = r.optLong("_size", 0L);
                            long modSec = r.optLong("date_modified", 0L);
                            long modMs = modSec > 10_000_000_000L ? modSec : modSec * 1000L;
                            long duration = r.optLong("duration", 0L);
                            out.put(new JSONObject()
                                .put("name", name)
                                .put("path", path)
                                .put("size", size)
                                .put("bytes", size)
                                .put("date", modMs)
                                .put("modified_ms", modMs)
                                .put("duration", duration));
                        }
                    }
                }
            } catch (SecurityException se) {
                if (readErrors != null) readErrors.put("媒体库录音读取权限不足");
            } catch (Throwable failure) {
                Throwable cause = failure;
                while (cause.getCause() != null) cause = cause.getCause();
                if (cause instanceof SecurityException && readErrors != null) {
                    readErrors.put("媒体库录音读取权限不足");
                }
            }
        }
        return out;
    }

    private static void collectRecordingFiles(File dir, int depth, List<File> out) {
        if (depth > 2 || out.size() >= 60) return;
        File[] children = dir.listFiles();
        if (children == null) return;
        for (File child : children) {
            if (out.size() >= 60) break;
            try {
                if (Files.isSymbolicLink(child.toPath()) || child.getName().startsWith(".")) continue;
                if (child.isDirectory()) {
                    collectRecordingFiles(child, depth + 1, out);
                } else if (child.isFile() && child.length() > 0 && ContextReadPolicy.isAudioRecordingName(child.getName())) {
                    out.add(child);
                }
            } catch (Throwable ignored) {}
        }
    }

    static Uri insertRow(String uri, ContentValues values) throws Exception {
        Uri target = Uri.parse(uri);
        Class<?> am = Class.forName("android.app.ActivityManager");
        Class<?> iam = Class.forName("android.app.IActivityManager");
        Object service = am.getMethod("getService").invoke(null);
        int user = (Integer) am.getMethod("getCurrentUser").invoke(null);
        android.os.IBinder token = new android.os.Binder();
        Object holder = iam.getMethod("getContentProviderExternal", String.class, int.class,
            android.os.IBinder.class, String.class).invoke(service, target.getAuthority(), user, token, "*cmd*");
        if (holder == null) throw new IOException("手机数据提供程序不可用");
        try {
            Object provider = holder.getClass().getField("provider").get(holder);
            Class<?> ipc = Class.forName("android.content.IContentProvider");
            android.content.AttributionSource attribution = new android.content.AttributionSource.Builder(android.os.Process.myUid())
                .setPackageName("com.android.shell").build();
            Bundle extras = new Bundle();
            return (Uri) ipc.getMethod("insert", android.content.AttributionSource.class,
                Uri.class, ContentValues.class, Bundle.class).invoke(provider, attribution, target, values, extras);
        } finally {
            iam.getMethod("removeContentProviderExternalAsUser", String.class, android.os.IBinder.class, int.class)
                .invoke(service, target.getAuthority(), token, user);
        }
    }

    static int updateRow(String uri, ContentValues values, String where) throws Exception {
        Uri target = Uri.parse(uri);
        Class<?> am = Class.forName("android.app.ActivityManager");
        Class<?> iam = Class.forName("android.app.IActivityManager");
        Object service = am.getMethod("getService").invoke(null);
        int user = (Integer) am.getMethod("getCurrentUser").invoke(null);
        android.os.IBinder token = new android.os.Binder();
        Object holder = iam.getMethod("getContentProviderExternal", String.class, int.class,
            android.os.IBinder.class, String.class).invoke(service, target.getAuthority(), user, token, "*cmd*");
        if (holder == null) throw new IOException("手机数据提供程序不可用");
        try {
            Object provider = holder.getClass().getField("provider").get(holder);
            Class<?> ipc = Class.forName("android.content.IContentProvider");
            android.content.AttributionSource attribution = new android.content.AttributionSource.Builder(android.os.Process.myUid())
                .setPackageName("com.android.shell").build();
            Bundle extras = new Bundle();
            if (where != null && !where.isEmpty()) {
                extras.putString(ContentResolver.QUERY_ARG_SQL_SELECTION, where);
            }
            return (Integer) ipc.getMethod("update", android.content.AttributionSource.class,
                Uri.class, ContentValues.class, Bundle.class).invoke(provider, attribution, target, values, extras);
        } finally {
            iam.getMethod("removeContentProviderExternalAsUser", String.class, android.os.IBinder.class, int.class)
                .invoke(service, target.getAuthority(), token, user);
        }
    }

    static int deleteRow(String uri, String where) throws Exception {
        Uri target = Uri.parse(uri);
        Class<?> am = Class.forName("android.app.ActivityManager");
        Class<?> iam = Class.forName("android.app.IActivityManager");
        Object service = am.getMethod("getService").invoke(null);
        int user = (Integer) am.getMethod("getCurrentUser").invoke(null);
        android.os.IBinder token = new android.os.Binder();
        Object holder = iam.getMethod("getContentProviderExternal", String.class, int.class,
            android.os.IBinder.class, String.class).invoke(service, target.getAuthority(), user, token, "*cmd*");
        if (holder == null) throw new IOException("手机数据提供程序不可用");
        try {
            Object provider = holder.getClass().getField("provider").get(holder);
            Class<?> ipc = Class.forName("android.content.IContentProvider");
            android.content.AttributionSource attribution = new android.content.AttributionSource.Builder(android.os.Process.myUid())
                .setPackageName("com.android.shell").build();
            Bundle extras = new Bundle();
            if (where != null && !where.isEmpty()) {
                extras.putString(ContentResolver.QUERY_ARG_SQL_SELECTION, where);
            }
            return (Integer) ipc.getMethod("delete", android.content.AttributionSource.class,
                Uri.class, Bundle.class).invoke(provider, attribution, target, extras);
        } finally {
            iam.getMethod("removeContentProviderExternalAsUser", String.class, android.os.IBinder.class, int.class)
                .invoke(service, target.getAuthority(), token, user);
        }
    }

    static JSONArray queryCalendarEvents() { return queryCalendarEvents(null); }

    static JSONArray queryCalendarEvents(JSONArray readErrors) {
        JSONArray out = new JSONArray();
        try {
            // Instances expands recurring events in the requested range. Querying
            // Events then taking the first 20 rows dropped today's occurrences.
            JSONObject page = readCalendarPage(new JSONObject().put("limit", 1000));
            JSONArray rows = page.optJSONArray("rows");
            if (rows == null) {
                if (readErrors != null) readErrors.put("手机日历未返回有效数据");
                return out;
            }
            if (page.optBoolean("has_more", false) && readErrors != null) {
                readErrors.put("日历范围内超过 1000 条，需分页读取；当前日程不完整");
            }
            for (int i = 0; i < rows.length(); i++) {
                JSONObject r = rows.getJSONObject(i);
                long id = r.optLong("event_id", 0L);
                if (id <= 0L) continue;
                long startMs = r.optLong("begin", 0L);
                long endMs = r.optLong("end", startMs + 3600_000L);
                String title = r.optString("title", "").trim();
                if (title.isEmpty()) title = "未命名日程";
                out.put(new JSONObject()
                    .put("id", id)
                    .put("instance_id", r.optLong("_id", 0L))
                    .put("title", title)
                    .put("description", r.optString("description", "").trim())
                    .put("location", r.optString("eventLocation", "").trim())
                    .put("start_ms", startMs)
                    .put("end_ms", endMs)
                    .put("recurring", !nullableCalendarText(r.optString("rrule", "")).isEmpty()
                        || !nullableCalendarText(r.optString("rdate", "")).isEmpty())
                    .put("all_day", "1".equals(r.optString("allDay", "0"))));
            }
        } catch (Throwable failure) {
            if (readErrors != null) readErrors.put("手机日历读取失败");
        }
        return out;
    }

    private static String nullableCalendarText(String value) {
        String text = value == null ? "" : value.trim();
        return "null".equalsIgnoreCase(text) ? "" : text;
    }

    static JSONArray queryAlarms() { return queryAlarms(null); }

    static JSONArray queryAlarms(JSONArray readErrors) {
        JSONArray out = new JSONArray();
        try {
            JSONArray rows = queryRows(
                "content://com.android.alarmclock/alarms",
                new String[]{"_id", "hour", "minutes", "daysofweek", "enabled", "vibrate", "label"},
                "hour ASC, minutes ASC"
            ).optJSONArray("rows");
            if (rows == null) {
                if (readErrors != null) readErrors.put("手机闹钟未返回有效数据");
                return out;
            }
            for (int i = 0; i < rows.length(); i++) {
                JSONObject r = rows.getJSONObject(i);
                long id = r.optLong("_id", 0L);
                if (id <= 0L) continue;
                int hour = r.optInt("hour", 8);
                int minutes = r.optInt("minutes", 0);
                int daysOfWeek = r.optInt("daysofweek", 0);
                boolean enabled = "1".equals(r.optString("enabled", "0"));
                boolean vibrate = "1".equals(r.optString("vibrate", "1"));
                String label = r.optString("label", "").trim();
                if ("null".equalsIgnoreCase(label)) label = "";
                out.put(new JSONObject()
                    .put("id", id)
                    .put("hour", hour)
                    .put("minutes", minutes)
                    .put("days_of_week", daysOfWeek)
                    .put("enabled", enabled)
                    .put("vibrate", vibrate)
                    .put("label", label));
                if (out.length() >= 20) break;
            }
        } catch (Throwable failure) {
            if (readErrors != null) readErrors.put("手机闹钟读取失败");
        }
        return out;
    }

    static int settingInt(String table, String key, int def) {
        try {
            ContentResolver cr = context.getContentResolver();
            if ("global".equals(table)) {
                return Settings.Global.getInt(cr, key);
            } else if ("system".equals(table)) {
                return Settings.System.getInt(cr, key);
            }
        } catch (Throwable ignored) {}
        try {
            JSONArray rows = queryRows(
                "content://settings/" + table,
                new String[]{"name", "value"},
                null,
                "name=?",
                new String[]{key}
            ).optJSONArray("rows");
            if (rows == null || rows.length() == 0) {
                rows = queryRows(
                    "content://settings/" + table + "/" + key,
                    new String[]{"value"},
                    null
                ).optJSONArray("rows");
            }
            if (rows != null && rows.length() > 0) {
                return rows.getJSONObject(0).optInt("value", def);
            }
        } catch (Throwable ignored) {}
        return def;
    }

    static int checkedSettingInt(String table, String key, int fallback, JSONArray readErrors) {
        int value = settingInt(table, key, Integer.MIN_VALUE);
        if (value == Integer.MIN_VALUE) {
            if (readErrors != null) readErrors.put("系统设置读取失败：" + key);
            return fallback;
        }
        return value;
    }

    static JSONObject querySystemControls() throws Exception { return querySystemControls(null); }

    static JSONObject querySystemControls(JSONArray readErrors) throws Exception {
        boolean dndOn = checkedSettingInt("global", "zen_mode", 0, readErrors) != 0;
        boolean wifiOn = checkedSettingInt("global", "wifi_on", 1, readErrors) != 0;
        boolean bluetoothOn = checkedSettingInt("global", "bluetooth_on", 0, readErrors) == 1;
        int raw = checkedSettingInt("system", "screen_brightness", 1600, readErrors);
        int max = raw > 4095 ? 32767 : (raw > 255 ? 4095 : 255);
        int brightnessPercent = Math.max(5, Math.min(100, Math.round(raw * 100.0f / max)));
        return new JSONObject()
            .put("dnd_on", dndOn)
            .put("wifi_on", wifiOn)
            .put("bluetooth_on", bluetoothOn)
            .put("brightness_percent", brightnessPercent);
    }

    static JSONObject calendarCreate(JSONObject req) throws Exception {
        String title = req.optString("title", "").trim();
        if (title.isEmpty()) title = "StepX 跨端协作日程";
        String description = req.optString("description", "由 StepX 桌面端创建").trim();
        String location = req.optString("location", "").trim();
        long startMs = req.optLong("start_ms", System.currentTimeMillis() + 3600_000L);
        long endMs = req.optLong("end_ms", startMs + 3600_000L);
        ContentValues values = new ContentValues();
        values.put("calendar_id", 1);
        values.put("title", title);
        values.put("description", description);
        values.put("eventLocation", location);
        values.put("dtstart", startMs);
        values.put("dtend", endMs);
        values.put("eventTimezone", TimeZone.getDefault().getID());
        values.put("allDay", 0);
        Uri created = insertRow("content://com.android.calendar/events", values);
        long id = 0L;
        if (created != null && created.getLastPathSegment() != null) {
            try { id = Long.parseLong(created.getLastPathSegment()); } catch (NumberFormatException ignored) {}
        }
        return new JSONObject()
            .put("created_id", id)
            .put("calendar_events", queryCalendarEvents());
    }

    static JSONObject calendarDelete(JSONObject req) throws Exception {
        long id = req.getLong("id");
        if (id <= 0L) throw new IllegalArgumentException("无效的日程 ID");
        int deleted = deleteRow("content://com.android.calendar/events/" + id, null);
        if (deleted <= 0) {
            deleted = deleteRow("content://com.android.calendar/events", "_id=" + id);
        }
        return new JSONObject()
            .put("deleted", deleted)
            .put("calendar_events", queryCalendarEvents());
    }

    static JSONObject alarmCreate(JSONObject req) throws Exception {
        int hour = Math.max(0, Math.min(23, req.optInt("hour", 8)));
        int minutes = Math.max(0, Math.min(59, req.optInt("minutes", 0)));
        int daysOfWeek = req.optInt("days_of_week", 0);
        String label = req.optString("label", "").trim();
        Object rawSkipHolidays = (req != null && req.has("skip_holidays") && !req.isNull("skip_holidays")) ? req.opt("skip_holidays") : null;
        Object rawStatutoryHolidays = (req != null && req.has("statutory_holidays") && !req.isNull("statutory_holidays")) ? req.opt("statutory_holidays") : null;
        Object rawDailyEnabled = (req != null && req.has("daily_enabled") && !req.isNull("daily_enabled")) ? req.opt("daily_enabled") : null;
        int skipHolidays = ContextReadPolicy.resolveSkipHolidays(rawSkipHolidays, rawStatutoryHolidays);
        int statutoryHolidays = ContextReadPolicy.resolveStatutoryHolidays(rawSkipHolidays, rawStatutoryHolidays);
        int dailyEnabled = ContextReadPolicy.resolveDailyEnabled(rawDailyEnabled);
        ContentValues values = new ContentValues();
        values.put("hour", hour);
        values.put("minutes", minutes);
        values.put("daysofweek", daysOfWeek);
        values.put("enabled", 1);
        values.put("vibrate", 1);
        values.put("label", label);
        values.put("delete_after_use", 0);
        values.put("volume", 10);
        values.put("snooze", 1);
        values.put("festival", 0);
        values.put("snooze_times", 8);
        values.put("daily_enabled", dailyEnabled);
        values.put("daily_play_type", 0);
        values.put("daily_skip_time", 0);
        values.put("upcoming", 0);
        values.put("shifts", 0);
        values.put("skip_holidays", skipHolidays);
        values.put("statutory_holidays", statutoryHolidays);
        values.put("ring_duration", 3);
        values.put("creation_method", 0);
        Uri created = insertRow("content://com.android.alarmclock/alarms", values);
        long id = 0L;
        if (created != null && created.getLastPathSegment() != null) {
            try { id = Long.parseLong(created.getLastPathSegment()); } catch (NumberFormatException ignored) {}
        }
        return new JSONObject()
            .put("created_id", id)
            .put("skip_holidays", skipHolidays)
            .put("statutory_holidays", statutoryHolidays)
            .put("daily_enabled", dailyEnabled)
            .put("alarms", queryAlarms());
    }

    static JSONObject alarmToggle(JSONObject req) throws Exception {
        long id = req.getLong("id");
        if (id <= 0L) throw new IllegalArgumentException("无效的闹钟 ID");
        boolean enabled = req.getBoolean("enabled");
        ContentValues values = new ContentValues();
        values.put("enabled", enabled ? 1 : 0);
        int updated = updateRow("content://com.android.alarmclock/alarms/" + id, values, null);
        return new JSONObject()
            .put("updated", updated)
            .put("alarms", queryAlarms());
    }

    static JSONObject alarmDelete(JSONObject req) throws Exception {
        long id = req.getLong("id");
        if (id <= 0L) throw new IllegalArgumentException("无效的闹钟 ID");
        int deleted = deleteRow("content://com.android.alarmclock/alarms", "_id=" + id);
        return new JSONObject()
            .put("deleted", deleted)
            .put("alarms", queryAlarms());
    }
}
