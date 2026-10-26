package com.stepx.companion.app;

import android.app.KeyguardManager;
import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.os.Binder;
import android.os.Bundle;
import android.os.PowerManager;
import android.os.Process;
import android.provider.Settings;
import android.util.Base64;
import org.json.JSONObject;
import java.nio.charset.StandardCharsets;

/**
 * Shell-only authenticated ContentProvider (content://com.stepx.companion.app.push)
 * for IM Chat Hub and Phone Push Notification Center RPCs.
 */
public final class PushControlProvider extends ContentProvider {
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
        PushStore store = PushStore.get(context);
        boolean listenerConnected = PushNotificationService.ensureRebound(context);
        boolean accessibilityConnected = GuiAccessibilityService.instance != null;
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
                .put("totalPushCount", snap.optInt("totalPushCount", 0));
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
            return snap;
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

            if ("ui".equalsIgnoreCase(mode)
                    || "ui_bridge".equalsIgnoreCase(mode)
                    || "ui_force".equalsIgnoreCase(mode)) {
                JSONObject res = ChatUiBridge.sendChatViaUi(context, pkg, title, text, force, clientMsgId);
                if (PushNotificationService.instance != null) {
                    PushNotificationService.instance.broadcastDelta(res);
                }
                return res;
            }

            if ("remote_input".equalsIgnoreCase(mode) || "remote_only".equalsIgnoreCase(mode)) {
                JSONObject res = store.executeRemoteReply(context, threadId, text, clientMsgId);
                if (PushNotificationService.instance != null) {
                    PushNotificationService.instance.broadcastDelta(res);
                }
                return res;
            }

            // Default mode = "auto": try RemoteInput first, fallback to ChatUiBridge if handle missing/expired
            try {
                JSONObject res = store.executeRemoteReply(context, threadId, text, clientMsgId);
                if (PushNotificationService.instance != null) {
                    PushNotificationService.instance.broadcastDelta(res);
                }
                return res;
            } catch (Exception remoteErr) {
                if (GuiAccessibilityService.instance != null) {
                    JSONObject res = ChatUiBridge.sendChatViaUi(context, pkg, title, text, force, clientMsgId);
                    if (PushNotificationService.instance != null) {
                        PushNotificationService.instance.broadcastDelta(res);
                    }
                    return res;
                }
                throw remoteErr;
            }
        }

        if ("sync_window".equals(op)) {
            JSONObject res = ChatUiBridge.captureActiveChatWindow(context);
            if (PushNotificationService.instance != null) {
                PushNotificationService.instance.broadcastDelta(res);
            }
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
            return store.openPush(context, key, pkg);
        }

        throw new IllegalArgumentException("未知推送操作: " + op);
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
