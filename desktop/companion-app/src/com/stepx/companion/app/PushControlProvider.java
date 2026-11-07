package com.stepx.companion.app;

import android.app.KeyguardManager;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.database.Cursor;
import android.graphics.drawable.Icon;
import android.net.Uri;
import android.os.BatteryManager;
import android.os.Binder;
import android.os.Bundle;
import android.os.PowerManager;
import android.os.Process;
import android.os.SystemClock;
import android.provider.Settings;
import android.service.notification.StatusBarNotification;
import android.util.Base64;
import org.json.JSONObject;
import java.nio.charset.StandardCharsets;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.Set;

/**
 * Shell-only authenticated ContentProvider (content://com.stepx.companion.app.push)
 * for IM Chat Hub and Phone Push Notification Center RPCs.
 */
public final class PushControlProvider extends ContentProvider {
    public static final String LIVE_CAPSULE_CHANNEL_ID = "stepx_live_capsule";
    public static final int LIVE_CAPSULE_NOTIFICATION_ID = 3200;
    private static final String[] KNOWN_CAPSULE_TAGS = new String[]{
        "stepx_live_capsule",
        "stepx_pomodoro",
        "stepx_pickup",
        "stepx_status",
        "stepx_schedule",
        "stepx_amoo",
        "stepx_transfer"
    };
    private static final Set<String> ACTIVE_CAPSULE_TAGS =
        Collections.synchronizedSet(new LinkedHashSet<String>());
    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public Bundle call(String method, String arg, Bundle extras) {
        int uid = Binder.getCallingUid();
        if (uid != 2000 && uid != Process.myUid()) {
            throw new SecurityException("不允许的调用方");
        }
        long identity = Binder.clearCallingIdentity();
        Bundle out = new Bundle();
        JSONObject result;
        try {
            if (!"request".equals(method) || arg == null || arg.length() > 65536) {
                throw new IllegalArgumentException("请求无效");
            }
            JSONObject q = new JSONObject(new String(Base64.decode(arg, Base64.DEFAULT), StandardCharsets.UTF_8));
            result = handleRequest(getContext(), q);
        } catch (Exception error) {
            Throwable cause = error.getCause() == null ? error : error.getCause();
            result = new JSONObject();
            try {
                result.put("ok", false).put("error", String.valueOf(cause.getMessage()));
            } catch (Exception ignored) {
            }
        } finally {
            Binder.restoreCallingIdentity(identity);
        }
        out.putString(
            "result",
            Base64.encodeToString(result.toString().getBytes(StandardCharsets.UTF_8), Base64.NO_WRAP)
        );
        return out;
    }

    static JSONObject handleRequest(Context context, JSONObject q) throws Exception {
        String op = q.optString("op", "").trim();
        // A call UUID/state read must not queue behind storage, GUI, or a listener rebind.
        if ("call_state".equals(op)) return PhoneCallMonitor.snapshot();
        PushStore store = PushStore.get(context);
        boolean listenerConnected = PushNotificationService.ensureRebound(context);
        boolean accessibilityConnected = GuiAccessibilityService.instance != null || ChatUiBridge.isApexBridgeAvailable();
        boolean humanTouching = GuiAccessibilityService.isHumanTouchingRecently(10_000L);
        boolean wifiAdbEnabled = isWifiAdbEnabled(context);
        boolean screenInteractive = isScreenInteractive(context);
        boolean keyguardLocked = isKeyguardLocked(context);

        if ("status".equals(op) || "ping".equals(op)) {
            JSONObject snap = store.statusSummary();
            return new JSONObject()
                .put("ok", true)
                .put("pong", true)
                .put("listenerConnected", listenerConnected)
                .put("accessibilityConnected", accessibilityConnected)
                .put("wifiAdbEnabled", wifiAdbEnabled)
                .put("screenInteractive", screenInteractive)
                .put("keyguardLocked", keyguardLocked)
                .put("humanTouchRecent", humanTouching)
                .put("socketName", PushNotificationService.SOCKET_NAME)
                .put("revision", snap.optLong("revision", store.getRevision()))
                .put("threadCount", snap.optInt("threadCount", store.getThreadCount()))
                .put("unreadImCount", snap.optInt("unreadImCount", 0))
                .put("activePushCount", snap.optInt("activePushCount", 0))
                .put("totalPushCount", snap.optInt("totalPushCount", 0))
                .put("smartBackScreen", SmartBackScreenController.get(context).getStatusJson(context));
        }

        if ("snapshot".equals(op)) {
            if (listenerConnected && PushNotificationService.instance != null && q.optBoolean("rescan", false)) {
                PushNotificationService.instance.scanActiveNotifications();
            }
            long sinceRevision = q.optLong("sinceRevision", q.optLong("since_revision", 0L));
            JSONObject snap = store.snapshot(sinceRevision);
            snap.put("listenerConnected", listenerConnected);
            snap.put("accessibilityConnected", accessibilityConnected);
            snap.put("wifiAdbEnabled", wifiAdbEnabled);
            snap.put("screenInteractive", screenInteractive);
            snap.put("keyguardLocked", keyguardLocked);
            snap.put("humanTouchRecent", humanTouching);
            snap.put("live_call", PhoneCallMonitor.snapshot());
            snap.put("batteryState", BatteryStateCache.snapshot(context));
            snap.put("smartBackScreen", SmartBackScreenController.get(context).getStatusJson(context));
            return snap;
        }

        if ("configure_smart_backscreen".equals(op)) {
            return SmartBackScreenController.get(context).configure(context, q);
        }

        if ("smart_backscreen_status".equals(op)) {
            return SmartBackScreenController.get(context).getStatusJson(context);
        }

        if ("phone_digest".equals(op)) {
            return buildPhoneDigest(
                context,
                store,
                listenerConnected,
                accessibilityConnected,
                wifiAdbEnabled,
                screenInteractive,
                keyguardLocked,
                humanTouching
            );
        }

        if ("thread_detail".equals(op)) {
            String threadId = q.optString("threadId", q.optString("thread_id", "")).trim();
            JSONObject detail = store.threadDetail(threadId);
            detail.put("listenerConnected", listenerConnected);
            detail.put("accessibilityConnected", accessibilityConnected);
            return detail;
        }

        if ("reply_im".equals(op)) {
            String threadId = q.optString("threadId", q.optString("thread_id", "")).trim();
            String pkg = q.optString("packageName", q.optString("package_name", "")).trim();
            String title = q.optString("title", "").trim();
            String text = q.optString("text", q.optString("body", "")).trim();
            String mode = q.optString("mode", "auto").trim();
            String clientMsgId = q.optString("clientMsgId", q.optString("client_msg_id", "")).trim();
            boolean force = q.optBoolean("force", "ui_force".equalsIgnoreCase(mode));

            if (text.isEmpty()) {
                throw new IllegalArgumentException("回复内容不能为空");
            }
            if (threadId.isEmpty() && !pkg.isEmpty() && !title.isEmpty()) {
                threadId = ImAppCatalog.buildThreadId(pkg, title);
            }
            if (pkg.isEmpty() && threadId.contains("::")) {
                pkg = threadId.substring(0, threadId.indexOf("::"));
            }
            if (title.isEmpty() && threadId.contains("::")) {
                title = threadId.substring(threadId.indexOf("::") + 2);
            }

            if (!store.hasReplyHandle(threadId) && PushNotificationService.instance != null) {
                try {
                    PushNotificationService.instance.scanActiveNotifications();
                } catch (Throwable ignored) {}
            }

            if ("ui_force".equalsIgnoreCase(mode)) {
                JSONObject res = ChatUiBridge.sendChatViaUi(context, pkg, title, text, true, clientMsgId);
                if (PushNotificationService.instance != null) {
                    PushNotificationService.instance.broadcastDelta(res);
                }
                SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
                return res;
            }

            if ("remote_input".equalsIgnoreCase(mode) || "remote_only".equalsIgnoreCase(mode)) {
                JSONObject res = store.executeRemoteReply(context, threadId, text, clientMsgId);
                if (PushNotificationService.instance != null) {
                    PushNotificationService.instance.broadcastDelta(res);
                }
                SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
                return res;
            }

            // Always try cached RemoteInput PendingIntent first (<200ms silent background reply even when locked),
            // then fall back to ChatUiBridge if handle is missing or expired.
            try {
                JSONObject res = store.executeRemoteReply(context, threadId, text, clientMsgId);
                if (PushNotificationService.instance != null) {
                    PushNotificationService.instance.broadcastDelta(res);
                }
                SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
                return res;
            } catch (Exception remoteErr) {
                if ("ui".equalsIgnoreCase(mode)
                        || "ui_bridge".equalsIgnoreCase(mode)
                        || GuiAccessibilityService.instance != null
                        || ChatUiBridge.isApexBridgeAvailable()) {
                    JSONObject res = ChatUiBridge.sendChatViaUi(context, pkg, title, text, force, clientMsgId);
                    if (PushNotificationService.instance != null) {
                        PushNotificationService.instance.broadcastDelta(res);
                    }
                    SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
                    return res;
                }
                throw remoteErr;
            }
        }

        if ("sync_window".equals(op) || "sync_im_list".equals(op)) {
            String pkg = q.optString("packageName", q.optString("package_name", "")).trim();
            String title = "sync_im_list".equals(op)
                ? ""
                : q.optString("title", q.optString("threadId", q.optString("thread_id", ""))).trim();
            if (pkg.isEmpty() && title.contains("::")) {
                pkg = title.substring(0, title.indexOf("::"));
            }
            if (title.contains("::")) {
                title = title.substring(title.indexOf("::") + 2);
            }
            JSONObject res = ChatUiBridge.captureActiveChatWindow(context, pkg, title);
            if (PushNotificationService.instance != null) {
                PushNotificationService.instance.broadcastDelta(res);
            }
            return res;
        }

        if ("delete_thread".equals(op)) {
            String threadId = q.optString("threadId", q.optString("thread_id", "")).trim();
            String pkg = q.optString("packageName", q.optString("package_name", "")).trim();
            String title = q.optString("title", "").trim();
            JSONObject res = store.deleteThread(context, threadId, pkg, title);
            if (PushNotificationService.instance != null) {
                PushNotificationService.instance.broadcastDelta(res);
            }
            SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
            return res;
        }

        if ("mark_thread_read".equals(op)) {
            String threadId = q.optString("threadId", q.optString("thread_id", "")).trim();
            String pkg = q.optString("packageName", q.optString("package_name", "")).trim();
            String title = q.optString("title", "").trim();
            boolean read = q.optBoolean("read", true);
            JSONObject res = store.markThreadRead(context, threadId, pkg, title, read);
            if (PushNotificationService.instance != null) {
                PushNotificationService.instance.broadcastDelta(res);
            }
            SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
            return res;
        }

        if ("clear_thread_messages".equals(op)) {
            String threadId = q.optString("threadId", q.optString("thread_id", "")).trim();
            String pkg = q.optString("packageName", q.optString("package_name", "")).trim();
            String title = q.optString("title", "").trim();
            JSONObject res = store.clearThreadMessages(threadId, pkg, title);
            if (PushNotificationService.instance != null) {
                PushNotificationService.instance.broadcastDelta(res);
            }
            SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
            return res;
        }

        if ("delete_message".equals(op)) {
            String threadId = q.optString("threadId", q.optString("thread_id", "")).trim();
            String msgId = q.optString("messageId", q.optString("message_id", q.optString("msgId", ""))).trim();
            String text = q.optString("text", q.optString("body", "")).trim();
            JSONObject res = store.deleteMessage(threadId, msgId, text);
            if (PushNotificationService.instance != null) {
                PushNotificationService.instance.broadcastDelta(res);
            }
            SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
            return res;
        }

        if ("dismiss_push".equals(op)) {
            String key = q.optString("key", q.optString("id", "")).trim();
            PushNotificationService.dismissNotification(context, key);
            return new JSONObject()
                .put("ok", true)
                .put("key", key)
                .put("revision", store.getRevision());
        }

        if ("dismiss_all_push".equals(op)) {
            String category = q.optString("category", "").trim();
            int count = PushNotificationService.dismissAllNotifications(context, category);
            return new JSONObject()
                .put("ok", true)
                .put("cleared", count)
                .put("count", count)
                .put("dismissed", count)
                .put("dismissedCount", count)
                .put("revision", store.getRevision());
        }

        if ("open_push".equals(op)) {
            String key = q.optString("key", q.optString("id", "")).trim();
            String pkg = q.optString("packageName", q.optString("package_name", "")).trim();
            JSONObject res = store.openPush(context, key, pkg);
            SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
            return res;
        }

        if ("post_live_capsule".equals(op)) {
            return postLiveCapsule(context, q);
        }

        if ("dismiss_live_capsule".equals(op)) {
            String tag = q.optString("tag", "stepx_live_capsule").trim();
            return PushNotificationService.dismissLiveCapsule(context, tag);
        }

        if ("clear_live_capsules".equals(op)) {
            return PushNotificationService.clearLiveCapsules(context);
        }

        throw new IllegalArgumentException("未知推送操作: " + op);
    }

    public static JSONObject postLiveCapsule(Context context, JSONObject q) throws Exception {
        if (context == null) {
            throw new IllegalStateException("缺少应用上下文");
        }
        String rawTag = q.optString("tag", "stepx_live_capsule").trim();
        String tag = rawTag.isEmpty() ? "stepx_live_capsule" : rawTag;
        String pillText = q.optString("pillText", q.optString("pill_text", "")).trim();
        String title = q.optString("title", "").trim();
        String subtitle = q.optString("subtitle", q.optString("text", "")).trim();
        String badgeText = q.optString("badgeText", q.optString("badge_text", "")).trim();
        int liveType = q.optInt("liveType", q.optInt("live_type", 1));
        long countdownSecs = Math.max(0L, q.optLong("countdownSecs", q.optLong("countdown_secs", 0L)));
        boolean ongoing = q.optBoolean("ongoing", true);

        if (title.isEmpty()) {
            title = !pillText.isEmpty() ? pillText : "StepX 实况胶囊";
        }
        if (pillText.isEmpty()) {
            pillText = title;
        }
        int effectiveLiveType = countdownSecs > 0L ? 11 : (liveType > 0 ? liveType : 1);

        NotificationManager nm = context.getSystemService(NotificationManager.class);
        if (nm == null) {
            throw new IllegalStateException("系统通知服务不可用");
        }

        NotificationChannel channel = new NotificationChannel(
            LIVE_CAPSULE_CHANNEL_ID,
            "StepX 实况胶囊",
            NotificationManager.IMPORTANCE_HIGH
        );
        channel.setSound(null, null);
        channel.enableVibration(false);
        channel.setShowBadge(false);
        nm.createNotificationChannel(channel);

        int iconRes = (countdownSecs > 0L || effectiveLiveType == 11)
            ? android.R.drawable.ic_lock_idle_alarm
            : (effectiveLiveType == 6
                ? android.R.drawable.ic_dialog_info
                : android.R.drawable.stat_notify_sync);
        Icon icon = Icon.createWithResource(context, iconRes);

        Bundle capsule = new Bundle();
        capsule.putInt("notification.live.capsuleStatus", 1);
        capsule.putInt("notification.live.capsuleType", 1);
        capsule.putParcelable("notification.live.capsuleIcon", icon);
        capsule.putString("notification.live.capsuleContent", pillText);
        capsule.putInt("notification.live.capsuleContentColor", 0xFFFFFFFF);
        capsule.putInt("notification.live.capsuleBgColor", 0);

        Bundle extras = new Bundle();
        extras.putBoolean("is_live", true);
        extras.putInt("notification.live.operation", 1);
        extras.putInt("notification.live.type", effectiveLiveType);
        extras.putString(
            "notification.live.event",
            (countdownSecs > 0L || effectiveLiveType == 11) ? "TIMER" : "BOT"
        );
        extras.putBoolean("notification.live.pinned", true);
        extras.putBoolean("notification.live.feature.progressHide", true);
        extras.putString("notification.live.titleOverlay", title);
        extras.putString("notification.live.contentOverlay", subtitle);
        extras.putParcelable("notification.live.iconOverlay", icon);
        extras.putParcelable("notification.live.capsuleIcon", icon);
        if (!badgeText.isEmpty()) {
            extras.putString("notification.live.task.state.desc", badgeText);
            extras.putInt("notification.live.task.state.desc.color", 0xFF10B981);
        }
        extras.putBoolean("notification.live.can.delete", true);
        extras.putBundle("notification.live.capsule", capsule);

        Intent launchIntent = new Intent(context, MainActivity.class)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        PendingIntent contentIntent = PendingIntent.getActivity(
            context,
            LIVE_CAPSULE_NOTIFICATION_ID,
            launchIntent,
            PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE
        );

        Notification.Builder builder = new Notification.Builder(context, LIVE_CAPSULE_CHANNEL_ID)
            .setSmallIcon(icon)
            .setContentTitle(title)
            .setContentText(subtitle)
            .setContentIntent(contentIntent)
            .setOngoing(ongoing)
            .setOnlyAlertOnce(true)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setCategory((countdownSecs > 0L || effectiveLiveType == 11)
                ? Notification.CATEGORY_ALARM
                : Notification.CATEGORY_STATUS);

        if (!badgeText.isEmpty()) {
            builder.setSubText(badgeText);
        }

        if (countdownSecs > 0L) {
            long remMs = countdownSecs * 1000L;
            extras.putLong("notification.live.feature.chronometerBase", SystemClock.elapsedRealtime() + remMs);
            extras.putBoolean("notification.live.feature.chronometerPause", false);
            extras.putInt("notification.live.feature.chronometerPosition", 1);
            builder.setUsesChronometer(true)
                .setChronometerCountDown(true)
                .setWhen(System.currentTimeMillis() + remMs)
                .setShowWhen(true);
        }

        builder.addExtras(extras);
        ACTIVE_CAPSULE_TAGS.add(tag);
        nm.notify(tag, LIVE_CAPSULE_NOTIFICATION_ID, builder.build());

        return new JSONObject()
            .put("ok", true)
            .put("tag", tag)
            .put("id", LIVE_CAPSULE_NOTIFICATION_ID)
            .put("liveType", effectiveLiveType)
            .put("pillText", pillText)
            .put("countdownSecs", countdownSecs);
    }

    public static JSONObject dismissLiveCapsule(Context context, String rawTag) throws Exception {
        if (context == null) {
            throw new IllegalStateException("缺少应用上下文");
        }
        String tag = (rawTag == null || rawTag.trim().isEmpty()) ? "stepx_live_capsule" : rawTag.trim();
        NotificationManager nm = context.getSystemService(NotificationManager.class);
        if (nm != null) {
            nm.cancel(tag, LIVE_CAPSULE_NOTIFICATION_ID);
            nm.cancel(LIVE_CAPSULE_NOTIFICATION_ID);
        }
        ACTIVE_CAPSULE_TAGS.remove(tag);
        return new JSONObject()
            .put("ok", true)
            .put("tag", tag)
            .put("dismissed", true);
    }

    public static JSONObject clearLiveCapsules(Context context) throws Exception {
        if (context == null) {
            throw new IllegalStateException("缺少应用上下文");
        }
        NotificationManager nm = context.getSystemService(NotificationManager.class);
        Set<String> tagsToCancel = new LinkedHashSet<String>();
        for (String known : KNOWN_CAPSULE_TAGS) {
            tagsToCancel.add(known);
        }
        synchronized (ACTIVE_CAPSULE_TAGS) {
            tagsToCancel.addAll(ACTIVE_CAPSULE_TAGS);
            ACTIVE_CAPSULE_TAGS.clear();
        }
        int cleared = 0;
        if (nm != null) {
            try {
                StatusBarNotification[] active = nm.getActiveNotifications();
                if (active != null) {
                    for (StatusBarNotification sbn : active) {
                        if (sbn != null && sbn.getId() == LIVE_CAPSULE_NOTIFICATION_ID && sbn.getTag() != null) {
                            tagsToCancel.add(sbn.getTag());
                        }
                    }
                }
            } catch (Exception ignored) {
            }
            for (String tag : tagsToCancel) {
                try {
                    nm.cancel(tag, LIVE_CAPSULE_NOTIFICATION_ID);
                    cleared++;
                } catch (Exception ignored) {
                }
            }
            try {
                nm.cancel(LIVE_CAPSULE_NOTIFICATION_ID);
            } catch (Exception ignored) {
            }
        }
        return new JSONObject()
            .put("ok", true)
            .put("cleared", cleared);
    }

    private static JSONObject buildPhoneDigest(
            Context context,
            PushStore store,
            boolean listenerConnected,
            boolean accessibilityConnected,
            boolean wifiAdbEnabled,
            boolean screenInteractive,
            boolean keyguardLocked,
            boolean humanTouching) throws Exception {
        JSONObject battery = BatteryStateCache.snapshot(context);
        Object batteryLevel = battery.opt("level");
        int batteryStatus = battery.optInt("status", -1);
        boolean charging = battery.optInt("plugged", -1) > 0
            && (batteryStatus == BatteryManager.BATTERY_STATUS_CHARGING
                || batteryStatus == BatteryManager.BATTERY_STATUS_FULL);
        int thermalStatus = 0;
        boolean powerSave = false;

        if (context != null) {
            try {
                PowerManager pm = context.getSystemService(PowerManager.class);
                if (pm != null) {
                    thermalStatus = pm.getCurrentThermalStatus();
                    powerSave = pm.isPowerSaveMode();
                }
            } catch (Throwable ignored) {
            }
        }

        JSONObject snap = store != null ? store.statusSummary() : new JSONObject();
        return new JSONObject()
            .put("ok", true)
            .put("batteryState", battery)
            .put("batteryLevel", batteryLevel)
            .put("battery", batteryLevel)
            .put("charging", charging)
            .put("thermalStatus", thermalStatus)
            .put("thermal", thermalStatus)
            .put("interactive", screenInteractive)
            .put("screenInteractive", screenInteractive)
            .put("powerSave", powerSave)
            .put("powerSaveMode", powerSave)
            .put("keyguardLocked", keyguardLocked)
            .put("wifiAdbEnabled", wifiAdbEnabled)
            .put("listenerConnected", listenerConnected)
            .put("accessibilityConnected", accessibilityConnected)
            .put("humanTouchRecent", humanTouching)
            .put("revision", snap.optLong("revision", store != null ? store.getRevision() : 0L))
            .put("unreadImCount", snap.optInt("unreadImCount", 0))
            .put("activePushCount", snap.optInt("activePushCount", 0))
            .put("smartBackScreen", SmartBackScreenController.get(context).getStatusJson(context));
    }

    private static boolean isWifiAdbEnabled(Context context) {
        if (context == null) return false;
        try {
            return Settings.Global.getInt(context.getContentResolver(), "adb_wifi_enabled", 0) == 1;
        } catch (Exception ignored) {
            return false;
        }
    }

    private static boolean isScreenInteractive(Context context) {
        if (context == null) return false;
        try {
            PowerManager pm = context.getSystemService(PowerManager.class);
            return pm != null && pm.isInteractive();
        } catch (Exception ignored) {
            return false;
        }
    }

    private static boolean isKeyguardLocked(Context context) {
        if (context == null) return false;
        try {
            KeyguardManager km = context.getSystemService(KeyguardManager.class);
            return km != null && km.isKeyguardLocked();
        } catch (Exception ignored) {
            return false;
        }
    }

    @Override
    public Cursor query(Uri u, String[] p, String s, String[] a, String o) {
        throw new UnsupportedOperationException();
    }

    @Override
    public String getType(Uri u) {
        return null;
    }

    @Override
    public Uri insert(Uri u, ContentValues v) {
        throw new UnsupportedOperationException();
    }

    @Override
    public int delete(Uri u, String s, String[] a) {
        throw new UnsupportedOperationException();
    }

    @Override
    public int update(Uri u, ContentValues v, String s, String[] a) {
        throw new UnsupportedOperationException();
    }
}
