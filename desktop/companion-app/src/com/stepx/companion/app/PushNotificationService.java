package com.stepx.companion.app;

import android.app.Notification;
import android.app.PendingIntent;
import android.app.Person;
import android.app.RemoteInput;
import android.content.ComponentName;
import android.content.ContentResolver;
import android.content.Context;
import android.content.pm.PackageManager;
import android.net.Credentials;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.NetworkRequest;
import android.os.Bundle;
import android.os.Parcelable;
import android.os.PowerManager;
import android.os.Process;
import android.provider.Settings;
import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;
import org.json.JSONObject;
import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Real-time notification listener, RemoteInput reply handle harvester, LocalServerSocket
 * ("stepx_push_v1") low-latency stream server, and same-Wi-Fi wireless debugging self-healer.
 */
public final class PushNotificationService extends NotificationListenerService {
    public static final String SOCKET_NAME = "stepx_push_v1";
    static volatile PushNotificationService instance;

    private volatile boolean running;
    private LocalServerSocket serverSocket;
    private Thread socketAcceptThread;
    private ConnectivityManager.NetworkCallback wifiCallback;
    private final CopyOnWriteArrayList<BufferedWriter> streamClients = new CopyOnWriteArrayList<BufferedWriter>();
    private final ExecutorService deltaExecutor = Executors.newSingleThreadExecutor();

    public static boolean isConnected() {
        return instance != null;
    }

    /**
     * Watchdog helper: ensures NotificationListenerService is enabled (if WRITE_SECURE_SETTINGS
     * is granted) and triggers requestRebind() if enabled in Settings.Secure but instance == null.
     */
    public static boolean ensureRebound(Context context) {
        if (instance != null) return true;
        if (context == null) return false;
        try {
            ContentResolver cr = context.getContentResolver();
            ComponentName cn = new ComponentName(context, PushNotificationService.class);
            String flat = cn.flattenToString();
            String shortFlat = cn.flattenToShortString();
            String enabled = Settings.Secure.getString(cr, "enabled_notification_listeners");
            boolean isEnabled = enabled != null && (enabled.contains(flat) || enabled.contains(shortFlat));

            if (!isEnabled && context.checkSelfPermission("android.permission.WRITE_SECURE_SETTINGS")
                    == PackageManager.PERMISSION_GRANTED) {
                String updated = (enabled == null || enabled.trim().isEmpty() || "null".equals(enabled.trim()))
                    ? flat
                    : (enabled + ":" + flat);
                Settings.Secure.putString(cr, "enabled_notification_listeners", updated);
                isEnabled = true;
            }

            if (isEnabled && instance == null) {
                requestRebind(cn);
            }
        } catch (Exception ignored) {
        }
        return instance != null;
    }

    public static void dismissNotification(Context context, String key) throws Exception {
        if (key == null || key.trim().isEmpty()) {
            throw new IllegalArgumentException("缺少通知 key");
        }
        String trimmed = key.trim();
        PushNotificationService svc = instance;
        if (svc != null) {
            try {
                svc.cancelNotification(trimmed);
            } catch (Exception ignored) {
            }
        }
        JSONObject delta = PushStore.get(context).onNotificationRemoved(
            trimmed, NotificationListenerService.REASON_CANCEL, System.currentTimeMillis()
        );
        if (svc != null && delta != null) {
            svc.broadcastDelta(delta);
        }
    }

    public static int dismissAllNotifications(Context context, String categoryFilter) throws Exception {
        PushStore store = PushStore.get(context);
        List<String> keys = store.getActiveDismissableKeys(categoryFilter);
        PushNotificationService svc = instance;
        int count = 0;
        long nowMs = System.currentTimeMillis();
        store.beginBatch();
        try {
            for (String key : keys) {
                if (svc != null) {
                    try {
                        svc.cancelNotification(key);
                    } catch (Exception ignored) {
                    }
                }
                JSONObject delta = store.onNotificationRemoved(
                    key, NotificationListenerService.REASON_CANCEL_ALL, nowMs
                );
                if (svc != null && delta != null) {
                    svc.broadcastDelta(delta);
                }
                count++;
            }
        } finally {
            store.endBatch();
        }
        return count;
    }

    @Override
    public void onListenerConnected() {
        super.onListenerConnected();
        instance = this;
        running = true;
        startSocketServer();
        registerWifiSelfHealCallback();
        scanActiveNotifications();
    }

    @Override
    public void onListenerDisconnected() {
        if (instance == this) {
            instance = null;
        }
        stopSocketServer();
        unregisterWifiSelfHealCallback();
        super.onListenerDisconnected();
    }

    @Override
    public void onDestroy() {
        running = false;
        if (instance == this) {
            instance = null;
        }
        stopSocketServer();
        unregisterWifiSelfHealCallback();
        deltaExecutor.shutdownNow();
        super.onDestroy();
    }

    public void scanActiveNotifications() {
        try {
            StatusBarNotification[] active = getActiveNotifications();
            if (active == null || active.length == 0) return;
            PushStore store = PushStore.get(this);
            store.beginBatch();
            try {
                for (StatusBarNotification sbn : active) {
                    if (sbn != null) {
                        ingestStatusBarNotification(sbn, false);
                    }
                }
            } finally {
                store.endBatch();
            }
        } catch (Exception ignored) {
        }
    }

    @Override
    public void onNotificationPosted(StatusBarNotification sbn) {
        if (sbn == null) return;
        acquireShortWakeLock();
        ingestStatusBarNotification(sbn, true);
    }

    @Override
    public void onNotificationRemoved(StatusBarNotification sbn, RankingMap rankingMap, int reason) {
        if (sbn == null) return;
        try {
            JSONObject delta = PushStore.get(this).onNotificationRemoved(
                sbn.getKey(), reason, System.currentTimeMillis()
            );
            if (delta != null) {
                broadcastDelta(delta);
            }
        } catch (Exception ignored) {
        }
    }

    @Override
    public void onNotificationRemoved(StatusBarNotification sbn) {
        onNotificationRemoved(sbn, null, NotificationListenerService.REASON_CANCEL);
    }

    private void acquireShortWakeLock() {
        try {
            PowerManager pm = getSystemService(PowerManager.class);
            if (pm != null) {
                PowerManager.WakeLock wl = pm.newWakeLock(
                    PowerManager.PARTIAL_WAKE_LOCK, "StepX:push-arrival"
                );
                wl.setReferenceCounted(false);
                wl.acquire(3000L);
            }
        } catch (Exception ignored) {
        }
    }

    private void ingestStatusBarNotification(StatusBarNotification sbn, boolean broadcast) {
        try {
            Notification n = sbn.getNotification();
            if (n == null) return;

            String pkg = sbn.getPackageName() == null ? "" : sbn.getPackageName();
            if ("com.stepx.companion.app".equals(pkg)) return;

            String sbnKey = sbn.getKey() == null ? "" : sbn.getKey();
            long postTimeMs = sbn.getPostTime() > 0 ? sbn.getPostTime() : System.currentTimeMillis();
            boolean ongoing = sbn.isOngoing() || ((n.flags & Notification.FLAG_ONGOING_EVENT) != 0);

            Bundle extras = n.extras;
            String title = "";
            String conversationTitle = "";
            String text = "";
            String bigText = "";
            String subText = "";
            boolean isGroup = false;
            boolean hasProgress = false;
            List<PushStore.StyleMessage> styleMessages = new ArrayList<PushStore.StyleMessage>();

            if (extras != null) {
                title = charSeqToString(extras.getCharSequence(Notification.EXTRA_TITLE));
                conversationTitle = charSeqToString(extras.getCharSequence(Notification.EXTRA_CONVERSATION_TITLE));
                text = charSeqToString(extras.getCharSequence(Notification.EXTRA_TEXT));
                bigText = charSeqToString(extras.getCharSequence(Notification.EXTRA_BIG_TEXT));
                subText = charSeqToString(extras.getCharSequence(Notification.EXTRA_SUB_TEXT));
                if (subText.isEmpty()) {
                    subText = charSeqToString(extras.getCharSequence(Notification.EXTRA_SUMMARY_TEXT));
                }
                isGroup = extras.getBoolean(Notification.EXTRA_IS_GROUP_CONVERSATION, false)
                    || !conversationTitle.isEmpty();
                int progressMax = extras.getInt(Notification.EXTRA_PROGRESS_MAX, 0);
                boolean progressIndeterminate = extras.getBoolean(Notification.EXTRA_PROGRESS_INDETERMINATE, false);
                hasProgress = progressMax > 0 || progressIndeterminate;

                // Extract MessagingStyle messages (EXTRA_MESSAGES)
                Parcelable[] rawMsgs = extras.getParcelableArray(Notification.EXTRA_MESSAGES);
                if (rawMsgs != null && rawMsgs.length > 0) {
                    for (Parcelable p : rawMsgs) {
                        if (p instanceof Bundle) {
                            Bundle mb = (Bundle) p;
                            String mText = charSeqToString(mb.getCharSequence("text"));
                            String mSender = charSeqToString(mb.getCharSequence("sender"));
                            if (mSender.isEmpty()) {
                                Parcelable personObj = mb.getParcelable("sender_person");
                                if (personObj instanceof Person) {
                                    mSender = charSeqToString(((Person) personObj).getName());
                                }
                            }
                            long mTime = mb.getLong("time", postTimeMs);
                            if (!mText.isEmpty()) {
                                styleMessages.add(new PushStore.StyleMessage(mSender, mText, mTime));
                            }
                        }
                    }
                }

                // Extract InboxStyle lines (EXTRA_TEXT_LINES) if EXTRA_MESSAGES was empty
                if (styleMessages.isEmpty()) {
                    CharSequence[] lines = extras.getCharSequenceArray(Notification.EXTRA_TEXT_LINES);
                    if (lines != null && lines.length > 0) {
                        String effectiveTitle = !conversationTitle.isEmpty() ? conversationTitle : title;
                        for (CharSequence lineObj : lines) {
                            String line = charSeqToString(lineObj);
                            if (!line.isEmpty()) {
                                ImAppCatalog.SenderBody sb = ImAppCatalog.extractSenderAndBody(
                                    pkg, effectiveTitle, line, isGroup
                                );
                                styleMessages.add(new PushStore.StyleMessage(sb.sender, sb.body, postTimeMs));
                            }
                        }
                        if (text.isEmpty() && !styleMessages.isEmpty()) {
                            PushStore.StyleMessage last = styleMessages.get(styleMessages.size() - 1);
                            text = last.sender.isEmpty() ? last.text : (last.sender + ": " + last.text);
                        }
                    }
                }
            }

            String fullText = !bigText.isEmpty() ? bigText : text;
            if (fullText.isEmpty() && !styleMessages.isEmpty()) {
                PushStore.StyleMessage last = styleMessages.get(styleMessages.size() - 1);
                fullText = last.text;
            }
            if (title.isEmpty() && conversationTitle.isEmpty() && fullText.isEmpty()) {
                return;
            }

            // Scan actions for RemoteInput reply handle
            PushStore.RemoteReplyHandle replyHandle = extractRemoteReplyHandle(
                n, sbnKey, pkg, !conversationTitle.isEmpty() ? conversationTitle : title, postTimeMs
            );

            String appName = ImAppCatalog.appDisplayName(this, pkg);
            JSONObject delta = PushStore.get(this).ingestNotification(
                sbnKey,
                pkg,
                appName,
                n.category,
                title,
                conversationTitle,
                fullText,
                subText,
                postTimeMs,
                ongoing,
                isGroup,
                hasProgress,
                styleMessages,
                replyHandle,
                n.contentIntent
            );

            if (broadcast && delta != null) {
                broadcastDelta(delta);
            }
        } catch (Exception ignored) {
        }
    }

    private static PushStore.RemoteReplyHandle extractRemoteReplyHandle(
            Notification n,
            String sbnKey,
            String pkg,
            String rawTitle,
            long postTimeMs) {
        if (n.actions == null) return null;
        for (Notification.Action action : n.actions) {
            if (action == null || action.actionIntent == null) continue;
            RemoteInput[] inputs = action.getRemoteInputs();
            if (inputs != null && inputs.length > 0) {
                String cleanTitle = ImAppCatalog.cleanConversationTitle(pkg, rawTitle);
                String threadId = ImAppCatalog.buildThreadId(pkg, cleanTitle);
                return new PushStore.RemoteReplyHandle(
                    action.actionIntent,
                    inputs,
                    n.contentIntent,
                    sbnKey,
                    pkg,
                    cleanTitle,
                    threadId,
                    postTimeMs
                );
            }
        }
        return null;
    }

    private static String charSeqToString(CharSequence cs) {
        return cs == null ? "" : cs.toString().trim();
    }

    private void registerWifiSelfHealCallback() {
        try {
            ConnectivityManager cm = getSystemService(ConnectivityManager.class);
            if (cm == null) return;
            unregisterWifiSelfHealCallback();
            NetworkRequest request = new NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .build();
            wifiCallback = new ConnectivityManager.NetworkCallback() {
                @Override
                public void onAvailable(Network network) {
                    healWirelessDebuggingIfPermitted();
                }

                @Override
                public void onCapabilitiesChanged(Network network, NetworkCapabilities caps) {
                    if (caps != null && caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
                        healWirelessDebuggingIfPermitted();
                    }
                }
            };
            cm.registerNetworkCallback(request, wifiCallback);
            healWirelessDebuggingIfPermitted();
        } catch (Exception ignored) {
        }
    }

    private void unregisterWifiSelfHealCallback() {
        try {
            if (wifiCallback != null) {
                ConnectivityManager cm = getSystemService(ConnectivityManager.class);
                if (cm != null) {
                    cm.unregisterNetworkCallback(wifiCallback);
                }
                wifiCallback = null;
            }
        } catch (Exception ignored) {
        }
    }

    private void healWirelessDebuggingIfPermitted() {
        try {
            if (checkSelfPermission("android.permission.WRITE_SECURE_SETTINGS")
                    == PackageManager.PERMISSION_GRANTED) {
                ContentResolver cr = getContentResolver();
                int current = Settings.Global.getInt(cr, "adb_wifi_enabled", 0);
                if (current == 0) {
                    Settings.Global.putInt(cr, "adb_wifi_enabled", 1);
                }
            }
        } catch (Exception ignored) {
        }
    }

    private synchronized void startSocketServer() {
        if (socketAcceptThread != null && socketAcceptThread.isAlive()) return;
        socketAcceptThread = new Thread(new Runnable() {
            @Override
            public void run() {
                try {
                    serverSocket = new LocalServerSocket(SOCKET_NAME);
                    while (running && serverSocket != null) {
                        final LocalSocket client = serverSocket.accept();
                        if (client == null) continue;
                        Thread handler = new Thread(new Runnable() {
                            @Override
                            public void run() {
                                handleLocalSocketClient(client);
                            }
                        }, "StepXPushClient");
                        handler.setDaemon(true);
                        handler.start();
                    }
                } catch (Exception ignored) {
                }
            }
        }, "StepXPushSocketAccept");
        socketAcceptThread.setDaemon(true);
        socketAcceptThread.start();
    }

    private synchronized void stopSocketServer() {
        try {
            if (serverSocket != null) {
                serverSocket.close();
                serverSocket = null;
            }
        } catch (Exception ignored) {
        }
        streamClients.clear();
    }

    private void handleLocalSocketClient(LocalSocket client) {
        BufferedWriter writer = null;
        try {
            Credentials creds = client.getPeerCredentials();
            int uid = creds.getUid();
            if (uid != 2000 && uid != Process.myUid()) {
                client.close();
                return;
            }
            BufferedReader reader = new BufferedReader(
                new InputStreamReader(client.getInputStream(), StandardCharsets.UTF_8)
            );
            writer = new BufferedWriter(
                new OutputStreamWriter(client.getOutputStream(), StandardCharsets.UTF_8)
            );

            String line;
            while (running && (line = reader.readLine()) != null) {
                String trimmed = line.trim();
                if (trimmed.isEmpty()) continue;
                JSONObject response;
                try {
                    JSONObject req = new JSONObject(trimmed);
                    String op = req.optString("op", "");
                    if ("subscribe".equals(op)) {
                        streamClients.addIfAbsent(writer);
                        response = new JSONObject()
                            .put("ok", true)
                            .put("subscribed", true)
                            .put("revision", PushStore.get(this).getRevision());
                    } else if ("expansion_call".equals(op)
                            || op.startsWith("expansion:")
                            || "expansion".equals(req.optString("provider", ""))) {
                        response = handleExpansionSocketRequest(req);
                    } else {
                        response = PushControlProvider.handleRequest(this, req);
                    }
                    if (req.has("reqId")) {
                        response.put("reqId", req.opt("reqId"));
                    }
                } catch (Exception e) {
                    response = new JSONObject()
                        .put("ok", false)
                        .put("error", String.valueOf(e.getMessage()));
                }
                synchronized (writer) {
                    writer.write(response.toString());
                    writer.write('\n');
                    writer.flush();
                }
            }
        } catch (Exception ignored) {
        } finally {
            if (writer != null) {
                streamClients.remove(writer);
            }
            try {
                client.close();
            } catch (Exception ignored) {
            }
        }
    }

    private JSONObject handleExpansionSocketRequest(JSONObject req) throws Exception {
        String op = req.optString("op", "");
        String method = req.optString("method", "request");
        if (op.startsWith("expansion:") && !req.has("method")) {
            method = op.substring("expansion:".length());
        }
        String argStr;
        if (req.has("arg")) {
            Object rawArg = req.opt("arg");
            argStr = rawArg == null ? "" : String.valueOf(rawArg);
        } else if (req.optJSONObject("query") != null) {
            argStr = req.optJSONObject("query").toString();
        } else {
            JSONObject copy = new JSONObject(req.toString());
            if ("expansion_call".equals(op) && !method.isEmpty() && !"request".equals(method)) {
                copy.put("op", method);
            } else if (op.startsWith("expansion:")) {
                copy.put("op", op.substring("expansion:".length()));
            }
            argStr = copy.toString();
        }
        Bundle bundle = getContentResolver().call(
            android.net.Uri.parse("content://com.stepx.companion.app.expansion"),
            method,
            argStr,
            null
        );
        if (bundle != null) {
            String resultJson = bundle.getString("result_json");
            if (resultJson != null && !resultJson.isEmpty()) {
                return new JSONObject(resultJson);
            }
            String b64 = bundle.getString("result");
            if (b64 != null && !b64.isEmpty()) {
                return new JSONObject(
                    new String(android.util.Base64.decode(b64, android.util.Base64.DEFAULT), StandardCharsets.UTF_8)
                );
            }
        }
        JSONObject q = ExpansionProvider.parseArg(method, argStr);
        return ExpansionProvider.handleRequest(this, q);
    }

    void broadcastDelta(JSONObject delta) {
        if (delta == null || streamClients.isEmpty()) return;
        final String payload = delta.toString();
        try {
            deltaExecutor.execute(new Runnable() {
                @Override
                public void run() {
                    for (BufferedWriter writer : streamClients) {
                        try {
                            synchronized (writer) {
                                writer.write(payload);
                                writer.write('\n');
                                writer.flush();
                            }
                        } catch (Exception broken) {
                            streamClients.remove(writer);
                        }
                    }
                }
            });
        } catch (Exception ignored) {
        }
    }
}
