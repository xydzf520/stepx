package com.stepx.companion.app;

import android.app.Notification;
import android.app.PendingIntent;
import android.app.Person;
import android.app.RemoteInput;
import android.content.ComponentName;
import android.content.ContentResolver;
import android.content.Context;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Path;
import android.graphics.RectF;
import android.graphics.drawable.BitmapDrawable;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.Icon;
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
import android.util.Base64;
import org.json.JSONObject;
import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.ByteArrayOutputStream;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Real-time notification listener, RemoteInput reply handle harvester, LocalServerSocket
 * ("stepx_push_v1") low-latency stream server, and same-Wi-Fi wireless debugging self-healer.
 */
public final class PushNotificationService extends NotificationListenerService {
    public static final String SOCKET_NAME = "stepx_push_v1";
    public static final long RESCAN_COOLDOWN_MS = 30_000L;
    static final int MAX_SOCKET_LINE_BYTES = McpRoutePolicy.MAX_SOCKET_LINE_BYTES;
    private static final int MAX_AVATAR_CACHE = 256;
    private static final int SOCKET_READ_TIMEOUT_MS = 5000;
    private static final int MAX_SUBSCRIBERS = 4;
    static volatile PushNotificationService instance;

    private static final class SubscriberSession {
        final LocalSocket socket;
        final BufferedWriter writer;

        SubscriberSession(LocalSocket socket, BufferedWriter writer) {
            this.socket = socket;
            this.writer = writer;
        }
    }

    private volatile boolean running;
    private volatile long lastFullScanElapsedMs = 0L;
    private LocalServerSocket serverSocket;
    private Thread socketAcceptThread;
    private ThreadPoolExecutor socketClientExecutor;
    private ThreadPoolExecutor subscriberExecutor;
    private ThreadPoolExecutor ingestExecutor;
    private volatile Thread ingestThread;
    private ExecutorService deltaExecutor;
    private ConnectivityManager.NetworkCallback wifiCallback;
    private final Set<LocalSocket> activeClientSockets =
        Collections.newSetFromMap(new ConcurrentHashMap<LocalSocket, Boolean>());
    private final CopyOnWriteArrayList<SubscriberSession> subscribers = new CopyOnWriteArrayList<SubscriberSession>();
    private final Map<String, String> avatarCache = new LinkedHashMap<String, String>(256, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, String> eldest) {
            return size() > MAX_AVATAR_CACHE;
        }
    };
    private PhoneCallMonitor callMonitor;

    public static boolean isConnected() {
        return instance != null;
    }

    public boolean hasRecentFullScan(long maxAgeMs) {
        long last = lastFullScanElapsedMs;
        if (last <= 0L) return false;
        long age = android.os.SystemClock.elapsedRealtime() - last;
        return age >= 0L && age < maxAgeMs;
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
        PushStore store = PushStore.get(context);
        JSONObject delta = store.onNotificationRemoved(
            trimmed, NotificationListenerService.REASON_CANCEL, System.currentTimeMillis()
        );
        if (svc != null && delta != null) {
            svc.broadcastDelta(delta);
        }
        SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
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
        SmartBackScreenController.get(context).onNotificationsUpdated(context, store);
        return count;
    }

    public static JSONObject postLiveCapsule(Context context, JSONObject q) throws Exception {
        return PushControlProvider.postLiveCapsule(context, q);
    }

    public static JSONObject dismissLiveCapsule(Context context, String tag) throws Exception {
        JSONObject res = PushControlProvider.dismissLiveCapsule(context, tag);
        PushNotificationService svc = instance;
        if (svc != null && tag != null && !tag.trim().isEmpty()) {
            try {
                StatusBarNotification[] active = svc.getActiveNotifications();
                if (active != null) {
                    for (StatusBarNotification sbn : active) {
                        if (sbn != null
                                && "com.stepx.companion.app".equals(sbn.getPackageName())
                                && sbn.getId() == PushControlProvider.LIVE_CAPSULE_NOTIFICATION_ID
                                && tag.trim().equals(sbn.getTag())) {
                            svc.cancelNotification(sbn.getKey());
                        }
                    }
                }
            } catch (Exception ignored) {
            }
        }
        return res;
    }

    public static JSONObject clearLiveCapsules(Context context) throws Exception {
        JSONObject res = PushControlProvider.clearLiveCapsules(context);
        PushNotificationService svc = instance;
        if (svc != null) {
            try {
                StatusBarNotification[] active = svc.getActiveNotifications();
                if (active != null) {
                    for (StatusBarNotification sbn : active) {
                        if (sbn != null
                                && "com.stepx.companion.app".equals(sbn.getPackageName())
                                && sbn.getId() == PushControlProvider.LIVE_CAPSULE_NOTIFICATION_ID) {
                            svc.cancelNotification(sbn.getKey());
                        }
                    }
                }
            } catch (Exception ignored) {
            }
        }
        return res;
    }

    void restartCallMonitor() {
        if (callMonitor != null) callMonitor.stop();
        callMonitor = new PhoneCallMonitor(this);
        callMonitor.start();
    }

    private synchronized ThreadPoolExecutor ensureIngestExecutor() {
        if (ingestExecutor == null || ingestExecutor.isShutdown() || ingestExecutor.isTerminated()) {
            ingestExecutor = new ThreadPoolExecutor(
                1,
                1,
                0L,
                TimeUnit.MILLISECONDS,
                new ArrayBlockingQueue<Runnable>(256),
                new ThreadFactory() {
                    @Override
                    public Thread newThread(Runnable r) {
                        Thread t = new Thread(r, "StepXPushIngest");
                        t.setDaemon(true);
                        ingestThread = t;
                        return t;
                    }
                },
                new ThreadPoolExecutor.DiscardOldestPolicy()
            );
        }
        return ingestExecutor;
    }

    private synchronized void shutdownIngestExecutor() {
        ingestThread = null;
        if (ingestExecutor != null) {
            ingestExecutor.shutdownNow();
            ingestExecutor = null;
        }
    }

    @Override
    public void onListenerConnected() {
        super.onListenerConnected();
        instance = this;
        running = true;
        ensureIngestExecutor();
        restartCallMonitor();
        BatteryStateCache.start(this);
        startSocketServer();
        registerWifiSelfHealCallback();
        scanActiveNotifications();
    }

    @Override
    public void onListenerDisconnected() {
        if (callMonitor != null) callMonitor.stop();
        BatteryStateCache.stop();
        PushStore.get(this).recordListenerGap();
        lastFullScanElapsedMs = 0L;
        if (instance == this) {
            instance = null;
        }
        stopSocketServer();
        shutdownIngestExecutor();
        unregisterWifiSelfHealCallback();
        super.onListenerDisconnected();
    }

    @Override
    public void onDestroy() {
        if (callMonitor != null) callMonitor.stop();
        BatteryStateCache.stop();
        PushStore.get(this).recordListenerGap();
        running = false;
        lastFullScanElapsedMs = 0L;
        if (instance == this) {
            instance = null;
        }
        stopSocketServer();
        shutdownIngestExecutor();
        unregisterWifiSelfHealCallback();
        shutdownDeltaExecutor();
        synchronized (avatarCache) {
            avatarCache.clear();
        }
        super.onDestroy();
    }

    public void scanActiveNotifications() {
        if (Thread.currentThread() == ingestThread) {
            scanActiveNotificationsSync();
            return;
        }
        if (android.os.Looper.myLooper() == android.os.Looper.getMainLooper()) {
            try {
                ensureIngestExecutor().execute(new Runnable() {
                    @Override
                    public void run() {
                        scanActiveNotificationsSync();
                    }
                });
            } catch (Exception ignored) {
            }
            return;
        }
        try {
            java.util.concurrent.Future<?> f = ensureIngestExecutor().submit(new Runnable() {
                @Override
                public void run() {
                    scanActiveNotificationsSync();
                }
            });
            f.get(4000L, TimeUnit.MILLISECONDS);
        } catch (Exception ignored) {
        }
    }

    private void scanActiveNotificationsSync() {
        try {
            StatusBarNotification[] active = getActiveNotifications();
            if (active == null) return;
            PushStore store = PushStore.get(this);
            java.util.Set<String> activeKeys = new java.util.HashSet<String>();
            store.beginBatch();
            try {
                for (StatusBarNotification sbn : active) {
                    if (sbn != null) {
                        if (sbn.getKey() != null && !sbn.getKey().isEmpty()) {
                            activeKeys.add(sbn.getKey());
                        }
                        ingestStatusBarNotification(sbn, false);
                    }
                }
                store.reconcileActiveNotifications(activeKeys, System.currentTimeMillis());
            } finally {
                store.endBatch();
            }
            lastFullScanElapsedMs = android.os.SystemClock.elapsedRealtime();
            SmartBackScreenController.get(this).onNotificationsUpdated(this, store);
        } catch (Exception ignored) {
        }
    }

    @Override
    public void onNotificationPosted(final StatusBarNotification sbn) {
        if (sbn == null) return;
        if ("com.stepx.companion.app".equals(sbn.getPackageName())) return;
        acquireShortWakeLock();
        try {
            ensureIngestExecutor().execute(new Runnable() {
                @Override
                public void run() {
                    ingestStatusBarNotification(sbn, true);
                }
            });
        } catch (Exception ignored) {
        }
    }

    @Override
    public void onNotificationRemoved(StatusBarNotification sbn, RankingMap rankingMap, final int reason) {
        if (sbn == null) return;
        final String key = sbn.getKey();
        final long removedAt = System.currentTimeMillis();
        try {
            ensureIngestExecutor().execute(new Runnable() {
                @Override
                public void run() {
                    try {
                        PushStore store = PushStore.get(PushNotificationService.this);
                        JSONObject delta = store.onNotificationRemoved(key, reason, removedAt);
                        if (delta != null) {
                            broadcastDelta(delta);
                        }
                        SmartBackScreenController.get(PushNotificationService.this)
                            .onNotificationsUpdated(PushNotificationService.this, store);
                    } catch (Exception ignored) {
                    }
                }
            });
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
            if (callMonitor != null) callMonitor.notification(sbn.getPackageName(), n);

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
            int progressCurrent = 0;
            int progressMax = 0;
            boolean progressIndeterminate = false;
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
                progressCurrent = extras.getInt(Notification.EXTRA_PROGRESS, 0);
                progressMax = extras.getInt(Notification.EXTRA_PROGRESS_MAX, 0);
                progressIndeterminate = extras.getBoolean(Notification.EXTRA_PROGRESS_INDETERMINATE, false);
                hasProgress = progressMax > 0 || progressIndeterminate;

                // Extract MessagingStyle messages (EXTRA_MESSAGES)
                String selfDisplayName = charSeqToString(extras.getCharSequence("android.selfDisplayName"));
                Parcelable[] rawMsgs = extras.getParcelableArray(Notification.EXTRA_MESSAGES);
                if (rawMsgs != null && rawMsgs.length > 0) {
                    for (Parcelable p : rawMsgs) {
                        if (p instanceof Bundle) {
                            Bundle mb = (Bundle) p;
                            String mText = charSeqToString(mb.getCharSequence("text"));
                            String mSender = charSeqToString(mb.getCharSequence("sender"));
                            Parcelable personObj = mb.getParcelable("sender_person");
                            if (mSender.isEmpty() && personObj instanceof Person) {
                                mSender = charSeqToString(((Person) personObj).getName());
                            }
                            boolean isSelfReply = (mSender.isEmpty() && personObj == null)
                                || "我".equals(mSender)
                                || "Me".equalsIgnoreCase(mSender)
                                || "自己".equals(mSender)
                                || (!selfDisplayName.isEmpty() && selfDisplayName.equals(mSender));
                            if (isSelfReply) {
                                mSender = "我";
                            }
                            long mTime = mb.getLong("time", postTimeMs);
                            if (!mText.isEmpty()) {
                                styleMessages.add(new PushStore.StyleMessage(mSender, mText, mTime, isSelfReply));
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

            String avatarB64 = extractNotificationAvatarBase64(sbnKey, postTimeMs, n, extras);

            String appName = ImAppCatalog.appDisplayName(this, pkg);
            PushStore store = PushStore.get(this);
            JSONObject delta = store.ingestNotification(
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
                n.contentIntent,
                avatarB64,
                progressCurrent,
                progressMax,
                progressIndeterminate
            );

            if (broadcast) {
                if (delta != null) {
                    broadcastDelta(delta);
                }
                SmartBackScreenController.get(this).onNotificationsUpdated(this, store);
            }
        } catch (Exception ignored) {
        }
    }

    private String extractNotificationAvatarBase64(String sbnKey, long postTimeMs, Notification n, Bundle extras) {
        if (n == null) return "";
        String cacheKey = (sbnKey != null && !sbnKey.isEmpty()) ? (sbnKey + "|" + postTimeMs) : "";
        if (!cacheKey.isEmpty()) {
            synchronized (avatarCache) {
                String cached = avatarCache.get(cacheKey);
                if (cached != null) {
                    return cached;
                }
            }
        }
        String computed = extractNotificationAvatarBase64Uncached(n, extras);
        if (!cacheKey.isEmpty()) {
            synchronized (avatarCache) {
                avatarCache.put(cacheKey, computed);
            }
        }
        return computed;
    }

    private String extractNotificationAvatarBase64Uncached(Notification n, Bundle extras) {
        if (n == null) return "";
        try {
            // 1. Latest MessagingStyle.Message sender Person.getIcon()
            if (extras != null) {
                Parcelable[] rawMsgs = extras.getParcelableArray(Notification.EXTRA_MESSAGES);
                if (rawMsgs != null && rawMsgs.length > 0) {
                    for (int i = rawMsgs.length - 1; i >= 0; i--) {
                        Parcelable p = rawMsgs[i];
                        if (p instanceof Bundle) {
                            Parcelable personObj = ((Bundle) p).getParcelable("sender_person");
                            if (personObj instanceof Person) {
                                Icon personIcon = ((Person) personObj).getIcon();
                                if (personIcon != null) {
                                    String b64 = iconToBase64Png(personIcon, null);
                                    if (!b64.isEmpty()) return b64;
                                }
                            }
                        }
                    }
                }
                // 2. android.conversationIcon or EXTRA_LARGE_ICON_BIG
                Parcelable convIcon = extras.getParcelable("android.conversationIcon");
                if (convIcon instanceof Icon) {
                    String b64 = iconToBase64Png((Icon) convIcon, null);
                    if (!b64.isEmpty()) return b64;
                } else if (convIcon instanceof Bitmap) {
                    String b64 = iconToBase64Png(null, new BitmapDrawable(getResources(), (Bitmap) convIcon));
                    if (!b64.isEmpty()) return b64;
                }
                Parcelable bigIcon = extras.getParcelable(Notification.EXTRA_LARGE_ICON_BIG);
                if (bigIcon instanceof Icon) {
                    String b64 = iconToBase64Png((Icon) bigIcon, null);
                    if (!b64.isEmpty()) return b64;
                } else if (bigIcon instanceof Bitmap) {
                    String b64 = iconToBase64Png(null, new BitmapDrawable(getResources(), (Bitmap) bigIcon));
                    if (!b64.isEmpty()) return b64;
                }
            }
            // 3. Notification.getLargeIcon()
            Icon largeIcon = n.getLargeIcon();
            if (largeIcon != null) {
                String b64 = iconToBase64Png(largeIcon, null);
                if (!b64.isEmpty()) return b64;
            }
            // 4. EXTRA_LARGE_ICON (Icon or Bitmap)
            if (extras != null) {
                Parcelable extraLarge = extras.getParcelable(Notification.EXTRA_LARGE_ICON);
                if (extraLarge instanceof Icon) {
                    String b64 = iconToBase64Png((Icon) extraLarge, null);
                    if (!b64.isEmpty()) return b64;
                } else if (extraLarge instanceof Bitmap) {
                    String b64 = iconToBase64Png(null, new BitmapDrawable(getResources(), (Bitmap) extraLarge));
                    if (!b64.isEmpty()) return b64;
                }
            }
        } catch (Throwable ignored) {
        }
        return "";
    }

    private String iconToBase64Png(Icon icon, Drawable fallbackDrawable) {
        try {
            Drawable d = null;
            if (icon != null) {
                d = icon.loadDrawable(this);
            }
            if (d == null) {
                d = fallbackDrawable;
            }
            if (d == null) return "";
            Bitmap bmp = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888);
            try {
                Canvas canvas = new Canvas(bmp);
                Path clip = new Path();
                clip.addRoundRect(new RectF(0f, 0f, 64f, 64f), 12f, 12f, Path.Direction.CW);
                canvas.clipPath(clip);
                d.setBounds(0, 0, 64, 64);
                d.draw(canvas);
                ByteArrayOutputStream baos = new ByteArrayOutputStream();
                bmp.compress(Bitmap.CompressFormat.PNG, 100, baos);
                return Base64.encodeToString(baos.toByteArray(), Base64.NO_WRAP);
            } finally {
                bmp.recycle();
            }
        } catch (Throwable ignored) {
            return "";
        }
    }

    private static PushStore.RemoteReplyHandle extractRemoteReplyHandle(
            Notification n,
            String sbnKey,
            String pkg,
            String rawTitle,
            long postTimeMs) {
        if (n == null) return null;
        String cleanTitle = ImAppCatalog.cleanConversationTitle(pkg, rawTitle);
        String threadId = ImAppCatalog.buildThreadId(pkg, cleanTitle);

        Notification.Action fallbackAction = null;
        RemoteInput[] fallbackInputs = null;

        if (n.actions != null) {
            for (Notification.Action action : n.actions) {
                if (action == null || action.actionIntent == null) continue;
                RemoteInput[] inputs = action.getRemoteInputs();
                if (inputs != null && inputs.length > 0) {
                    if (isPreferredReplyAction(action, inputs)) {
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
                    if (fallbackAction == null) {
                        fallbackAction = action;
                        fallbackInputs = inputs;
                    }
                }
            }
            if (fallbackAction != null && fallbackInputs != null) {
                return new PushStore.RemoteReplyHandle(
                    fallbackAction.actionIntent,
                    fallbackInputs,
                    n.contentIntent,
                    sbnKey,
                    pkg,
                    cleanTitle,
                    threadId,
                    postTimeMs
                );
            }
        }

        try {
            List<Notification.Action> wearableActions = new Notification.WearableExtender(n).getActions();
            if (wearableActions != null) {
                Notification.Action wearableFallbackAction = null;
                RemoteInput[] wearableFallbackInputs = null;
                for (Notification.Action action : wearableActions) {
                    if (action == null || action.actionIntent == null) continue;
                    RemoteInput[] inputs = action.getRemoteInputs();
                    if (inputs != null && inputs.length > 0) {
                        if (isPreferredReplyAction(action, inputs)) {
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
                        if (wearableFallbackAction == null) {
                            wearableFallbackAction = action;
                            wearableFallbackInputs = inputs;
                        }
                    }
                }
                if (wearableFallbackAction != null && wearableFallbackInputs != null) {
                    return new PushStore.RemoteReplyHandle(
                        wearableFallbackAction.actionIntent,
                        wearableFallbackInputs,
                        n.contentIntent,
                        sbnKey,
                        pkg,
                        cleanTitle,
                        threadId,
                        postTimeMs
                    );
                }
            }
        } catch (Throwable ignored) {
        }

        try {
            Notification.CarExtender.UnreadConversation unread =
                new Notification.CarExtender(n).getUnreadConversation();
            if (unread != null && unread.getReplyPendingIntent() != null && unread.getRemoteInput() != null) {
                return new PushStore.RemoteReplyHandle(
                    unread.getReplyPendingIntent(),
                    new RemoteInput[] { unread.getRemoteInput() },
                    n.contentIntent,
                    sbnKey,
                    pkg,
                    cleanTitle,
                    threadId,
                    postTimeMs
                );
            }
        } catch (Throwable ignored) {
        }

        if (n.extras != null) {
            try {
                Bundle carExt = n.extras.getBundle("android.car.EXTENSIONS");
                if (carExt == null) {
                    carExt = n.extras.getBundle("app_extensions");
                }
                if (carExt != null) {
                    Bundle carConv = carExt.getBundle("car_conversation");
                    if (carConv == null) {
                        carConv = carExt;
                    }
                    Parcelable replyPi = carConv.getParcelable("on_reply");
                    if (replyPi == null) {
                        replyPi = carConv.getParcelable("reply");
                    }
                    Parcelable remoteIn = carConv.getParcelable("remote_input");
                    if (replyPi instanceof PendingIntent && remoteIn instanceof RemoteInput) {
                        return new PushStore.RemoteReplyHandle(
                            (PendingIntent) replyPi,
                            new RemoteInput[] { (RemoteInput) remoteIn },
                            n.contentIntent,
                            sbnKey,
                            pkg,
                            cleanTitle,
                            threadId,
                            postTimeMs
                        );
                    }
                }
            } catch (Throwable ignored) {
            }
        }
        return null;
    }

    private static boolean isPreferredReplyAction(Notification.Action action, RemoteInput[] inputs) {
        if (action == null || inputs == null) return false;
        try {
            if (action.getSemanticAction() == Notification.Action.SEMANTIC_ACTION_REPLY) {
                return true;
            }
        } catch (Throwable ignored) {
        }
        for (RemoteInput input : inputs) {
            if (input != null && input.getAllowFreeFormInput()) {
                return true;
            }
        }
        return false;
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

    private synchronized ThreadPoolExecutor ensureSocketClientExecutor() {
        if (socketClientExecutor == null || socketClientExecutor.isShutdown() || socketClientExecutor.isTerminated()) {
            ThreadPoolExecutor exec = new ThreadPoolExecutor(
                2,
                4,
                30L,
                TimeUnit.SECONDS,
                new ArrayBlockingQueue<Runnable>(16),
                new ThreadFactory() {
                    private final AtomicInteger seq = new AtomicInteger(1);

                    @Override
                    public Thread newThread(Runnable r) {
                        Thread t = new Thread(r, "StepXPushClient-" + seq.getAndIncrement());
                        t.setDaemon(true);
                        return t;
                    }
                }
            );
            exec.allowCoreThreadTimeOut(true);
            socketClientExecutor = exec;
        }
        return socketClientExecutor;
    }

    private synchronized ExecutorService ensureDeltaExecutor() {
        if (deltaExecutor == null || deltaExecutor.isShutdown() || deltaExecutor.isTerminated()) {
            deltaExecutor = Executors.newSingleThreadExecutor(new ThreadFactory() {
                @Override
                public Thread newThread(Runnable r) {
                    Thread t = new Thread(r, "StepXPushDelta");
                    t.setDaemon(true);
                    return t;
                }
            });
        }
        return deltaExecutor;
    }

    private synchronized void shutdownDeltaExecutor() {
        if (deltaExecutor != null) {
            deltaExecutor.shutdownNow();
            deltaExecutor = null;
        }
    }

    private synchronized void startSocketServer() {
        if (socketAcceptThread != null && socketAcceptThread.isAlive()) return;
        ensureSocketClientExecutor();
        socketAcceptThread = new Thread(new Runnable() {
            @Override
            public void run() {
                try {
                    serverSocket = new LocalServerSocket(SOCKET_NAME);
                    while (running && serverSocket != null) {
                        final LocalSocket client = serverSocket.accept();
                        if (client == null) continue;
                        activeClientSockets.add(client);
                        try {
                            ensureSocketClientExecutor().execute(new Runnable() {
                                @Override
                                public void run() {
                                    handleLocalSocketClient(client);
                                }
                            });
                        } catch (Throwable rejected) {
                            activeClientSockets.remove(client);
                            try {
                                client.close();
                            } catch (Exception ignored) {
                            }
                        }
                    }
                } catch (Exception ignored) {
                }
            }
        }, "StepXPushSocketAccept");
        socketAcceptThread.setDaemon(true);
        socketAcceptThread.start();
    }

    private synchronized ThreadPoolExecutor ensureSubscriberExecutor() {
        if (subscriberExecutor == null || subscriberExecutor.isShutdown() || subscriberExecutor.isTerminated()) {
            ThreadPoolExecutor exec = new ThreadPoolExecutor(
                1,
                MAX_SUBSCRIBERS,
                30L,
                TimeUnit.SECONDS,
                new ArrayBlockingQueue<Runnable>(MAX_SUBSCRIBERS),
                new ThreadFactory() {
                    private final AtomicInteger seq = new AtomicInteger(1);

                    @Override
                    public Thread newThread(Runnable r) {
                        Thread t = new Thread(r, "StepXPushSub-" + seq.getAndIncrement());
                        t.setDaemon(true);
                        return t;
                    }
                }
            );
            exec.allowCoreThreadTimeOut(true);
            subscriberExecutor = exec;
        }
        return subscriberExecutor;
    }

    private void closeSubscriber(SubscriberSession session) {
        if (session == null) return;
        subscribers.remove(session);
        activeClientSockets.remove(session.socket);
        try {
            session.socket.close();
        } catch (Exception ignored) {
        }
    }

    private synchronized void stopSocketServer() {
        try {
            if (serverSocket != null) {
                serverSocket.close();
                serverSocket = null;
            }
        } catch (Exception ignored) {
        }
        for (SubscriberSession sub : subscribers) {
            closeSubscriber(sub);
        }
        subscribers.clear();
        for (LocalSocket client : activeClientSockets) {
            try {
                client.close();
            } catch (Exception ignored) {
            }
        }
        activeClientSockets.clear();
        if (subscriberExecutor != null) {
            subscriberExecutor.shutdownNow();
            subscriberExecutor = null;
        }
        if (socketClientExecutor != null) {
            socketClientExecutor.shutdownNow();
            socketClientExecutor = null;
        }
    }

    private JSONObject dispatchSocketJsonRequest(JSONObject req) throws Exception {
        String op = req.optString("op", "");
        JSONObject response;
        if ("expansion_call".equals(op)
                || op.startsWith("expansion:")
                || "expansion".equals(req.optString("provider", ""))) {
            response = handleExpansionSocketRequest(req);
        } else {
            response = PushControlProvider.handleRequest(this, req, false);
        }
        if (req.has("reqId")) {
            response.put("reqId", req.opt("reqId"));
        }
        return response;
    }

    private boolean handOffSubscriber(
            final LocalSocket client,
            final McpRoutePolicy.BoundedUtf8LineReader reader,
            final BufferedWriter writer) {
        if (!running) return false;
        while (subscribers.size() >= MAX_SUBSCRIBERS) {
            SubscriberSession oldest = subscribers.isEmpty() ? null : subscribers.get(0);
            if (oldest == null) break;
            closeSubscriber(oldest);
        }
        final SubscriberSession session = new SubscriberSession(client, writer);
        subscribers.add(session);
        try {
            ensureSubscriberExecutor().execute(new Runnable() {
                @Override
                public void run() {
                    runSubscriberLoop(session, reader);
                }
            });
            return true;
        } catch (Throwable rejected) {
            subscribers.remove(session);
            return false;
        }
    }

    private void runSubscriberLoop(SubscriberSession session, McpRoutePolicy.BoundedUtf8LineReader reader) {
        int idleTimeouts = 0;
        try {
            while (running && subscribers.contains(session)) {
                final String line;
                try {
                    line = reader.readLine();
                    idleTimeouts = 0;
                } catch (SocketTimeoutException timeout) {
                    if (!running) break;
                    if (++idleTimeouts >= 3) {
                        idleTimeouts = 0;
                        synchronized (session.writer) {
                            session.writer.write('\n');
                            session.writer.flush();
                        }
                    }
                    continue;
                }
                if (line == null) break;
                String trimmed = line.trim();
                if (trimmed.isEmpty()) continue;
                JSONObject response;
                try {
                    JSONObject req = new JSONObject(trimmed);
                    if ("subscribe".equals(req.optString("op", ""))) {
                        response = new JSONObject()
                            .put("ok", true)
                            .put("subscribed", true)
                            .put("revision", PushStore.get(this).getRevision());
                        if (req.has("reqId")) response.put("reqId", req.opt("reqId"));
                    } else {
                        response = dispatchSocketJsonRequest(req);
                    }
                } catch (Exception e) {
                    response = new JSONObject()
                        .put("ok", false)
                        .put("error", String.valueOf(e.getMessage()));
                }
                synchronized (session.writer) {
                    session.writer.write(response.toString());
                    session.writer.write('\n');
                    session.writer.flush();
                }
            }
        } catch (Exception ignored) {
        } finally {
            closeSubscriber(session);
        }
    }

    private void handleLocalSocketClient(LocalSocket client) {
        BufferedWriter writer = null;
        boolean handedOffToSubscriber = false;
        try {
            client.setSoTimeout(SOCKET_READ_TIMEOUT_MS);
            Credentials creds = client.getPeerCredentials();
            int uid = creds.getUid();
            if (uid != 2000 && uid != Process.myUid()) {
                return;
            }
            McpRoutePolicy.BoundedUtf8LineReader reader = new McpRoutePolicy.BoundedUtf8LineReader(
                client.getInputStream(), MAX_SOCKET_LINE_BYTES
            );
            writer = new BufferedWriter(
                new OutputStreamWriter(client.getOutputStream(), StandardCharsets.UTF_8)
            );

            while (running) {
                final String line;
                try {
                    line = reader.readLine();
                } catch (SocketTimeoutException timeout) {
                    break;
                }
                if (line == null) break;
                String trimmed = line.trim();
                if (trimmed.isEmpty()) continue;
                JSONObject response;
                boolean subscribeRequested = false;
                try {
                    JSONObject req = new JSONObject(trimmed);
                    String op = req.optString("op", "");
                    if ("subscribe".equals(op)) {
                        subscribeRequested = true;
                        response = new JSONObject()
                            .put("ok", true)
                            .put("subscribed", true)
                            .put("revision", PushStore.get(this).getRevision());
                        if (req.has("reqId")) {
                            response.put("reqId", req.opt("reqId"));
                        }
                    } else {
                        response = dispatchSocketJsonRequest(req);
                    }
                } catch (Exception e) {
                    subscribeRequested = false;
                    response = new JSONObject()
                        .put("ok", false)
                        .put("error", String.valueOf(e.getMessage()));
                }
                synchronized (writer) {
                    writer.write(response.toString());
                    writer.write('\n');
                    writer.flush();
                }
                if (subscribeRequested) {
                    handedOffToSubscriber = handOffSubscriber(client, reader, writer);
                    break;
                }
            }
        } catch (Exception ignored) {
        } finally {
            if (!handedOffToSubscriber) {
                activeClientSockets.remove(client);
                try {
                    client.close();
                } catch (Exception ignored) {
                }
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
        if (delta == null || subscribers.isEmpty()) return;
        final String payload = delta.toString();
        try {
            ensureDeltaExecutor().execute(new Runnable() {
                @Override
                public void run() {
                    for (SubscriberSession session : subscribers) {
                        try {
                            synchronized (session.writer) {
                                session.writer.write(payload);
                                session.writer.write('\n');
                                session.writer.flush();
                            }
                        } catch (Exception broken) {
                            closeSubscriber(session);
                        }
                    }
                }
            });
        } catch (Exception ignored) {
        }
    }
}
