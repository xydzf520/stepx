package com.stepx.companion.app;

import android.app.PendingIntent;
import android.app.RemoteInput;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.service.notification.NotificationListenerService;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.TimeUnit;

/**
 * Thread-safe persistent store for IM conversations (im_threads.json), full phone push feed
 * (push_feed.json), in-memory RemoteReplyHandle cache, and 64-entry clientMsgId idempotency ring.
 */
public final class PushStore {
    public static final int MAX_THREADS = 100;
    public static final int MAX_MESSAGES_PER_THREAD = 100;
    public static final int MAX_NOTIFICATIONS = 500;
    public static final int MAX_HANDLES = 80;
    public static final int MAX_CLIENT_MSG_IDS = 64;
    public static final long ONGOING_THROTTLE_WINDOW_MS = 2000L;
    public static final long OTP_TTL_MS = 15L * 60L * 1000L;

    private static boolean isNativeSmsThread(ThreadRecord t) {
        if (t == null) return false;
        if (ImAppCatalog.isNativeSmsPackage(t.packageName)) return true;
        if (t.threadId != null) {
            String tid = t.threadId.trim();
            if (tid.startsWith("com.android.mms::")
                    || tid.startsWith("com.google.android.apps.messaging::")
                    || tid.startsWith("com.samsung.android.messaging::")
                    || tid.startsWith("com.android.messaging::")) {
                return true;
            }
        }
        return false;
    }

    private static volatile PushStore singleton;

    public static PushStore get(Context context) {
        if (singleton == null) {
            synchronized (PushStore.class) {
                if (singleton == null) {
                    File dir = context != null ? context.getExternalFilesDir(null) : null;
                    singleton = new PushStore(dir);
                }
            }
        }
        return singleton;
    }

    public static PushStore getInstance(Context context) {
        return get(context);
    }

    public static final class RemoteReplyHandle {
        public final PendingIntent actionIntent;
        public final RemoteInput[] remoteInputs;
        public final PendingIntent contentIntent;
        public final String sbnKey;
        public final String packageName;
        public final String title;
        public final String threadId;
        public final long capturedAtMs;

        public RemoteReplyHandle(
                PendingIntent actionIntent,
                RemoteInput[] remoteInputs,
                PendingIntent contentIntent,
                String sbnKey,
                String packageName,
                String title,
                String threadId,
                long capturedAtMs) {
            this.actionIntent = actionIntent;
            this.remoteInputs = remoteInputs;
            this.contentIntent = contentIntent;
            this.sbnKey = sbnKey == null ? "" : sbnKey;
            this.packageName = packageName == null ? "" : packageName;
            this.title = title == null ? "" : title;
            this.threadId = threadId == null ? "" : threadId;
            this.capturedAtMs = capturedAtMs;
        }
    }

    public static final class StyleMessage {
        public final String sender;
        public final String text;
        public final long timestampMs;
        public final boolean outgoing;

        public StyleMessage(String sender, String text, long timestampMs) {
            this(sender, text, timestampMs, false);
        }

        public StyleMessage(String sender, String text, long timestampMs, boolean outgoing) {
            this.sender = sender == null ? "" : sender;
            this.text = text == null ? "" : text;
            this.timestampMs = timestampMs;
            this.outgoing = outgoing;
        }
    }

    public static final class CapturedBubble {
        public final String sender;
        public final String text;
        public final boolean outgoing;
        public final long timestampMs;
        public final String avatarB64;
        public final String mediaB64;

        public CapturedBubble(String sender, String text, boolean outgoing, long timestampMs) {
            this(sender, text, outgoing, timestampMs, "", "");
        }

        public CapturedBubble(String sender, String text, boolean outgoing, long timestampMs, String avatarB64) {
            this(sender, text, outgoing, timestampMs, avatarB64, "");
        }

        public CapturedBubble(String sender, String text, boolean outgoing, long timestampMs, String avatarB64, String mediaB64) {
            this.sender = sender == null ? "" : sender;
            this.text = text == null ? "" : text;
            this.outgoing = outgoing;
            this.timestampMs = timestampMs;
            this.avatarB64 = avatarB64 == null ? "" : avatarB64;
            this.mediaB64 = mediaB64 == null ? "" : mediaB64;
        }

        public CapturedBubble(String sender, String text, boolean outgoing, String timeLabel, String avatarB64) {
            this(sender, text, outgoing, 0L, avatarB64, "");
        }
    }

    public static final class CapturedConversationRow {
        public final String rawTitle;
        public final String previewText;
        public final String senderHint;
        public final int unreadCount;
        public final String avatarB64;

        public CapturedConversationRow(String rawTitle, String previewText, String senderHint, int unreadCount) {
            this(rawTitle, previewText, senderHint, unreadCount, "");
        }

        public CapturedConversationRow(
                String rawTitle,
                String previewText,
                String senderHint,
                int unreadCount,
                String avatarB64) {
            this.rawTitle = rawTitle == null ? "" : rawTitle.trim();
            this.previewText = previewText == null ? "" : previewText.trim();
            this.senderHint = senderHint == null ? "" : senderHint.trim();
            this.unreadCount = Math.max(0, unreadCount);
            this.avatarB64 = avatarB64 == null ? "" : avatarB64.trim();
        }
    }

    public static final class MessageRecord {
        public String id;
        public String sender;
        public String text;
        public long timestampMs;
        public long observedAtMs;
        public boolean timeKnown;
        public boolean outgoing;
        public String deliveryStatus;
        public String via;
        public String clientMsgId;
        public boolean recalled;
        public String mediaB64 = "";

        private transient JSONObject cachedJson;

        JSONObject toJson() throws JSONException {
            if (cachedJson != null) return cachedJson;
            JSONObject msg = new JSONObject();
            msg.put("id", id);
            msg.put("sender", sender);
            msg.put("text", text);
            msg.put("timestampMs", timestampMs);
            msg.put("observedAtMs", observedAtMs);
            msg.put("timeKnown", timeKnown);
            msg.put("outgoing", outgoing);
            msg.put("deliveryStatus", deliveryStatus);
            msg.put("via", via);
            msg.put("recalled", recalled);
            if (clientMsgId != null && !clientMsgId.isEmpty()) {
                msg.put("clientMsgId", clientMsgId);
            }
            String mediaHint = ImAppCatalog.inferMediaHint(text);
            if (!mediaHint.isEmpty()) {
                msg.put("mediaHint", mediaHint);
            }
            if (mediaB64 != null && !mediaB64.isEmpty()) {
                msg.put("mediaB64", mediaB64);
            }
            cachedJson = msg;
            return msg;
        }

        static MessageRecord fromJson(JSONObject m) {
            MessageRecord r = new MessageRecord();
            r.id = m.optString("id", "");
            r.sender = m.optString("sender", "");
            r.text = m.optString("text", "");
            r.timestampMs = m.optLong("timestampMs", m.optLong("timestamp_ms", 0L));
            r.outgoing = m.optBoolean("outgoing", false);
            r.deliveryStatus = m.optString("deliveryStatus", m.optString("delivery_status", "received"));
            r.via = m.optString("via", "notification");
            r.observedAtMs = m.optLong("observedAtMs", r.timestampMs);
            r.timeKnown = ContextProvenance.timeKnown(r.via, m.has("timeKnown"), m.optBoolean("timeKnown", false));
            if (!r.timeKnown) r.timestampMs = 0L;
            r.clientMsgId = m.optString("clientMsgId", m.optString("client_msg_id", ""));
            r.recalled = m.optBoolean("recalled", false);
            r.mediaB64 = m.optString("mediaB64", m.optString("media_b64", ""));
            return r;
        }
    }

    public static final class ThreadRecord {
        public String threadId;
        public String packageName;
        public String appName;
        public String title;
        public boolean isGroup;
        public String lastSender = "";
        public String lastMessage = "";
        public long lastTimeMs;
        public int unreadCount;
        public boolean canReply;
        public boolean hasContentIntent;
        public String lastNotificationKey = "";
        public String avatarB64 = "";
        public final Map<String, String> senderAvatars = new HashMap<String, String>();
        public final List<MessageRecord> messages = new ArrayList<MessageRecord>();

        JSONObject toJson() throws JSONException {
            JSONObject t = new JSONObject();
            t.put("threadId", threadId);
            t.put("packageName", packageName);
            t.put("appName", appName);
            t.put("title", title);
            t.put("isGroup", isGroup);
            t.put("lastSender", lastSender);
            t.put("lastMessage", lastMessage);
            t.put("lastTimestampMs", lastTimeMs);
            t.put("unreadCount", unreadCount);
            t.put("canReply", canReply);
            t.put("canUiReply", GuiAccessibilityService.instance != null || ChatUiBridge.isApexBridgeAvailable());
            t.put("hasContentIntent", hasContentIntent);
            if (lastNotificationKey != null && !lastNotificationKey.isEmpty()) {
                t.put("notificationKey", lastNotificationKey);
            }
            if (avatarB64 != null && !avatarB64.isEmpty()) {
                t.put("avatar_b64", avatarB64);
            }
            if (!senderAvatars.isEmpty()) {
                JSONObject sa = new JSONObject();
                for (Map.Entry<String, String> e : senderAvatars.entrySet()) {
                    if (e.getKey() != null && !e.getKey().isEmpty()
                            && e.getValue() != null && !e.getValue().isEmpty()) {
                        sa.put(e.getKey(), e.getValue());
                    }
                }
                t.put("sender_avatars", sa);
            }
            JSONArray msgArr = new JSONArray();
            for (MessageRecord m : messages) {
                msgArr.put(m.toJson());
            }
            t.put("messages", msgArr);
            return t;
        }

        JSONObject toSnapshotJson(int[] remainingAvatarChars, boolean allowSenderAvatars) throws JSONException {
            JSONObject t = new JSONObject();
            t.put("threadId", threadId);
            t.put("packageName", packageName);
            t.put("appName", appName);
            t.put("title", title);
            t.put("isGroup", isGroup);
            t.put("lastSender", lastSender);
            t.put("lastMessage", lastMessage);
            t.put("lastTimestampMs", lastTimeMs);
            t.put("unreadCount", unreadCount);
            t.put("canReply", canReply);
            t.put("canUiReply", GuiAccessibilityService.instance != null || ChatUiBridge.isApexBridgeAvailable());
            t.put("hasContentIntent", hasContentIntent);
            if (lastNotificationKey != null && !lastNotificationKey.isEmpty()) {
                t.put("notificationKey", lastNotificationKey);
            }
            if (avatarB64 != null && !avatarB64.isEmpty()
                    && remainingAvatarChars != null
                    && remainingAvatarChars[0] >= avatarB64.length()) {
                t.put("avatar_b64", avatarB64);
                remainingAvatarChars[0] -= avatarB64.length();
            }
            if (allowSenderAvatars && !senderAvatars.isEmpty() && remainingAvatarChars != null) {
                JSONObject sa = new JSONObject();
                int count = 0;
                for (Map.Entry<String, String> e : senderAvatars.entrySet()) {
                    if (count >= 8) break;
                    String k = e.getKey();
                    String v = e.getValue();
                    if (k != null && !k.isEmpty() && v != null && !v.isEmpty()) {
                        if (remainingAvatarChars[0] < v.length()) break;
                        sa.put(k, v);
                        remainingAvatarChars[0] -= v.length();
                        count++;
                    }
                }
                if (sa.length() > 0) {
                    t.put("sender_avatars", sa);
                }
            }
            JSONArray msgArr = new JSONArray();
            int startIdx = Math.max(0, messages.size() - 40);
            for (int i = startIdx; i < messages.size(); i++) {
                msgArr.put(messages.get(i).toJson());
            }
            t.put("messages", msgArr);
            return t;
        }

        static ThreadRecord fromJson(JSONObject j) throws JSONException {
            ThreadRecord t = new ThreadRecord();
            t.threadId = j.optString("threadId", j.optString("thread_id", ""));
            t.packageName = j.optString("packageName", j.optString("package_name", ""));
            t.appName = j.optString("appName", j.optString("app_name", ""));
            t.title = j.optString("title", "");
            t.isGroup = j.optBoolean("isGroup", j.optBoolean("is_group", false));
            t.lastSender = j.optString("lastSender", j.optString("last_sender", ""));
            t.lastMessage = j.optString("lastMessage", j.optString("last_message", ""));
            t.lastTimeMs = j.optLong(
                "lastTimestampMs",
                j.optLong("lastTimeMs", j.optLong("last_timestamp_ms", 0L))
            );
            t.unreadCount = j.optInt("unreadCount", j.optInt("unread_count", 0));
            t.canReply = false;
            t.hasContentIntent = j.optBoolean("hasContentIntent", false);
            t.lastNotificationKey = j.optString(
                "notificationKey",
                j.optString("lastNotificationKey", j.optString("notification_key", ""))
            );
            t.avatarB64 = j.optString("avatar_b64", j.optString("avatarB64", ""));
            JSONObject sa = j.optJSONObject("sender_avatars");
            if (sa == null) {
                sa = j.optJSONObject("senderAvatars");
            }
            if (sa != null) {
                java.util.Iterator<String> keys = sa.keys();
                while (keys.hasNext()) {
                    String k = keys.next();
                    String v = sa.optString(k, "");
                    if (k != null && !k.isEmpty() && !v.isEmpty()) {
                        t.senderAvatars.put(k, v);
                    }
                }
            }
            JSONArray arr = j.optJSONArray("messages");
            if (arr != null) {
                for (int i = 0; i < arr.length() && i < MAX_MESSAGES_PER_THREAD; i++) {
                    t.messages.add(MessageRecord.fromJson(arr.getJSONObject(i)));
                }
                boolean containsUnknownTime = false;
                long latestKnown = 0L;
                for (MessageRecord message : t.messages) {
                    containsUnknownTime |= !message.timeKnown;
                    if (message.timeKnown) latestKnown = Math.max(latestKnown, message.timestampMs);
                }
                if (containsUnknownTime) t.lastTimeMs = latestKnown;
            }
            return t;
        }
    }

    public static final class NotificationRecord {
        public String eventId;
        public long seqId;
        public String key;
        public String packageName;
        public String appName;
        public String category;
        public String title;
        public String sender = "";
        public String text;
        public String subText;
        public long postTimeMs;
        public long removedTimeMs;
        public int removeReason;
        public boolean active;
        public boolean ongoing;
        public boolean isGroup;
        public boolean read;
        public boolean dismissed;
        public boolean canReply;
        public boolean hasContentIntent;
        public String otpCode;
        public String firstUrl;
        public String threadId;
        public String avatarB64 = "";
        public int progressCurrent;
        public int progressMax;
        public boolean progressIndeterminate;

        transient JSONObject cachedJson;

        JSONObject toJson() throws JSONException {
            if (cachedJson != null) return cachedJson;
            JSONObject entry = new JSONObject();
            entry.put("seqId", seqId);
            entry.put("key", key);
            entry.put("packageName", packageName);
            entry.put("appName", appName);
            entry.put("category", category);
            entry.put("title", title);
            entry.put("sender", sender == null ? "" : sender);
            entry.put("text", text);
            entry.put("subText", subText);
            entry.put("postTimeMs", postTimeMs);
            if (removedTimeMs > 0) {
                entry.put("removedTimeMs", removedTimeMs);
                entry.put("removeReason", removeReason);
            }
            entry.put("active", active);
            entry.put("ongoing", ongoing);
            entry.put("isGroup", isGroup);
            entry.put("read", read);
            entry.put("dismissed", dismissed);
            entry.put("canReply", canReply);
            entry.put("hasContentIntent", hasContentIntent);
            entry.put("progressCurrent", progressCurrent);
            entry.put("progressMax", progressMax);
            entry.put("progressIndeterminate", progressIndeterminate);
            if (otpCode != null && !otpCode.isEmpty()) {
                entry.put("otpCode", otpCode);
            }
            if (firstUrl != null && !firstUrl.isEmpty()) {
                entry.put("urlHint", firstUrl);
            }
            if (threadId != null && !threadId.isEmpty()) {
                entry.put("threadId", threadId);
            }
            if (avatarB64 != null && !avatarB64.isEmpty()) {
                entry.put("avatar_b64", avatarB64);
            }
            cachedJson = entry;
            return entry;
        }

        JSONObject toJson(boolean includeAvatar) throws JSONException {
            return toSnapshotJson(includeAvatar);
        }

        JSONObject toSnapshotJson(boolean includeAvatar) throws JSONException {
            JSONObject base = toJson();
            if (includeAvatar || avatarB64 == null || avatarB64.isEmpty()) {
                return base;
            }
            JSONObject entry = new JSONObject();
            entry.put("seqId", seqId);
            entry.put("key", key);
            entry.put("packageName", packageName);
            entry.put("appName", appName);
            entry.put("category", category);
            entry.put("title", title);
            entry.put("sender", sender == null ? "" : sender);
            entry.put("text", text);
            entry.put("subText", subText);
            entry.put("postTimeMs", postTimeMs);
            if (removedTimeMs > 0) {
                entry.put("removedTimeMs", removedTimeMs);
                entry.put("removeReason", removeReason);
            }
            entry.put("active", active);
            entry.put("ongoing", ongoing);
            entry.put("isGroup", isGroup);
            entry.put("read", read);
            entry.put("dismissed", dismissed);
            entry.put("canReply", canReply);
            entry.put("hasContentIntent", hasContentIntent);
            entry.put("progressCurrent", progressCurrent);
            entry.put("progressMax", progressMax);
            entry.put("progressIndeterminate", progressIndeterminate);
            if (otpCode != null && !otpCode.isEmpty()) {
                entry.put("otpCode", otpCode);
            }
            if (firstUrl != null && !firstUrl.isEmpty()) {
                entry.put("urlHint", firstUrl);
            }
            if (threadId != null && !threadId.isEmpty()) {
                entry.put("threadId", threadId);
            }
            return entry;
        }

        static NotificationRecord fromJson(JSONObject j) {
            NotificationRecord n = new NotificationRecord();
            n.seqId = j.optLong("seqId", j.optLong("seq_id", 0L));
            n.key = j.optString("key", "");
            n.postTimeMs = j.optLong("postTimeMs", j.optLong("post_time_ms", 0L));
            n.eventId = n.key + ":" + n.postTimeMs;
            n.packageName = j.optString("packageName", j.optString("package_name", ""));
            n.appName = j.optString("appName", j.optString("app_name", ""));
            n.category = j.optString("category", ImAppCatalog.CAT_OTHER);
            n.title = j.optString("title", "");
            n.sender = j.optString("sender", "");
            n.text = j.optString("text", "");
            n.subText = j.optString("subText", j.optString("sub_text", ""));
            n.removedTimeMs = j.optLong("removedTimeMs", 0L);
            n.removeReason = j.optInt("removeReason", 0);
            n.active = j.optBoolean("active", false);
            n.ongoing = j.optBoolean("ongoing", j.optBoolean("isOngoing", false));
            n.isGroup = j.optBoolean("isGroup", false);
            n.read = j.optBoolean("read", false);
            n.dismissed = j.optBoolean("dismissed", false);
            n.canReply = false;
            n.hasContentIntent = j.optBoolean("hasContentIntent", false);
            n.progressCurrent = j.optInt("progressCurrent", j.optInt("progress_current", 0));
            n.progressMax = j.optInt("progressMax", j.optInt("progress_max", 0));
            n.progressIndeterminate = j.optBoolean(
                "progressIndeterminate",
                j.optBoolean("progress_indeterminate", false)
            );
            n.otpCode = j.optString("otpCode", null);
            n.firstUrl = j.optString("urlHint", j.optString("firstUrl", null));
            n.threadId = j.optString("threadId", null);
            n.avatarB64 = j.optString("avatar_b64", j.optString("avatarB64", ""));
            return n;
        }
    }

    public static final class ReplyOutcome {
        public final String threadId;
        public final String via;
        public final String clientMsgId;
        public final boolean deduplicated;
        public final long revision;
        public final long timestampMs;
        public final ThreadRecord thread;

        ReplyOutcome(
                String threadId,
                String via,
                String clientMsgId,
                boolean deduplicated,
                long revision,
                long timestampMs,
                ThreadRecord thread) {
            this.threadId = threadId;
            this.via = via;
            this.clientMsgId = clientMsgId;
            this.deduplicated = deduplicated;
            this.revision = revision;
            this.timestampMs = timestampMs;
            this.thread = thread;
        }
    }

    interface RemoteReplyDispatcher {
        void send(Context context, RemoteReplyHandle handle, String text) throws PendingIntent.CanceledException;
    }

    private static final RemoteReplyDispatcher DEFAULT_DISPATCHER = new RemoteReplyDispatcher() {
        @Override
        public void send(Context context, RemoteReplyHandle handle, String text)
                throws PendingIntent.CanceledException {
            if (handle == null || handle.actionIntent == null
                    || handle.remoteInputs == null || handle.remoteInputs.length == 0) {
                throw new PendingIntent.CanceledException("Missing RemoteInput or PendingIntent");
            }
            Intent fillInIntent = new Intent();
            fillInIntent.addFlags(Intent.FLAG_RECEIVER_FOREGROUND);
            Bundle results = new Bundle();
            for (RemoteInput input : handle.remoteInputs) {
                if (input != null && input.getResultKey() != null) {
                    results.putCharSequence(input.getResultKey(), text);
                }
            }
            RemoteInput.addResultsToIntent(handle.remoteInputs, fillInIntent, results);
            try {
                RemoteInput.setResultsSource(fillInIntent, RemoteInput.SOURCE_FREE_FORM_INPUT);
            } catch (Throwable ignored) {
            }
            Bundle opts = buildBackgroundActivityLaunchOptions();
            if (opts != null) {
                try {
                    handle.actionIntent.send(context, 0, fillInIntent, null, null, null, opts);
                    return;
                } catch (PendingIntent.CanceledException canceled) {
                    throw canceled;
                } catch (Throwable ignored) {
                }
            }
            handle.actionIntent.send(context, 0, fillInIntent);
        }
    };

    private static final class ThrottleState {
        final long lastPostedMs;
        final String signature;

        ThrottleState(long lastPostedMs, String signature) {
            this.lastPostedMs = lastPostedMs;
            this.signature = signature;
        }
    }

    private final File storageDir;
    private PushDatabase database;
    private boolean storageLoadFailed = false;
    private final Object diskCommitLock = new Object();
    private RemoteReplyDispatcher replyDispatcher = DEFAULT_DISPATCHER;
    private long revision = 1L;
    private long nextSeqId = 1L;
    private long observedSinceMs = System.currentTimeMillis();
    private long lastObservedMs = 0L;
    private long lastGapMs = 0L;
    private boolean historyTruncated = false;
    private boolean persistenceError = false;
    private boolean threadsDirty = false;
    private boolean feedDirty = false;
    private int batchDepth = 0;
    private ScheduledFuture<?> pendingSaveFuture = null;
    private static final ScheduledExecutorService DISK_WRITER = Executors.newSingleThreadScheduledExecutor(
        new ThreadFactory() {
            @Override
            public Thread newThread(Runnable r) {
                Thread t = new Thread(r, "StepXPushDiskWriter");
                t.setDaemon(true);
                return t;
            }
        }
    );
    private long cachedSnapshotRevision = -1;
    private String cachedSnapshotJson = null;
    private JSONObject cachedSnapshotObj = null;

    private final Map<String, ThreadRecord> threadsById = new LinkedHashMap<String, ThreadRecord>();
    private final List<NotificationRecord> notifications = new ArrayList<NotificationRecord>();

    // Soft-retained LRU caches for RemoteReplyHandle
    private final Map<String, RemoteReplyHandle> handlesByThread = new LinkedHashMap<String, RemoteReplyHandle>(32, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, RemoteReplyHandle> eldest) {
            return size() > MAX_HANDLES;
        }
    };
    private final Map<String, RemoteReplyHandle> handlesByKey = new LinkedHashMap<String, RemoteReplyHandle>(32, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, RemoteReplyHandle> eldest) {
            return size() > MAX_HANDLES;
        }
    };
    private final Map<String, PendingIntent> contentIntentsByKey = new LinkedHashMap<String, PendingIntent>(64, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, PendingIntent> eldest) {
            return size() > MAX_HANDLES * 2;
        }
    };
    private final Map<String, PendingIntent> contentIntentsByThread = new LinkedHashMap<String, PendingIntent>(64, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, PendingIntent> eldest) {
            return size() > MAX_HANDLES * 2;
        }
    };

    // Bounded 64-entry idempotency map: clientMsgId -> ReplyOutcome
    private final Map<String, ReplyOutcome> recentClientMsgIds = new LinkedHashMap<String, ReplyOutcome>(64, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, ReplyOutcome> eldest) {
            return size() > MAX_CLIENT_MSG_IDS;
        }
    };

    private final Map<String, ThrottleState> ongoingThrottle = new HashMap<String, ThrottleState>();
    private final Map<String, String> resolvedGenericThreadAliases = new HashMap<String, String>();
    private final Map<String, Long> deletedThreads = new LinkedHashMap<String, Long>();
    private final Map<String, Long> clearedThreads = new LinkedHashMap<String, Long>();
    private final java.util.Set<String> deletedMessages = new java.util.LinkedHashSet<String>();

    PushStore(File storageDir) {
        this.storageDir = storageDir;
        loadFromDisk();
        if (!threadsById.isEmpty() || !notifications.isEmpty()) {
            lastGapMs = System.currentTimeMillis();
        }
    }

    private static String canonicalThreadKey(String threadId, String packageName, String rawTitle) {
        String pkg = packageName == null ? "" : packageName.trim();
        String title = rawTitle == null ? "" : rawTitle.trim();
        if ((pkg.isEmpty() || title.isEmpty()) && threadId != null) {
            int sep = threadId.indexOf("::");
            if (sep > 0 && sep + 2 < threadId.length()) {
                if (pkg.isEmpty()) pkg = threadId.substring(0, sep).trim();
                if (title.isEmpty()) title = threadId.substring(sep + 2).trim();
            }
        }
        if (!pkg.isEmpty() && !title.isEmpty()) {
            String clean = ImAppCatalog.cleanConversationTitle(pkg, title);
            if (!clean.isEmpty()) {
                return ImAppCatalog.buildThreadId(pkg, clean);
            }
            return ImAppCatalog.buildThreadId(pkg, title);
        }
        return threadId == null ? "" : threadId.trim();
    }

    private long getDeletedAtMs(String threadId, String packageName, String title) {
        long ts = 0L;
        if (threadId != null && deletedThreads.containsKey(threadId)) {
            ts = Math.max(ts, deletedThreads.get(threadId));
        }
        String canonical = canonicalThreadKey(threadId, packageName, title);
        if (!canonical.isEmpty() && deletedThreads.containsKey(canonical)) {
            ts = Math.max(ts, deletedThreads.get(canonical));
        }
        return ts;
    }

    private long getClearedAtMs(String threadId, String packageName, String title) {
        long ts = 0L;
        if (threadId != null && clearedThreads.containsKey(threadId)) {
            ts = Math.max(ts, clearedThreads.get(threadId));
        }
        String canonical = canonicalThreadKey(threadId, packageName, title);
        if (!canonical.isEmpty() && clearedThreads.containsKey(canonical)) {
            ts = Math.max(ts, clearedThreads.get(canonical));
        }
        return ts;
    }

    private boolean isMessageDeleted(String threadId, String msgId, String text) {
        if (msgId != null && !msgId.isEmpty() && deletedMessages.contains(msgId)) {
            return true;
        }
        if (threadId != null && !threadId.isEmpty() && text != null && !text.trim().isEmpty()) {
            return deletedMessages.contains(threadId + "::" + text.trim());
        }
        return false;
    }

    synchronized void setReplyDispatcherForTest(RemoteReplyDispatcher dispatcher) {
        this.replyDispatcher = dispatcher == null ? DEFAULT_DISPATCHER : dispatcher;
    }

    public synchronized long getRevision() {
        return revision;
    }

    synchronized void callStateChanged() {
        revision++;
    }

    public synchronized void recordListenerGap() {
        lastGapMs = System.currentTimeMillis();
        revision++;
        feedDirty = true;
        saveToDisk();
    }

    private JSONObject coverageJson() throws JSONException {
        return new JSONObject().put("observedSinceMs", observedSinceMs)
            .put("lastObservedMs", lastObservedMs).put("lastGapMs", lastGapMs)
            .put("historyTruncated", historyTruncated).put("persistenceError", persistenceError)
            .put("scope", "posted_notifications_and_explicit_visible_captures");
    }

    private void loadCoverage(JSONObject root) {
        JSONObject coverage = root.optJSONObject("coverage");
        if (coverage == null) {
            // Pre-upgrade storage does not establish continuous capture coverage.
            historyTruncated = true;
            return;
        }
        long first = coverage.optLong("observedSinceMs", 0L);
        if (first > 0L) observedSinceMs = Math.min(observedSinceMs, first);
        lastObservedMs = Math.max(lastObservedMs, coverage.optLong("lastObservedMs", 0L));
        lastGapMs = Math.max(lastGapMs, coverage.optLong("lastGapMs", 0L));
        historyTruncated |= coverage.optBoolean("historyTruncated", false);
    }

    public synchronized int getThreadCount() {
        return threadsById.size();
    }

    public synchronized int getNotificationCount() {
        return notifications.size();
    }

    public synchronized void beginBatch() {
        batchDepth++;
    }

    public synchronized void endBatch() {
        if (batchDepth > 0) {
            batchDepth--;
            if (batchDepth == 0 && (threadsDirty || feedDirty)) {
                saveToDisk();
            }
        }
    }

    public synchronized JSONObject statusSummary() throws Exception {
        int unreadImCount = 0;
        for (ThreadRecord t : threadsById.values()) {
            unreadImCount += t.unreadCount;
        }
        int activePushCount = 0;
        for (NotificationRecord n : notifications) {
            if (n.active && !n.ongoing) {
                activePushCount++;
            }
        }
        return new JSONObject()
            .put("revision", revision)
            .put("threadCount", threadsById.size())
            .put("unreadImCount", unreadImCount)
            .put("activePushCount", activePushCount)
            .put("totalPushCount", notifications.size());
    }

    public synchronized ThreadRecord getThreadRecord(String threadId) {
        if (threadId == null || threadId.isEmpty()) return null;
        String key = threadId.trim();
        ThreadRecord direct = threadsById.get(key);
        if (direct != null) return direct;
        String alias = resolvedGenericThreadAliases.get(key);
        if (alias != null) {
            ThreadRecord aliased = threadsById.get(alias);
            if (aliased != null) return aliased;
        }
        int sep = key.indexOf("::");
        if (sep > 0 && sep + 2 < key.length()) {
            String pkg = key.substring(0, sep).trim();
            String rawTitle = key.substring(sep + 2).trim();
            String clean = ImAppCatalog.cleanConversationTitle(pkg, rawTitle);
            if (!clean.isEmpty()) {
                String canonicalId = ImAppCatalog.buildThreadId(pkg, clean);
                ThreadRecord canonical = threadsById.get(canonicalId);
                if (canonical != null) return canonical;
                String canonicalAlias = resolvedGenericThreadAliases.get(canonicalId);
                if (canonicalAlias != null) {
                    ThreadRecord ca = threadsById.get(canonicalAlias);
                    if (ca != null) return ca;
                }
            }
            for (ThreadRecord t : threadsById.values()) {
                if (!pkg.equals(t.packageName)) continue;
                if (rawTitle.equals(t.title)
                        || (!clean.isEmpty() && clean.equals(ImAppCatalog.cleanConversationTitle(pkg, t.title)))
                        || titlesReferToSameConversation(pkg, t.title, rawTitle)) {
                    return t;
                }
            }
            if (ImAppCatalog.isGenericPlaceholderTitle(rawTitle) || ImAppCatalog.isGenericPlaceholderTitle(clean)) {
                List<String> hints = ImAppCatalog.extractGenericGroupParticipantHints(rawTitle);
                if (!hints.isEmpty()) {
                    for (ThreadRecord t : threadsById.values()) {
                        if (!pkg.equals(t.packageName)) continue;
                        if (threadMatchesParticipantHints(t, hints)) {
                            return t;
                        }
                    }
                }
                String genericGroupId = ImAppCatalog.buildThreadId(pkg, "群聊");
                ThreadRecord genericGroup = threadsById.get(genericGroupId);
                if (genericGroup != null) return genericGroup;
                String genericGroupAlias = resolvedGenericThreadAliases.get(genericGroupId);
                if (genericGroupAlias != null) {
                    ThreadRecord ga = threadsById.get(genericGroupAlias);
                    if (ga != null) return ga;
                }
                for (ThreadRecord t : threadsById.values()) {
                    if (pkg.equals(t.packageName) && ImAppCatalog.isGenericPlaceholderTitle(t.title)) {
                        return t;
                    }
                }
            }
        }
        for (ThreadRecord t : threadsById.values()) {
            if (key.equals(t.lastNotificationKey)) {
                return t;
            }
        }
        return null;
    }

    private static boolean threadMatchesParticipantHints(ThreadRecord t, List<String> hints) {
        if (t == null || hints == null || hints.isEmpty()) return false;
        List<String> tHints = ImAppCatalog.extractGenericGroupParticipantHints(t.title);
        for (String h : hints) {
            if (h == null || h.isEmpty()) continue;
            if (tHints.contains(h)) return true;
            if (t.title != null && t.title.contains(h)) return true;
            if (t.lastSender != null && h.equals(t.lastSender.trim())) return true;
            if (t.messages != null) {
                for (MessageRecord m : t.messages) {
                    if (m != null && m.sender != null && h.equals(m.sender.trim())) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    /**
     * Returns recent message texts (most recent first) for the given package and title/threadId,
     * used by ChatUiBridge to verify or locate conversations whose notification title was generic ("群聊").
     */
    public synchronized List<String> getRecentMessageTextsForThread(String packageName, String rawTitleOrThreadId) {
        List<String> out = new ArrayList<String>();
        String safePkg = packageName == null || packageName.isEmpty() ? "com.tencent.mm" : packageName.trim();
        String key = rawTitleOrThreadId == null ? "" : rawTitleOrThreadId.trim();
        ThreadRecord thread = null;
        if (key.contains("::")) {
            thread = getThreadRecord(key);
        }
        if (thread == null && !key.isEmpty()) {
            thread = getThreadRecord(ImAppCatalog.buildThreadId(safePkg, key));
            if (thread == null) {
                String clean = ImAppCatalog.cleanConversationTitle(safePkg, key);
                if (!clean.isEmpty()) {
                    thread = getThreadRecord(ImAppCatalog.buildThreadId(safePkg, clean));
                }
            }
        }
        if (thread == null && ImAppCatalog.isGenericPlaceholderTitle(key)) {
            thread = getThreadRecord(ImAppCatalog.buildThreadId(safePkg, "群聊"));
            if (thread == null) {
                for (ThreadRecord t : threadsById.values()) {
                    if (safePkg.equals(t.packageName) && (t.isGroup || ImAppCatalog.isGenericPlaceholderTitle(t.title))) {
                        thread = t;
                        break;
                    }
                }
            }
        }
        if (thread != null && thread.messages != null) {
            for (int i = thread.messages.size() - 1; i >= 0 && out.size() < 20; i--) {
                MessageRecord m = thread.messages.get(i);
                if (m != null && m.text != null) {
                    String txt = m.text.trim();
                    if (!txt.isEmpty() && !out.contains(txt)) {
                        out.add(txt);
                    }
                }
            }
        }
        return out;
    }

    /**
     * Returns known participant/sender nicknames for the given package and title/threadId,
     * combining enriched generic group title hints and non-generic incoming message senders.
     */
    public synchronized List<String> getParticipantHintsForThread(String packageName, String rawTitleOrThreadId) {
        List<String> out = new ArrayList<String>();
        String safePkg = packageName == null || packageName.isEmpty() ? "com.tencent.mm" : packageName.trim();
        String key = rawTitleOrThreadId == null ? "" : rawTitleOrThreadId.trim();
        for (String h : ImAppCatalog.extractGenericGroupParticipantHints(key)) {
            if (!out.contains(h)) out.add(h);
        }
        ThreadRecord thread = key.contains("::")
            ? getThreadRecord(key)
            : getThreadRecord(ImAppCatalog.buildThreadId(safePkg, key));
        if (thread == null && ImAppCatalog.isGenericPlaceholderTitle(key)) {
            thread = getThreadRecord(ImAppCatalog.buildThreadId(safePkg, "群聊"));
        }
        if (thread != null) {
            for (String h : ImAppCatalog.extractGenericGroupParticipantHints(thread.title)) {
                if (!out.contains(h)) out.add(h);
            }
            if (thread.lastSender != null) {
                String ls = thread.lastSender.trim();
                if (!ls.isEmpty() && !"我".equals(ls)
                        && !ImAppCatalog.isGenericSenderPlaceholder(ls)
                        && !ImAppCatalog.isGenericPlaceholderTitle(ls)
                        && !out.contains(ls)) {
                    out.add(ls);
                }
            }
            if (thread.messages != null) {
                for (int i = thread.messages.size() - 1; i >= 0 && out.size() < 12; i--) {
                    MessageRecord m = thread.messages.get(i);
                    if (m != null && !m.outgoing && m.sender != null) {
                        String s = m.sender.trim();
                        if (!s.isEmpty() && !"我".equals(s)
                                && !ImAppCatalog.isGenericSenderPlaceholder(s)
                                && !ImAppCatalog.isGenericPlaceholderTitle(s)
                                && !out.contains(s)) {
                            out.add(s);
                        }
                    }
                }
            }
        }
        return out;
    }

    public synchronized NotificationRecord getNotificationRecordByKey(String key) {
        for (NotificationRecord n : notifications) {
            if (key != null && key.equals(n.key)) return n;
        }
        return null;
    }

    public synchronized boolean hasReplyHandle(String threadIdOrKey) {
        return getReplyHandle(threadIdOrKey) != null;
    }

    public synchronized RemoteReplyHandle getReplyHandle(String threadIdOrKey) {
        if (threadIdOrKey == null || threadIdOrKey.isEmpty()) return null;
        String key = threadIdOrKey.trim();
        RemoteReplyHandle h = handlesByThread.get(key);
        if (h != null) return h;
        h = handlesByKey.get(key);
        if (h != null) return h;
        String alias = resolvedGenericThreadAliases.get(key);
        if (alias != null) {
            h = handlesByThread.get(alias);
            if (h != null) return h;
        }
        for (Map.Entry<String, String> entry : resolvedGenericThreadAliases.entrySet()) {
            if (key.equals(entry.getValue())) {
                h = handlesByThread.get(entry.getKey());
                if (h != null) return h;
            }
        }
        ThreadRecord thread = getThreadRecord(key);
        if (thread != null) {
            h = handlesByThread.get(thread.threadId);
            if (h != null) return h;
            if (thread.lastNotificationKey != null && !thread.lastNotificationKey.isEmpty()) {
                RemoteReplyHandle byKey = handlesByKey.get(thread.lastNotificationKey);
                if (byKey != null && (thread.threadId.equals(byKey.threadId)
                        || titlesReferToSameConversation(thread.packageName, thread.title, byKey.title))) {
                    return byKey;
                }
            }
        }
        NotificationRecord notif = getNotificationRecordByKey(key);
        if (notif != null && notif.threadId != null && !notif.threadId.isEmpty()) {
            h = handlesByThread.get(notif.threadId);
            if (h != null) return h;
        }
        int sep = key.indexOf("::");
        if (sep > 0 && sep + 2 < key.length()) {
            String pkg = key.substring(0, sep).trim();
            String rawTitle = key.substring(sep + 2).trim();
            String clean = ImAppCatalog.cleanConversationTitle(pkg, rawTitle);
            if (!clean.isEmpty()) {
                h = handlesByThread.get(ImAppCatalog.buildThreadId(pkg, clean));
                if (h != null) return h;
                for (RemoteReplyHandle candidate : handlesByThread.values()) {
                    if (candidate != null && pkg.equals(candidate.packageName)) {
                        String candClean = ImAppCatalog.cleanConversationTitle(pkg, candidate.title);
                        if (clean.equals(candClean) || rawTitle.equals(candidate.title)) {
                            return candidate;
                        }
                    }
                }
            }
            if (ImAppCatalog.isGenericPlaceholderTitle(rawTitle) || ImAppCatalog.isGenericPlaceholderTitle(clean)) {
                String genericGroupId = ImAppCatalog.buildThreadId(pkg, "群聊");
                h = handlesByThread.get(genericGroupId);
                if (h != null) return h;
                List<String> hints = ImAppCatalog.extractGenericGroupParticipantHints(rawTitle);
                for (RemoteReplyHandle candidate : handlesByKey.values()) {
                    if (candidate != null && pkg.equals(candidate.packageName)) {
                        if (genericGroupId.equals(candidate.threadId)
                                || ImAppCatalog.isGenericPlaceholderTitle(candidate.title)) {
                            return candidate;
                        }
                        if (!hints.isEmpty()) {
                            List<String> candHints = ImAppCatalog.extractGenericGroupParticipantHints(candidate.title);
                            for (String hint : hints) {
                                if (candHints.contains(hint)
                                        || (candidate.title != null && candidate.title.contains(hint))) {
                                    return candidate;
                                }
                            }
                        }
                    }
                }
                for (RemoteReplyHandle candidate : handlesByThread.values()) {
                    if (candidate != null && pkg.equals(candidate.packageName)) {
                        if (genericGroupId.equals(candidate.threadId)
                                || ImAppCatalog.isGenericPlaceholderTitle(candidate.title)) {
                            return candidate;
                        }
                        if (!hints.isEmpty()) {
                            List<String> candHints = ImAppCatalog.extractGenericGroupParticipantHints(candidate.title);
                            for (String hint : hints) {
                                if (candHints.contains(hint)
                                        || (candidate.title != null && candidate.title.contains(hint))) {
                                    return candidate;
                                }
                            }
                        }
                    }
                }
            }
        }
        return null;
    }

    public static String normalizeConversationTitle(String pkg, String rawTitle) {
        if (rawTitle == null) return "";
        String s = ImAppCatalog.cleanConversationTitle(pkg, rawTitle).trim();
        while (s.endsWith("…")) {
            s = s.substring(0, s.length() - 1).trim();
        }
        while (s.endsWith("...")) {
            s = s.substring(0, s.length() - 3).trim();
        }
        int atIdx = s.indexOf('@');
        int fullAtIdx = s.indexOf('＠');
        int splitIdx = -1;
        if (atIdx >= 0 && fullAtIdx >= 0) {
            splitIdx = Math.min(atIdx, fullAtIdx);
        } else if (atIdx >= 0) {
            splitIdx = atIdx;
        } else if (fullAtIdx >= 0) {
            splitIdx = fullAtIdx;
        }
        if (splitIdx > 0) {
            String before = s.substring(0, splitIdx);
            String prefix = before.trim();
            String after = s.substring(splitIdx + 1).trim();
            boolean hasSpaceBefore = before.endsWith(" ") || before.endsWith("\u3000") || prefix.contains(" ");
            boolean afterHasNonAscii = false;
            for (int i = 0; i < after.length(); i++) {
                if (after.charAt(i) > 127) {
                    afterHasNonAscii = true;
                    break;
                }
            }
            if (prefix.length() >= 2 && (hasSpaceBefore || afterHasNonAscii)) {
                s = prefix;
            }
        }
        return s.trim();
    }

    private static boolean hasTrailingEllipsis(String title) {
        if (title == null) return false;
        String t = title.trim();
        return t.endsWith("…") || t.endsWith("...");
    }

    public static boolean titlesReferToSameConversation(String pkg, String existingTitle, String incomingTitle) {
        if (existingTitle == null || incomingTitle == null) return false;
        String cleanA = ImAppCatalog.cleanConversationTitle(pkg, existingTitle).trim();
        String cleanB = ImAppCatalog.cleanConversationTitle(pkg, incomingTitle).trim();
        if (cleanA.isEmpty() || cleanB.isEmpty()) return false;
        if (cleanA.equalsIgnoreCase(cleanB)) return true;
        if (isServiceNotificationContainerTitle(cleanA) || isServiceNotificationContainerTitle(cleanB)) {
            return cleanA.equalsIgnoreCase(cleanB);
        }
        if ((cleanA.startsWith("群聊") && cleanB.startsWith("群聊"))) {
            return true;
        }
        String normA = normalizeConversationTitle(pkg, existingTitle);
        String normB = normalizeConversationTitle(pkg, incomingTitle);
        if (!normA.isEmpty() && normA.equalsIgnoreCase(normB)) {
            return true;
        }
        if (normA.length() >= 3 && normB.length() >= 3) {
            String lowerA = normA.toLowerCase();
            String lowerB = normB.toLowerCase();
            if (hasTrailingEllipsis(existingTitle) && lowerB.startsWith(lowerA)) {
                return true;
            }
            if (hasTrailingEllipsis(incomingTitle) && lowerA.startsWith(lowerB)) {
                return true;
            }
        }
        return false;
    }

    public static String preferCleanerConversationTitle(String pkg, String titleA, String titleB) {
        String cleanA = ImAppCatalog.cleanConversationTitle(pkg, titleA == null ? "" : titleA).trim();
        String cleanB = ImAppCatalog.cleanConversationTitle(pkg, titleB == null ? "" : titleB).trim();
        if (cleanA.isEmpty()) return cleanB;
        if (cleanB.isEmpty()) return cleanA;
        if (ImAppCatalog.isGenericPlaceholderTitle(cleanA) && !ImAppCatalog.isGenericPlaceholderTitle(cleanB)) {
            return cleanB;
        }
        if (ImAppCatalog.isGenericPlaceholderTitle(cleanB) && !ImAppCatalog.isGenericPlaceholderTitle(cleanA)) {
            return cleanA;
        }
        boolean aHasAt = cleanA.indexOf('@') >= 0 || cleanA.indexOf('＠') >= 0;
        boolean bHasAt = cleanB.indexOf('@') >= 0 || cleanB.indexOf('＠') >= 0;
        boolean aTrunc = hasTrailingEllipsis(cleanA);
        boolean bTrunc = hasTrailingEllipsis(cleanB);
        if (!aHasAt && !aTrunc && (bHasAt || bTrunc)) return cleanA;
        if (!bHasAt && !bTrunc && (aHasAt || aTrunc)) return cleanB;
        if (!aTrunc && bTrunc) return cleanA;
        if (!bTrunc && aTrunc) return cleanB;
        return cleanA;
    }

    public static boolean isServiceNotificationContainerTitle(String title) {
        if (title == null) return false;
        String s = title.trim();
        return "服务通知".equals(s)
            || "公众号".equals(s)
            || "订阅号".equals(s)
            || "订阅号消息".equals(s)
            || "腾讯新闻".equals(s)
            || "微信运动".equals(s);
    }

    /**
     * Core POJO ingestion logic. Returns the created NotificationRecord, or null if throttled.
     */
    public synchronized NotificationRecord ingestRecord(
            String sbnKey,
            String packageName,
            String appName,
            String categoryHint,
            String title,
            String conversationTitle,
            String text,
            String subText,
            long postTimeMs,
            boolean ongoing,
            boolean isGroup,
            boolean hasProgress,
            List<StyleMessage> styleMessages,
            RemoteReplyHandle replyHandle,
            PendingIntent contentIntent) {
        return ingestRecord(
            sbnKey, packageName, appName, categoryHint, title, conversationTitle,
            text, subText, postTimeMs, ongoing, isGroup, hasProgress,
            styleMessages, replyHandle, contentIntent, ""
        );
    }

    public synchronized NotificationRecord ingestRecord(
            String sbnKey,
            String packageName,
            String appName,
            String categoryHint,
            String title,
            String conversationTitle,
            String text,
            String subText,
            long postTimeMs,
            boolean ongoing,
            boolean isGroup,
            boolean hasProgress,
            List<StyleMessage> styleMessages,
            RemoteReplyHandle replyHandle,
            PendingIntent contentIntent,
            String avatarB64) {
        return ingestRecord(
            sbnKey, packageName, appName, categoryHint, title, conversationTitle,
            text, subText, postTimeMs, ongoing, isGroup, hasProgress,
            styleMessages, replyHandle, contentIntent, avatarB64, 0, 0, false
        );
    }

    public synchronized NotificationRecord ingestRecord(
            String sbnKey,
            String packageName,
            String appName,
            String categoryHint,
            String title,
            String conversationTitle,
            String text,
            String subText,
            long postTimeMs,
            boolean ongoing,
            boolean isGroup,
            boolean hasProgress,
            List<StyleMessage> styleMessages,
            RemoteReplyHandle replyHandle,
            PendingIntent contentIntent,
            String avatarB64,
            int progressCurrent,
            int progressMax,
            boolean progressIndeterminate) {
        lastObservedMs = System.currentTimeMillis();
        String safeKey = sbnKey == null ? "" : sbnKey;
        String safePkg = packageName == null ? "" : packageName;
        String safeAppName = (appName == null || appName.isEmpty())
            ? ImAppCatalog.appDisplayName(null, safePkg)
            : appName;
        String effectiveRawTitle = (conversationTitle != null && !conversationTitle.trim().isEmpty())
            ? conversationTitle.trim()
            : (title == null ? "" : title.trim());
        String safeText = text == null ? "" : text.trim();
        String safeSubText = subText == null ? "" : subText.trim();
        String safeAvatarB64 = avatarB64 == null ? "" : avatarB64.trim();
        long nowMs = postTimeMs > 0 ? postTimeMs : System.currentTimeMillis();

        // Throttle ongoing or progress-bar notifications within a 2-second window
        if ((ongoing || hasProgress) && !safeKey.isEmpty()) {
            String signature = effectiveRawTitle + "\n" + safeText + "\n" + safeSubText;
            ThrottleState prev = ongoingThrottle.get(safeKey);
            if (prev != null && (nowMs - prev.lastPostedMs) >= 0
                    && (nowMs - prev.lastPostedMs) < ONGOING_THROTTLE_WINDOW_MS) {
                if (signature.equals(prev.signature) || hasProgress) {
                    return null;
                }
            }
            ongoingThrottle.put(safeKey, new ThrottleState(nowMs, signature));
        }

        if (contentIntent != null && !safeKey.isEmpty()) {
            contentIntentsByKey.put(safeKey, contentIntent);
        }

        boolean canReply = replyHandle != null;
        String category = ImAppCatalog.classifyNotification(
            safePkg, categoryHint, effectiveRawTitle, safeText, canReply
        );
        String otpCode = ImAppCatalog.extractOtpCode(effectiveRawTitle, safeText);
        String firstUrl = ImAppCatalog.extractFirstUrl(safeText);

        boolean isNoise = ImAppCatalog.isSystemNoiseNotification(safePkg, effectiveRawTitle, safeText, ongoing);
        boolean shouldCreateImThread = !isNoise
            && !ImAppCatalog.isNativeSmsPackage(safePkg)
            && (
                ImAppCatalog.CAT_IM.equals(category)
                || ImAppCatalog.CAT_SOCIAL.equals(category)
                || ImAppCatalog.CAT_DELIVERY_TRAVEL.equals(category)
                || ImAppCatalog.CAT_FINANCE.equals(category)
                || ImAppCatalog.isMainstreamMessagePackage(safePkg)
                || canReply
                || "msg".equalsIgnoreCase(categoryHint == null ? "" : categoryHint)
                || (styleMessages != null && !styleMessages.isEmpty())
            );

        String threadId = null;
        String detectedSender = "";
        if (shouldCreateImThread && (!effectiveRawTitle.isEmpty() || !safeText.isEmpty())) {
            String fallbackTitle = effectiveRawTitle.isEmpty() ? safeAppName : effectiveRawTitle;
            threadId = ImAppCatalog.buildThreadId(safePkg, fallbackTitle);
            if (ImAppCatalog.isGenericPlaceholderTitle(fallbackTitle)) {
                if (!safeKey.isEmpty()) {
                    for (ThreadRecord existingThread : threadsById.values()) {
                        if (safePkg.equals(existingThread.packageName)
                                && safeKey.equals(existingThread.lastNotificationKey)
                                && !ImAppCatalog.isGenericPlaceholderTitle(existingThread.title)
                                && titlesReferToSameConversation(safePkg, existingThread.title, fallbackTitle)) {
                            fallbackTitle = existingThread.title;
                            threadId = existingThread.threadId;
                            break;
                        }
                    }
                }
                if (resolvedGenericThreadAliases.containsKey(threadId)) {
                    String mappedId = resolvedGenericThreadAliases.get(threadId);
                    ThreadRecord mappedThread = threadsById.get(mappedId);
                    if (mappedThread != null) {
                        threadId = mappedId;
                        fallbackTitle = mappedThread.title;
                    }
                }
            }
            PendingIntent effectiveContentIntent = contentIntent != null
                ? contentIntent
                : (replyHandle != null ? replyHandle.contentIntent : null);
            if (effectiveContentIntent != null && threadId != null && !threadId.isEmpty()) {
                contentIntentsByThread.put(threadId, effectiveContentIntent);
            }
            if (replyHandle != null) {
                RemoteReplyHandle bound = new RemoteReplyHandle(
                    replyHandle.actionIntent,
                    replyHandle.remoteInputs,
                    effectiveContentIntent,
                    safeKey,
                    safePkg,
                    ImAppCatalog.cleanConversationTitle(safePkg, fallbackTitle),
                    threadId,
                    nowMs
                );
                handlesByThread.put(threadId, bound);
                if (!safeKey.isEmpty()) {
                    handlesByKey.put(safeKey, bound);
                }
            }
            upsertThreadFromNotification(
                threadId,
                safeKey,
                safePkg,
                safeAppName,
                fallbackTitle,
                safeText,
                nowMs,
                isGroup,
                effectiveContentIntent != null,
                styleMessages,
                safeAvatarB64
            );
            ThreadRecord tr = threadsById.get(threadId);
            if (tr != null) {
                detectedSender = tr.lastSender;
                if (!ImAppCatalog.isGenericPlaceholderTitle(tr.title) || tr.title.startsWith("群聊")) {
                    effectiveRawTitle = tr.title;
                }
                if (safeAvatarB64.isEmpty() && tr.avatarB64 != null && !tr.avatarB64.isEmpty()) {
                    safeAvatarB64 = tr.avatarB64;
                }
            }
        } else if (replyHandle != null && !safeKey.isEmpty()) {
            handlesByKey.put(safeKey, replyHandle);
        }

        boolean threadCanReply = (threadId != null && handlesByThread.containsKey(threadId))
            || (!safeKey.isEmpty() && handlesByKey.containsKey(safeKey));

        boolean threadWasDeletedAndSuppressed = false;
        if (shouldCreateImThread && (!effectiveRawTitle.isEmpty() || !safeText.isEmpty())) {
            long preDeletedAt = getDeletedAtMs(threadId, safePkg, effectiveRawTitle);
            long maxIncomingTs = nowMs;
            if (styleMessages != null) {
                for (StyleMessage sm : styleMessages) {
                    if (sm != null && sm.timestampMs > maxIncomingTs) {
                        maxIncomingTs = sm.timestampMs;
                    }
                }
            }
            if (preDeletedAt > 0L && maxIncomingTs <= preDeletedAt && !threadsById.containsKey(threadId)) {
                threadWasDeletedAndSuppressed = true;
            }
        }

        long seqId = nextSeqId++;
        revision++;

        NotificationRecord entry = new NotificationRecord();
        entry.eventId = safeKey + ":" + nowMs;
        entry.seqId = seqId;
        entry.key = safeKey;
        entry.packageName = safePkg;
        entry.appName = safeAppName;
        entry.category = category;
        String cleanedTitle = ImAppCatalog.cleanConversationTitle(safePkg, effectiveRawTitle);
        entry.title = cleanedTitle.isEmpty() ? effectiveRawTitle : cleanedTitle;
        if (effectiveRawTitle.startsWith("群聊 · ") || effectiveRawTitle.startsWith("群聊(") || effectiveRawTitle.startsWith("群聊（")) {
            entry.title = effectiveRawTitle;
        }
        entry.sender = detectedSender;
        entry.text = safeText;
        if (!detectedSender.isEmpty() && !ImAppCatalog.isGenericSenderPlaceholder(detectedSender)
                && (safeText.startsWith("微信用户: ") || safeText.startsWith("微信用户："))) {
            int colon = safeText.indexOf(':');
            if (colon < 0) colon = safeText.indexOf('：');
            if (colon >= 0 && colon + 1 < safeText.length()) {
                entry.text = detectedSender + ": " + safeText.substring(colon + 1).trim();
            }
        }
        entry.subText = safeSubText;
        entry.postTimeMs = nowMs;
        entry.active = !threadWasDeletedAndSuppressed;
        entry.ongoing = ongoing;
        entry.isGroup = isGroup;
        entry.read = threadWasDeletedAndSuppressed;
        entry.dismissed = threadWasDeletedAndSuppressed;
        entry.canReply = threadCanReply;
        entry.hasContentIntent = contentIntent != null
            || (replyHandle != null && replyHandle.contentIntent != null);
        entry.otpCode = otpCode;
        entry.firstUrl = firstUrl;
        entry.threadId = threadId;
        entry.avatarB64 = safeAvatarB64;
        entry.progressCurrent = Math.max(0, progressCurrent);
        entry.progressMax = Math.max(0, progressMax);
        entry.progressIndeterminate = progressIndeterminate;

        upsertNotificationRecord(entry, safeKey, ongoing);
        feedDirty = true;
        saveToDisk();
        return entry;
    }

    public synchronized JSONObject ingestNotification(
            String sbnKey,
            String packageName,
            String appName,
            String categoryHint,
            String title,
            String conversationTitle,
            String text,
            String subText,
            long postTimeMs,
            boolean ongoing,
            boolean isGroup,
            boolean hasProgress,
            List<StyleMessage> styleMessages,
            RemoteReplyHandle replyHandle,
            PendingIntent contentIntent) throws JSONException {
        return ingestNotification(
            sbnKey, packageName, appName, categoryHint, title, conversationTitle,
            text, subText, postTimeMs, ongoing, isGroup, hasProgress,
            styleMessages, replyHandle, contentIntent, "", 0, 0, false
        );
    }

    public synchronized JSONObject ingestNotification(
            String sbnKey,
            String packageName,
            String appName,
            String categoryHint,
            String title,
            String conversationTitle,
            String text,
            String subText,
            long postTimeMs,
            boolean ongoing,
            boolean isGroup,
            boolean hasProgress,
            List<StyleMessage> styleMessages,
            RemoteReplyHandle replyHandle,
            PendingIntent contentIntent,
            String avatarB64) throws JSONException {
        return ingestNotification(
            sbnKey, packageName, appName, categoryHint, title, conversationTitle,
            text, subText, postTimeMs, ongoing, isGroup, hasProgress,
            styleMessages, replyHandle, contentIntent, avatarB64, 0, 0, false
        );
    }

    public synchronized JSONObject ingestNotification(
            String sbnKey,
            String packageName,
            String appName,
            String categoryHint,
            String title,
            String conversationTitle,
            String text,
            String subText,
            long postTimeMs,
            boolean ongoing,
            boolean isGroup,
            boolean hasProgress,
            List<StyleMessage> styleMessages,
            RemoteReplyHandle replyHandle,
            PendingIntent contentIntent,
            String avatarB64,
            int progressCurrent,
            int progressMax,
            boolean progressIndeterminate) throws JSONException {
        NotificationRecord entry = ingestRecord(
            sbnKey, packageName, appName, categoryHint, title, conversationTitle,
            text, subText, postTimeMs, ongoing, isGroup, hasProgress,
            styleMessages, replyHandle, contentIntent, avatarB64,
            progressCurrent, progressMax, progressIndeterminate
        );
        if (entry == null) return null;

        JSONObject delta = new JSONObject();
        delta.put("event", "notification_posted");
        delta.put("revision", revision);
        delta.put("seqId", entry.seqId);
        delta.put("notification", entry.toJson());
        if (entry.threadId != null && threadsById.containsKey(entry.threadId)) {
            delta.put("thread", threadsById.get(entry.threadId).toJson());
        }
        return delta;
    }

    private void upsertNotificationRecord(NotificationRecord entry, String sbnKey, boolean ongoing) {
        if (!sbnKey.isEmpty()) {
            for (int i = 0; i < notifications.size(); i++) {
                NotificationRecord existing = notifications.get(i);
                if (sbnKey.equals(existing.key)) {
                    if (ongoing || entry.eventId.equals(existing.eventId)
                            || (existing.active
                                && entry.title.equals(existing.title)
                                && entry.text.equals(existing.text))) {
                        notifications.remove(i);
                        break;
                    } else {
                        existing.active = false;
                        existing.dismissed = true;
                        existing.read = true;
                        existing.cachedJson = null;
                    }
                }
            }
        }
        notifications.add(0, entry);
        while (notifications.size() > MAX_NOTIFICATIONS) {
            historyTruncated = true;
            notifications.remove(notifications.size() - 1);
        }
    }

    private void upsertThreadFromNotification(
            String threadId,
            String sbnKey,
            String packageName,
            String appName,
            String rawTitle,
            String text,
            long postTimeMs,
            boolean isGroupHint,
            boolean hasContentIntent,
            List<StyleMessage> styleMessages,
            String avatarB64) {
        if (ImAppCatalog.isNativeSmsPackage(packageName)) {
            return;
        }
        String cleanTitle = ImAppCatalog.cleanConversationTitle(packageName, rawTitle);
        if (cleanTitle.isEmpty()) cleanTitle = appName;

        String canonicalKey = canonicalThreadKey(threadId, packageName, cleanTitle);
        long deletedAt = getDeletedAtMs(threadId, packageName, cleanTitle);
        long maxMsgTs = postTimeMs;
        if (styleMessages != null) {
            for (StyleMessage sm : styleMessages) {
                if (sm != null && sm.timestampMs > maxMsgTs) {
                    maxMsgTs = sm.timestampMs;
                }
            }
        }
        if (deletedAt > 0L) {
            if (maxMsgTs <= deletedAt) {
                return;
            }
            deletedThreads.remove(threadId);
            if (!canonicalKey.isEmpty()) {
                deletedThreads.remove(canonicalKey);
                clearedThreads.put(canonicalKey, Math.max(getClearedAtMs(threadId, packageName, cleanTitle), deletedAt));
            }
        }
        long clearedAt = getClearedAtMs(threadId, packageName, cleanTitle);

        // Clear stale reused sbnKey on any other conversation thread unless titles refer to same conversation
        if (sbnKey != null && !sbnKey.isEmpty()) {
            for (ThreadRecord other : threadsById.values()) {
                if (other != null && !threadId.equals(other.threadId) && sbnKey.equals(other.lastNotificationKey)) {
                    if (!ImAppCatalog.isGenericPlaceholderTitle(cleanTitle)
                            && !titlesReferToSameConversation(packageName, other.title, cleanTitle)) {
                        other.lastNotificationKey = "";
                    }
                }
            }
        }

        ThreadRecord thread = threadsById.get(threadId);
        if (thread == null && !ImAppCatalog.isGenericPlaceholderTitle(cleanTitle)) {
            for (ThreadRecord candidate : threadsById.values()) {
                if (candidate != null && packageName.equals(candidate.packageName)
                        && !ImAppCatalog.isGenericPlaceholderTitle(candidate.title)
                        && titlesReferToSameConversation(packageName, candidate.title, cleanTitle)) {
                    thread = candidate;
                    threadId = candidate.threadId;
                    cleanTitle = preferCleanerConversationTitle(packageName, candidate.title, cleanTitle);
                    break;
                }
            }
        }
        if (thread == null) {
            thread = new ThreadRecord();
            thread.threadId = threadId;
            thread.packageName = packageName;
            thread.appName = appName;
            thread.title = cleanTitle;
            thread.unreadCount = 0;
        }

        boolean hadExistingMessages = !thread.messages.isEmpty();
        long prevLastTimeMs = thread.lastTimeMs;
        String prevLastSender = thread.lastSender;
        String prevLastMessage = thread.lastMessage;

        boolean isGroup = isGroupHint || thread.isGroup || "群聊".equals(cleanTitle);
        int addedCount = 0;
        String lastSender = thread.lastSender.isEmpty() ? cleanTitle : thread.lastSender;
        String lastBody = thread.lastMessage.isEmpty() ? text : thread.lastMessage;
        long latestTs = Math.max(postTimeMs, thread.lastTimeMs);
        String knownRealSender = findSingleKnownIncomingSender(thread.messages);

        if (styleMessages != null && !styleMessages.isEmpty()) {
            if (styleMessages.size() > 1) {
                isGroup = isGroupHint || hasDistinctSenders(styleMessages, cleanTitle);
            }
            for (StyleMessage sm : styleMessages) {
                if (sm == null || sm.text.trim().isEmpty()) continue;
                boolean smOutgoing = sm.outgoing || "我".equals(sm.sender) || "Me".equalsIgnoreCase(sm.sender) || "自己".equals(sm.sender);
                String s = smOutgoing
                    ? "我"
                    : (sm.sender.isEmpty() ? cleanTitle : ImAppCatalog.cleanConversationTitle(packageName, sm.sender));
                if (!smOutgoing && ImAppCatalog.isGenericSenderPlaceholder(s) && !knownRealSender.isEmpty()) {
                    s = knownRealSender;
                }
                long ts = sm.timestampMs > 0 ? sm.timestampMs : postTimeMs;
                if (clearedAt > 0L && ts <= clearedAt) continue;
                String smBody = sm.text.trim();
                if (isMessageDeleted(threadId, null, smBody)) continue;
                String status = smOutgoing ? "sent" : "received";
                if (appendMessageIfNotDuplicate(thread.messages, s, smBody, ts, smOutgoing, status, "notification", "")) {
                    addedCount++;
                }
                lastSender = s;
                lastBody = smBody;
                if (ts > latestTs) latestTs = ts;
            }
        } else if (!text.isEmpty() && (clearedAt <= 0L || postTimeMs > clearedAt)) {
            ImAppCatalog.SenderBody sb = ImAppCatalog.extractSenderAndBody(packageName, cleanTitle, text, isGroup);
            if (!sb.sender.isEmpty() && !sb.sender.equals(cleanTitle)) {
                isGroup = true;
            }
            String sender = sb.sender.isEmpty() ? cleanTitle : sb.sender;
            if (ImAppCatalog.isGenericSenderPlaceholder(sender) && !knownRealSender.isEmpty()) {
                sender = knownRealSender;
            }
            String body = sb.body.isEmpty() ? text : sb.body;
            if (!isMessageDeleted(threadId, null, body)) {
                if (appendMessageIfNotDuplicate(thread.messages, sender, body, postTimeMs, false, "received", "notification", "")) {
                    addedCount++;
                }
                lastSender = sender;
                lastBody = body;
            }
        }

        int unread = hadExistingMessages && addedCount == 0
            ? thread.unreadCount
            : (thread.unreadCount + Math.max(1, addedCount));
        boolean canReply = handlesByThread.containsKey(threadId);
        String effectiveTitle = cleanTitle;
        if (ImAppCatalog.isGenericPlaceholderTitle(cleanTitle)
                && thread.title != null
                && !thread.title.isEmpty()
                && (!ImAppCatalog.isGenericPlaceholderTitle(thread.title)
                    || thread.title.startsWith("群聊 · ")
                    || thread.title.startsWith("群聊(")
                    || thread.title.startsWith("群聊（"))) {
            effectiveTitle = thread.title;
        }
        String preview = isGroup && !lastSender.isEmpty() && !lastSender.equals(effectiveTitle)
            ? (lastSender + ": " + lastBody)
            : lastBody;

        boolean tailIsOutgoing = !thread.messages.isEmpty()
            && thread.messages.get(thread.messages.size() - 1).outgoing;
        if (hadExistingMessages && addedCount == 0 && (postTimeMs <= prevLastTimeMs || tailIsOutgoing)) {
            lastSender = prevLastSender;
            preview = prevLastMessage;
            latestTs = prevLastTimeMs;
        }

        thread.title = effectiveTitle;
        thread.appName = appName;
        thread.isGroup = isGroup;
        thread.lastSender = lastSender;
        thread.lastMessage = preview;
        thread.lastTimeMs = latestTs;
        thread.unreadCount = unread;
        thread.canReply = canReply;
        thread.hasContentIntent = thread.hasContentIntent || hasContentIntent;
        if (!sbnKey.isEmpty()) {
            thread.lastNotificationKey = sbnKey;
        }
        if (avatarB64 != null && !avatarB64.isEmpty()) {
            if (!isGroup || thread.avatarB64 == null || thread.avatarB64.isEmpty()) {
                thread.avatarB64 = avatarB64;
            }
            if (!lastSender.isEmpty() && !"我".equals(lastSender) && !ImAppCatalog.isGenericSenderPlaceholder(lastSender)) {
                thread.senderAvatars.put(lastSender, avatarB64);
            }
            if (!isGroup && !effectiveTitle.isEmpty()) {
                thread.senderAvatars.put(effectiveTitle, avatarB64);
            }
        }

        threadsById.put(threadId, thread);
        trimThreadsCapacity();
        threadsDirty = true;
    }

    private static String findSingleKnownIncomingSender(List<MessageRecord> messages) {
        if (messages == null || messages.isEmpty()) return "";
        String candidate = "";
        for (MessageRecord m : messages) {
            if (m == null || m.outgoing || m.sender == null) continue;
            String s = m.sender.trim();
            if (s.isEmpty() || "我".equals(s) || ImAppCatalog.isGenericSenderPlaceholder(s)) continue;
            if (candidate.isEmpty()) {
                candidate = s;
            } else if (!candidate.equals(s)) {
                return "";
            }
        }
        return candidate;
    }

    private static boolean hasDistinctSenders(List<StyleMessage> list, String cleanTitle) {
        for (StyleMessage sm : list) {
            if (sm != null && !sm.outgoing && !"我".equals(sm.sender) && !sm.sender.isEmpty() && !sm.sender.equals(cleanTitle)) {
                return true;
            }
        }
        return false;
    }

    private boolean appendMessageIfNotDuplicate(
            List<MessageRecord> messages,
            String sender,
            String text,
            long timestampMs,
            boolean outgoing,
            String deliveryStatus,
            String via,
            String clientMsgId) {
        boolean recalled = text.contains("撤回了一条消息") || text.contains("recalled a message");
        int start = Math.max(0, messages.size() - 15);
        for (int i = start; i < messages.size(); i++) {
            MessageRecord m = messages.get(i);
            // A screen bubble has an observed direction even when its time is unknown.
            boolean mIsUntimedPreviewOrScrape = "ui_list_preview".equals(m.via);
            boolean sameDirectionOrPreview = m.outgoing == outgoing || mIsUntimedPreviewOrScrape;
            if (sameDirectionOrPreview
                    && (mIsUntimedPreviewOrScrape || sender.equals(m.sender) || ((outgoing || m.outgoing) && "我".equals(m.sender)))
                    && text.equals(m.text)) {
                if (outgoing || m.outgoing) {
                    m.outgoing = true;
                    m.sender = "我";
                    if (!"sent".equals(m.deliveryStatus)) {
                        m.deliveryStatus = "sent";
                    }
                }
                if (!m.timeKnown || m.timestampMs == 0L) {
                    m.timestampMs = timestampMs;
                    m.observedAtMs = System.currentTimeMillis();
                    m.timeKnown = true;
                    m.deliveryStatus = m.outgoing ? "sent" : deliveryStatus;
                    m.via = via;
                    if (clientMsgId != null && !clientMsgId.isEmpty()) {
                        m.clientMsgId = clientMsgId;
                    }
                    m.cachedJson = null;
                    return false;
                }
                if (Math.abs(timestampMs - m.timestampMs) <= 60_000L) {
                    if (clientMsgId != null && !clientMsgId.isEmpty() && (m.clientMsgId == null || m.clientMsgId.isEmpty())) {
                        m.clientMsgId = clientMsgId;
                    }
                    m.cachedJson = null;
                    return false;
                }
            }
        }

        MessageRecord msg = new MessageRecord();
        msg.id = "m_" + timestampMs + "_" + Integer.toHexString((sender + ":" + text).hashCode());
        msg.sender = sender;
        msg.text = text;
        msg.timestampMs = timestampMs;
        msg.observedAtMs = System.currentTimeMillis();
        msg.timeKnown = true;
        msg.outgoing = outgoing;
        msg.deliveryStatus = deliveryStatus;
        msg.via = via;
        msg.clientMsgId = clientMsgId == null ? "" : clientMsgId;
        msg.recalled = recalled;
        messages.add(msg);

        while (messages.size() > MAX_MESSAGES_PER_THREAD) {
            historyTruncated = true;
            messages.remove(0);
        }
        return true;
    }

    private void trimThreadsCapacity() {
        if (threadsById.size() <= MAX_THREADS) return;
        historyTruncated = true;
        List<ThreadRecord> sorted = getSortedThreadsList();
        threadsById.clear();
        int limit = Math.min(MAX_THREADS, sorted.size());
        for (int i = 0; i < limit; i++) {
            ThreadRecord t = sorted.get(i);
            threadsById.put(t.threadId, t);
        }
    }

    /**
     * Core POJO notification removal logic.
     * Marks notification read=true, dismissed=true, and resets thread unreadCount=0 on
     * REASON_APP_CANCEL / REASON_CLICK / user cancel, while SOFT-RETAINING RemoteReplyHandle!
     */
    public synchronized String removeRecord(String sbnKey, int reason, long nowMs) {
        if (sbnKey == null || sbnKey.isEmpty()) return null;
        ongoingThrottle.remove(sbnKey);

        boolean userOrAppRead = (reason == NotificationListenerService.REASON_APP_CANCEL
            || reason == NotificationListenerService.REASON_CLICK
            || reason == NotificationListenerService.REASON_CANCEL
            || reason == NotificationListenerService.REASON_CANCEL_ALL
            || reason == NotificationListenerService.REASON_GROUP_SUMMARY_CANCELED);

        boolean changed = false;
        String affectedThreadId = null;

        for (int i = 0; i < notifications.size(); i++) {
            NotificationRecord n = notifications.get(i);
            if (sbnKey.equals(n.key)) {
                if (n.active || !n.dismissed || !n.read) {
                    n.active = false;
                    n.dismissed = true;
                    n.read = true;
                    if (n.removedTimeMs <= 0L) {
                        n.removedTimeMs = nowMs;
                    }
                    n.removeReason = reason;
                    n.cachedJson = null;
                    changed = true;
                    feedDirty = true;
                }
                if (n.threadId != null && !n.threadId.isEmpty()) {
                    affectedThreadId = n.threadId;
                }
            }
        }

        if (userOrAppRead) {
            for (ThreadRecord thread : threadsById.values()) {
                if ((affectedThreadId != null && affectedThreadId.equals(thread.threadId))
                        || sbnKey.equals(thread.lastNotificationKey)) {
                    if (thread.unreadCount > 0) {
                        thread.unreadCount = 0;
                        changed = true;
                        threadsDirty = true;
                    }
                    // Crucial: keep canReply=true because handle is soft-retained in memory!
                    thread.canReply = handlesByThread.containsKey(thread.threadId);
                }
            }
        }

        if (!changed) return null;
        revision++;
        saveToDisk();
        return affectedThreadId != null ? affectedThreadId : "";
    }

    /**
     * Reconciles stored active notifications against the currently active StatusBarNotification keys
     * from NotificationListenerService.getActiveNotifications(). Any stored notification whose key
     * is absent from activeKeys is marked inactive, dismissed, and read.
     */
    public synchronized int reconcileActiveNotifications(java.util.Set<String> activeKeys, long nowMs) {
        if (activeKeys == null) return 0;
        int reconciled = 0;
        for (int i = 0; i < notifications.size(); i++) {
            NotificationRecord n = notifications.get(i);
            if (n == null) continue;
            if (!activeKeys.contains(n.key)) {
                if (n.active || !n.dismissed || !n.read || n.removedTimeMs <= 0L) {
                    n.active = false;
                    n.dismissed = true;
                    n.read = true;
                    if (n.removedTimeMs <= 0L) {
                        n.removedTimeMs = nowMs;
                    }
                    n.cachedJson = null;
                    reconciled++;
                }
            }
        }
        if (reconciled > 0) {
            feedDirty = true;
            revision++;
            saveToDisk();
        }
        return reconciled;
    }

    public synchronized JSONObject onNotificationRemoved(String sbnKey, int reason) throws JSONException {
        return onNotificationRemoved(sbnKey, reason, System.currentTimeMillis());
    }

    public synchronized JSONObject onNotificationRemoved(String sbnKey, int reason, long nowMs) throws JSONException {
        String affectedThreadId = removeRecord(sbnKey, reason, nowMs);
        if (affectedThreadId == null) return null;

        boolean userOrAppRead = (reason == NotificationListenerService.REASON_APP_CANCEL
            || reason == NotificationListenerService.REASON_CLICK
            || reason == NotificationListenerService.REASON_CANCEL
            || reason == NotificationListenerService.REASON_CANCEL_ALL
            || reason == NotificationListenerService.REASON_GROUP_SUMMARY_CANCELED);

        JSONObject delta = new JSONObject();
        delta.put("event", "notification_removed");
        delta.put("revision", revision);
        delta.put("key", sbnKey);
        delta.put("reason", reason);
        delta.put("read", userOrAppRead);
        if (!affectedThreadId.isEmpty()) {
            delta.put("threadId", affectedThreadId);
        }
        return delta;
    }

    /**
     * Core POJO remote reply logic with 64-entry clientMsgId idempotency and handle soft-retention.
     */
    public synchronized ReplyOutcome executeRemoteReplyOutcome(
            Context context,
            String threadId,
            String text,
            String clientMsgId) throws Exception {
        if (text == null || text.trim().isEmpty()) {
            throw new IllegalArgumentException("回复内容不能为空");
        }
        String trimmedText = text.trim();
        String safeClientId = clientMsgId == null ? "" : clientMsgId.trim();

        // 1. Idempotency check on clientMsgId (64 recent IDs)
        if (!safeClientId.isEmpty() && recentClientMsgIds.containsKey(safeClientId)) {
            ReplyOutcome prev = recentClientMsgIds.get(safeClientId);
            return new ReplyOutcome(
                prev.threadId, prev.via, safeClientId, true, revision, prev.timestampMs, prev.thread
            );
        }

        // 2. Lookup soft-retained RemoteReplyHandle by threadId, alias, enriched title, or notification key
        RemoteReplyHandle handle = getReplyHandle(threadId);
        if (handle == null) {
            throw new IllegalStateException("当前会话暂无后台快捷回复句柄，请解锁手机使用看屏发送或等待对方发来新消息");
        }

        ThreadRecord existingThread = getThreadRecord(threadId);
        if (existingThread == null && !handle.threadId.isEmpty()) {
            existingThread = getThreadRecord(handle.threadId);
        }
        String resolvedThreadId = existingThread != null
            ? existingThread.threadId
            : (!handle.threadId.isEmpty() ? handle.threadId : threadId);
        String resolvedTitle = (existingThread != null && existingThread.title != null && !existingThread.title.isEmpty())
            ? existingThread.title
            : handle.title;

        // 3. Dispatch PendingIntent with RemoteInput results (do NOT cancel notification!)
        try {
            replyDispatcher.send(context, handle, trimmedText);
        } catch (PendingIntent.CanceledException canceled) {
            handlesByThread.remove(resolvedThreadId);
            if (!handle.threadId.isEmpty()) {
                handlesByThread.remove(handle.threadId);
            }
            if (!handle.sbnKey.isEmpty()) {
                handlesByKey.remove(handle.sbnKey);
            }
            ThreadRecord thread = threadsById.get(resolvedThreadId);
            if (thread != null) {
                thread.canReply = false;
                threadsDirty = true;
            }
            revision++;
            saveToDisk();
            throw new IllegalStateException("通知快捷回复句柄已过期，请解锁手机通过看屏通道发送", canceled);
        }

        // 4. Record outgoing bubble & bump revision
        long nowMs = System.currentTimeMillis();
        String appName = ImAppCatalog.appDisplayName(context, handle.packageName);
        ThreadRecord thread = recordOutgoingMessageInternal(
            resolvedThreadId,
            handle.packageName,
            appName,
            resolvedTitle,
            trimmedText,
            nowMs,
            "remote_input",
            safeClientId
        );

        ReplyOutcome outcome = new ReplyOutcome(
            resolvedThreadId, "remote_input", safeClientId, false, revision, nowMs, thread
        );
        if (!safeClientId.isEmpty()) {
            recentClientMsgIds.put(safeClientId, outcome);
        }
        return outcome;
    }

    public synchronized JSONObject executeRemoteReply(
            Context context,
            String threadId,
            String text,
            String clientMsgId) throws Exception {
        ReplyOutcome outcome = executeRemoteReplyOutcome(context, threadId, text, clientMsgId);
        return outcomeToJson(outcome);
    }

    public synchronized JSONObject recordOutgoingUiMessage(
            Context context,
            String packageName,
            String rawTitle,
            String text,
            String clientMsgId) throws JSONException {
        String trimmedText = text == null ? "" : text.trim();
        String safeClientId = clientMsgId == null ? "" : clientMsgId.trim();
        if (!safeClientId.isEmpty() && recentClientMsgIds.containsKey(safeClientId)) {
            ReplyOutcome prev = recentClientMsgIds.get(safeClientId);
            return outcomeToJson(new ReplyOutcome(
                prev.threadId, prev.via, safeClientId, true, revision, prev.timestampMs, prev.thread
            ));
        }

        String safePkg = packageName == null || packageName.isEmpty() ? "com.tencent.mm" : packageName;
        String appName = ImAppCatalog.appDisplayName(context, safePkg);
        String cleanTitle = ImAppCatalog.cleanConversationTitle(safePkg, rawTitle);
        if (cleanTitle.isEmpty()) cleanTitle = appName;
        String threadId = ImAppCatalog.buildThreadId(safePkg, cleanTitle);
        ThreadRecord existing = getThreadRecord(ImAppCatalog.buildThreadId(safePkg, rawTitle == null ? cleanTitle : rawTitle.trim()));
        if (existing == null) {
            existing = getThreadRecord(threadId);
        }
        if (existing != null) {
            threadId = existing.threadId;
            if (existing.title != null && !existing.title.isEmpty()
                    && (ImAppCatalog.isGenericPlaceholderTitle(cleanTitle) || "群聊".equals(cleanTitle))) {
                cleanTitle = existing.title;
            }
        }
        long nowMs = System.currentTimeMillis();

        ThreadRecord thread = recordOutgoingMessageInternal(
            threadId, safePkg, appName, cleanTitle, trimmedText, nowMs, "ui_bridge", safeClientId
        );

        ReplyOutcome outcome = new ReplyOutcome(
            threadId, "ui_bridge", safeClientId, false, revision, nowMs, thread
        );
        if (!safeClientId.isEmpty()) {
            recentClientMsgIds.put(safeClientId, outcome);
        }
        return outcomeToJson(outcome);
    }

    private static JSONObject outcomeToJson(ReplyOutcome outcome) throws JSONException {
        JSONObject response = new JSONObject();
        response.put("ok", true);
        response.put("via", outcome.via);
        response.put("deliveryStatus", "sent");
        response.put("threadId", outcome.threadId);
        response.put("clientMsgId", outcome.clientMsgId);
        response.put("timestampMs", outcome.timestampMs);
        response.put("message", "已发送");
        response.put("deduplicated", outcome.deduplicated);
        response.put("revision", outcome.revision);
        if (outcome.thread != null) {
            response.put("thread", outcome.thread.toJson());
        }
        return response;
    }

    private ThreadRecord recordOutgoingMessageInternal(
            String threadId,
            String packageName,
            String appName,
            String rawTitle,
            String text,
            long nowMs,
            String via,
            String clientMsgId) {
        String cleanTitle = ImAppCatalog.cleanConversationTitle(packageName, rawTitle);
        if (cleanTitle.isEmpty()) cleanTitle = appName;
        String canonicalKey = canonicalThreadKey(threadId, packageName, cleanTitle);
        deletedThreads.remove(threadId);
        if (!canonicalKey.isEmpty()) {
            deletedThreads.remove(canonicalKey);
        }

        ThreadRecord thread = threadsById.get(threadId);
        if (thread == null && !ImAppCatalog.isGenericPlaceholderTitle(cleanTitle)) {
            for (ThreadRecord candidate : threadsById.values()) {
                if (candidate != null && packageName.equals(candidate.packageName)
                        && !ImAppCatalog.isGenericPlaceholderTitle(candidate.title)
                        && titlesReferToSameConversation(packageName, candidate.title, cleanTitle)) {
                    thread = candidate;
                    threadId = candidate.threadId;
                    cleanTitle = preferCleanerConversationTitle(packageName, candidate.title, cleanTitle);
                    break;
                }
            }
        }
        if (thread == null) {
            thread = new ThreadRecord();
            thread.threadId = threadId;
            thread.packageName = packageName;
            thread.appName = appName;
            thread.title = (rawTitle != null && rawTitle.startsWith("群聊(")) ? rawTitle.trim() : cleanTitle;
        } else if (rawTitle != null && !rawTitle.trim().isEmpty()
                && ImAppCatalog.isGenericPlaceholderTitle(thread.title)
                && !ImAppCatalog.isGenericPlaceholderTitle(cleanTitle)) {
            thread.title = cleanTitle;
        }

        appendMessageIfNotDuplicate(thread.messages, "我", text, nowMs, true, "sent", via, clientMsgId);

        thread.lastSender = "我";
        thread.lastMessage = "我: " + text;
        thread.lastTimeMs = nowMs;
        thread.unreadCount = 0;
        thread.canReply = hasReplyHandle(threadId);

        threadsById.put(threadId, thread);
        trimThreadsCapacity();
        threadsDirty = true;
        revision++;
        saveToDisk();
        return thread;
    }

    public static final class CaptureMergeOutcome {
        public final String packageName;
        public final String appName;
        public final String title;
        public final String threadId;
        public final String migratedFromThreadId;
        public final int capturedCount;
        public final int upgradedSendersCount;
        public final long revision;
        public final ThreadRecord thread;

        public CaptureMergeOutcome(
                String packageName,
                String appName,
                String title,
                String threadId,
                String migratedFromThreadId,
                int capturedCount,
                int upgradedSendersCount,
                long revision,
                ThreadRecord thread) {
            this.packageName = packageName;
            this.appName = appName;
            this.title = title;
            this.threadId = threadId;
            this.migratedFromThreadId = migratedFromThreadId;
            this.capturedCount = capturedCount;
            this.upgradedSendersCount = upgradedSendersCount;
            this.revision = revision;
            this.thread = thread;
        }
    }

    /**
     * Merges chat bubbles captured from the active foreground window by ChatUiBridge.
     */
    public synchronized JSONObject mergeCapturedWindowMessages(
            String packageName,
            String appName,
            String rawTitle,
            List<CapturedBubble> bubbles) throws JSONException {
        return mergeCapturedWindowMessages(packageName, appName, rawTitle, bubbles, false, null);
    }

    /**
     * Merges chat bubbles captured from the active window, upgrading generic "群聊" threads
     * and "微信用户" sender placeholders in-place across both im_threads.json and push_feed.json.
     */
    public synchronized JSONObject mergeCapturedWindowMessages(
            String packageName,
            String appName,
            String rawTitle,
            List<CapturedBubble> bubbles,
            boolean isGroupHint,
            String aliasTitleOrThreadId) throws JSONException {
        return mergeCapturedWindowMessages(
            packageName, appName, rawTitle, bubbles, isGroupHint, aliasTitleOrThreadId, "", null
        );
    }

    public synchronized JSONObject mergeCapturedWindowMessages(
            String packageName,
            String appName,
            String rawTitle,
            List<CapturedBubble> bubbles,
            boolean isGroupHint,
            String aliasTitleOrThreadId,
            String threadAvatarB64,
            Map<String, String> senderAvatars) throws JSONException {
        CaptureMergeOutcome outcome = mergeCapturedWindowMessagesOutcome(
            packageName, appName, rawTitle, bubbles, isGroupHint, aliasTitleOrThreadId, threadAvatarB64, senderAvatars
        );
        JSONObject threadJson = outcome.thread.toJson();
        JSONObject result = new JSONObject();
        result.put("ok", true);
        result.put("packageName", outcome.packageName);
        result.put("appName", outcome.appName);
        result.put("title", outcome.title);
        result.put("threadId", outcome.threadId);
        if (outcome.migratedFromThreadId != null) {
            result.put("migratedFromThreadId", outcome.migratedFromThreadId);
        }
        result.put("capturedCount", outcome.capturedCount);
        result.put("upgradedSendersCount", outcome.upgradedSendersCount);
        result.put("messages", threadJson.optJSONArray("messages"));
        result.put("thread", threadJson);
        result.put("revision", outcome.revision);
        return result;
    }

    public synchronized CaptureMergeOutcome mergeCapturedWindowMessagesOutcome(
            String packageName,
            String appName,
            String rawTitle,
            List<CapturedBubble> bubbles,
            boolean isGroupHint,
            String aliasTitleOrThreadId) {
        return mergeCapturedWindowMessagesOutcome(
            packageName, appName, rawTitle, bubbles, isGroupHint, aliasTitleOrThreadId, "", null
        );
    }

    public synchronized CaptureMergeOutcome mergeCapturedWindowMessagesOutcome(
            String packageName,
            String appName,
            String rawTitle,
            List<CapturedBubble> bubbles,
            boolean isGroupHint,
            String aliasTitleOrThreadId,
            String threadAvatarB64,
            Map<String, String> senderAvatars) {
        String safePkg = packageName == null ? "" : packageName;
        if (ImAppCatalog.isNativeSmsPackage(safePkg)) {
            return null;
        }
        String safeAppName = (appName == null || appName.isEmpty())
            ? ImAppCatalog.appDisplayName(null, safePkg)
            : appName;
        String cleanTitle = ImAppCatalog.cleanConversationTitle(safePkg, rawTitle);
        if (cleanTitle.isEmpty()) cleanTitle = safeAppName;

        boolean isServiceContainer = isServiceNotificationContainerTitle(cleanTitle);
        boolean hasGroupCount = ImAppCatalog.hasGroupMemberCountSuffix(rawTitle);
        boolean isGroup = !isServiceContainer && (isGroupHint || hasGroupCount || "群聊".equals(cleanTitle));
        if (!isGroup && !isServiceContainer && bubbles != null) {
            for (CapturedBubble b : bubbles) {
                if (b != null && !b.outgoing && !b.sender.trim().isEmpty()
                        && !b.sender.trim().equals(cleanTitle)
                        && !ImAppCatalog.isGenericSenderPlaceholder(b.sender)) {
                    isGroup = true;
                    break;
                }
            }
        }

        // If this is a service notification container ("服务通知" / "公众号"), also split each distinct
        // merchant card into its own merchant thread (packageName + "::" + merchantSender) so merchant
        // conversations each have their full messages without polluting a single thread.
        CaptureMergeOutcome matchedMerchantOutcome = null;
        if (isServiceContainer && bubbles != null && !bubbles.isEmpty()) {
            Map<String, List<CapturedBubble>> byMerchant = new LinkedHashMap<String, List<CapturedBubble>>();
            for (CapturedBubble b : bubbles) {
                if (b == null || b.outgoing || b.text == null || b.text.trim().isEmpty()) continue;
                String mSender = b.sender == null ? "" : b.sender.trim();
                if (mSender.isEmpty() || mSender.equals(cleanTitle)
                        || isServiceNotificationContainerTitle(mSender)
                        || ImAppCatalog.isGenericSenderPlaceholder(mSender)) {
                    continue;
                }
                List<CapturedBubble> list = byMerchant.get(mSender);
                if (list == null) {
                    list = new ArrayList<CapturedBubble>();
                    byMerchant.put(mSender, list);
                }
                list.add(b);
            }
            for (Map.Entry<String, List<CapturedBubble>> entry : byMerchant.entrySet()) {
                String merchantName = entry.getKey();
                List<CapturedBubble> merchantBubbles = entry.getValue();
                String merchantAvatar = "";
                for (CapturedBubble mb : merchantBubbles) {
                    if (mb.avatarB64 != null && !mb.avatarB64.isEmpty()) {
                        merchantAvatar = mb.avatarB64;
                        break;
                    }
                }
                if (merchantAvatar.isEmpty() && senderAvatars != null && senderAvatars.containsKey(merchantName)) {
                    merchantAvatar = senderAvatars.get(merchantName);
                }
                CaptureMergeOutcome mOut = mergeCapturedWindowMessagesOutcome(
                    safePkg, safeAppName, merchantName, merchantBubbles, false, "", merchantAvatar, senderAvatars
                );
                if (aliasTitleOrThreadId != null && !aliasTitleOrThreadId.trim().isEmpty()
                        && titlesReferToSameConversation(safePkg, merchantName, aliasTitleOrThreadId)) {
                    matchedMerchantOutcome = mOut;
                }
            }
        }

        String threadId = ImAppCatalog.buildThreadId(safePkg, cleanTitle);
        long nowMs = System.currentTimeMillis();

        // Check if there is an existing generic "群聊" thread that should be merged into this real thread
        String migratedFromThreadId = null;
        ThreadRecord genericSource = isServiceContainer ? null : findMatchingGenericThreadToUpgrade(
            safePkg, threadId, cleanTitle, isGroup, bubbles, aliasTitleOrThreadId
        );

        ThreadRecord thread = threadsById.get(threadId);
        if (thread == null && genericSource == null) {
            for (ThreadRecord candidate : threadsById.values()) {
                if (candidate != null && safePkg.equals(candidate.packageName)
                        && titlesReferToSameConversation(safePkg, candidate.title, cleanTitle)) {
                    thread = candidate;
                    threadId = candidate.threadId;
                    cleanTitle = preferCleanerConversationTitle(safePkg, candidate.title, cleanTitle);
                    break;
                }
            }
        }
        if (genericSource != null && !genericSource.threadId.equals(threadId)) {
            migratedFromThreadId = genericSource.threadId;
            if (thread == null) {
                thread = genericSource;
                threadsById.remove(migratedFromThreadId);
                thread.threadId = threadId;
                thread.title = cleanTitle;
                thread.packageName = safePkg;
                thread.appName = safeAppName;
            } else {
                mergeSourceThreadIntoTarget(genericSource, thread);
                threadsById.remove(migratedFromThreadId);
            }
            if (handlesByThread.containsKey(migratedFromThreadId)) {
                RemoteReplyHandle oldHandle = handlesByThread.remove(migratedFromThreadId);
                if (oldHandle != null && !handlesByThread.containsKey(threadId)) {
                    RemoteReplyHandle rebound = new RemoteReplyHandle(
                        oldHandle.actionIntent,
                        oldHandle.remoteInputs,
                        oldHandle.contentIntent,
                        oldHandle.sbnKey,
                        safePkg,
                        cleanTitle,
                        threadId,
                        oldHandle.capturedAtMs
                    );
                    handlesByThread.put(threadId, rebound);
                    if (!oldHandle.sbnKey.isEmpty()) {
                        handlesByKey.put(oldHandle.sbnKey, rebound);
                    }
                }
            }
            if (contentIntentsByThread.containsKey(migratedFromThreadId)) {
                PendingIntent oldPi = contentIntentsByThread.remove(migratedFromThreadId);
                if (oldPi != null && !contentIntentsByThread.containsKey(threadId)) {
                    contentIntentsByThread.put(threadId, oldPi);
                }
            }
            resolvedGenericThreadAliases.put(migratedFromThreadId, threadId);
        }

        if (thread == null) {
            thread = new ThreadRecord();
            thread.threadId = threadId;
            thread.packageName = safePkg;
            thread.appName = safeAppName;
            thread.title = cleanTitle;
            thread.unreadCount = 0;
        } else {
            for (int i = thread.messages.size() - 1; i >= 0; i--) {
                MessageRecord m = thread.messages.get(i);
                if (m != null && m.text != null) {
                    String mt = m.text.trim();
                    if ("ImageView".equals(mt) || "credential".equals(mt) || mt.startsWith("[\\u0001-")) {
                        thread.messages.remove(i);
                    }
                }
            }
        }
        thread.isGroup = thread.isGroup || isGroup;

        boolean avatarsChanged = false;
        if (threadAvatarB64 != null && !threadAvatarB64.trim().isEmpty()) {
            String cleanAv = threadAvatarB64.trim();
            if (!cleanAv.equals(thread.avatarB64)) {
                thread.avatarB64 = cleanAv;
                avatarsChanged = true;
            }
            if (!thread.isGroup && !cleanTitle.isEmpty()) {
                thread.senderAvatars.put(cleanTitle, cleanAv);
            }
        }
        if (senderAvatars != null && !senderAvatars.isEmpty()) {
            for (Map.Entry<String, String> e : senderAvatars.entrySet()) {
                String k = e.getKey() == null ? "" : e.getKey().trim();
                String v = e.getValue() == null ? "" : e.getValue().trim();
                if (!k.isEmpty() && !v.isEmpty() && !v.equals(thread.senderAvatars.get(k))) {
                    thread.senderAvatars.put(k, v);
                    avatarsChanged = true;
                }
            }
        }

        int added = 0;
        int upgradedSenders = 0;
        Map<String, Integer> occurrences = new HashMap<String, Integer>();
        int initialMessageCount = thread.messages.size();
        boolean[] matchedExisting = new boolean[initialMessageCount];
        String lastBody = thread.lastMessage;
        String lastSender = thread.lastSender.isEmpty() ? cleanTitle : thread.lastSender;

        if (bubbles != null) {
            int bubbleCount = bubbles.size();
            MessageRecord[] matchedRecords = new MessageRecord[bubbleCount];
            MessageRecord[] pendingNewRecords = new MessageRecord[bubbleCount];
            boolean[] bubbleIsOutgoing = new boolean[bubbleCount];
            String[] bubbleSender = new String[bubbleCount];
            String[] bubbleBody = new String[bubbleCount];
            boolean[] bubbleValid = new boolean[bubbleCount];

            for (int k = 0; k < bubbleCount; k++) {
                CapturedBubble b = bubbles.get(k);
                if (b == null || b.text == null || b.text.trim().isEmpty()) continue;
                String rawSender = b.sender == null ? "" : b.sender.trim();
                String sender = b.outgoing ? "我" : (rawSender.isEmpty() ? cleanTitle : rawSender);
                String body = ChatUiBridge.normalizeVoiceBubbleText(b.text.trim());
                if (body.isEmpty()) continue;
                if (b.avatarB64 != null && !b.avatarB64.trim().isEmpty()) {
                    String bAv = b.avatarB64.trim();
                    if (!sender.isEmpty() && !bAv.equals(thread.senderAvatars.get(sender))) {
                        thread.senderAvatars.put(sender, bAv);
                        avatarsChanged = true;
                    }
                    if (!b.outgoing && (!thread.isGroup || thread.avatarB64 == null || thread.avatarB64.isEmpty())) {
                        if (!bAv.equals(thread.avatarB64)) {
                            thread.avatarB64 = bAv;
                            avatarsChanged = true;
                        }
                    }
                }
                String fingerprint = ContextProvenance.captureId(sender, body, b.outgoing, 0);
                int occurrence = occurrences.containsKey(fingerprint) ? occurrences.get(fingerprint) : 0;
                occurrences.put(fingerprint, occurrence + 1);
                String captureId = ContextProvenance.captureId(sender, body, b.outgoing, occurrence);
                if (isMessageDeleted(threadId, captureId, body)) {
                    continue;
                }

                bubbleValid[k] = true;
                bubbleIsOutgoing[k] = b.outgoing;
                bubbleSender[k] = sender;
                bubbleBody[k] = body;

                int checkLimit = Math.min(initialMessageCount, thread.messages.size());
                for (int i = 0; i < checkLimit; i++) {
                    if (matchedExisting[i]) continue;
                    MessageRecord existing = thread.messages.get(i);
                    if (existing == null) continue;
                    String existingBody = ChatUiBridge.normalizeVoiceBubbleText(
                        existing.text == null ? "" : existing.text.trim()
                    );
                    boolean isUntimedPreviewOrScrape = "ui_list_preview".equals(existing.via);
                    boolean idMatch = captureId.equals(existing.id);
                    boolean textMatch = (existing.outgoing == b.outgoing || isUntimedPreviewOrScrape)
                        && body.equals(existingBody);
                    boolean prefixedMatch = !b.outgoing && (!existing.outgoing || isUntimedPreviewOrScrape)
                        && (existingBody.endsWith(": " + body) || existingBody.endsWith("：" + body));
                    boolean voiceDurationUpgrade = (existing.outgoing == b.outgoing || isUntimedPreviewOrScrape)
                        && body.startsWith("[语音] ")
                        && ("[语音]".equals(existingBody)
                            || existingBody.endsWith(": [语音]")
                            || existingBody.endsWith("：[语音]"));
                    if (idMatch || textMatch || prefixedMatch || voiceDurationUpgrade) {
                        matchedExisting[i] = true;
                        matchedRecords[k] = existing;
                        if (isUntimedPreviewOrScrape && existing.outgoing != b.outgoing) {
                            existing.outgoing = b.outgoing;
                            existing.sender = b.outgoing ? "我" : sender;
                            if (b.outgoing && !"sent".equals(existing.deliveryStatus)) {
                                existing.deliveryStatus = "sent";
                            }
                            existing.cachedJson = null;
                            upgradedSenders++;
                        }
                        if ((prefixedMatch || voiceDurationUpgrade || !body.equals(existing.text)) && !body.equals(existing.text)) {
                            existing.text = body;
                            existing.cachedJson = null;
                            upgradedSenders++;
                        }
                        if (!existing.outgoing
                                && !ImAppCatalog.isGenericSenderPlaceholder(sender)
                                && (ImAppCatalog.isGenericSenderPlaceholder(existing.sender)
                                    || "群聊".equals(existing.sender))) {
                            existing.sender = sender;
                            existing.cachedJson = null;
                            upgradedSenders++;
                        }
                        if (b.mediaB64 != null && !b.mediaB64.trim().isEmpty()
                                && (existing.mediaB64 == null || existing.mediaB64.isEmpty())) {
                            existing.mediaB64 = b.mediaB64.trim();
                            existing.cachedJson = null;
                            upgradedSenders++;
                        }
                        bubbleIsOutgoing[k] = existing.outgoing;
                        bubbleSender[k] = existing.outgoing ? "我" : existing.sender;
                        break;
                    }
                }

                if (matchedRecords[k] == null) {
                    MessageRecord msg = new MessageRecord();
                    msg.id = captureId;
                    msg.sender = sender;
                    msg.text = body;
                    msg.timestampMs = 0L;
                    msg.observedAtMs = nowMs;
                    msg.timeKnown = false;
                    msg.outgoing = b.outgoing;
                    msg.deliveryStatus = "observed";
                    msg.via = "ui_capture";
                    msg.clientMsgId = "";
                    msg.recalled = body.contains("撤回了一条消息") || body.contains("recalled a message");
                    if (b.mediaB64 != null && !b.mediaB64.trim().isEmpty()) {
                        msg.mediaB64 = b.mediaB64.trim();
                    }
                    pendingNewRecords[k] = msg;
                }
            }

            MessageRecord lastAnchorRecord = null;
            for (int k = 0; k < bubbleCount; k++) {
                if (!bubbleValid[k]) continue;
                MessageRecord matched = matchedRecords[k];
                if (matched != null) {
                    if (lastAnchorRecord != null) {
                        int prevIdx = thread.messages.indexOf(lastAnchorRecord);
                        int currIdx = thread.messages.indexOf(matched);
                        if (prevIdx >= 0 && currIdx >= 0 && currIdx < prevIdx && !matched.timeKnown) {
                            thread.messages.remove(currIdx);
                            int newPrevIdx = thread.messages.indexOf(lastAnchorRecord);
                            thread.messages.add(newPrevIdx + 1, matched);
                        }
                    }
                    lastAnchorRecord = matched;
                } else {
                    MessageRecord msg = pendingNewRecords[k];
                    if (msg != null) {
                        if (lastAnchorRecord != null) {
                            int prevIdx = thread.messages.indexOf(lastAnchorRecord);
                            if (prevIdx >= 0 && prevIdx + 1 <= thread.messages.size()) {
                                thread.messages.add(prevIdx + 1, msg);
                            } else {
                                thread.messages.add(msg);
                            }
                        } else {
                            MessageRecord nextAnchor = null;
                            for (int m = k + 1; m < bubbleCount; m++) {
                                if (matchedRecords[m] != null) {
                                    nextAnchor = matchedRecords[m];
                                    break;
                                }
                            }
                            if (nextAnchor != null) {
                                int nextIdx = thread.messages.indexOf(nextAnchor);
                                if (nextIdx >= 0) {
                                    thread.messages.add(nextIdx, msg);
                                } else {
                                    thread.messages.add(msg);
                                }
                            } else {
                                thread.messages.add(msg);
                            }
                        }
                        lastAnchorRecord = msg;
                        added++;
                    }
                }
                String s = bubbleSender[k];
                String bText = bubbleBody[k];
                boolean isOut = bubbleIsOutgoing[k];
                lastSender = s;
                lastBody = isOut
                    ? ("我: " + bText)
                    : ((thread.isGroup || isServiceContainer) && !s.isEmpty() && !s.equals(cleanTitle)
                        ? (s + ": " + bText)
                        : bText);
            }
            while (thread.messages.size() > MAX_MESSAGES_PER_THREAD) {
                thread.messages.remove(0);
                historyTruncated = true;
            }
        }

        // If any older messages in the thread still have a generic placeholder sender ("微信用户" / "群聊")
        // and we have a single unambiguous real incoming sender (or 1-on-1 contact name), upgrade them too.
        String fallbackRealSender = findSingleKnownIncomingSender(thread.messages);
        if (fallbackRealSender.isEmpty() && !thread.isGroup && !isServiceContainer && !ImAppCatalog.isGenericPlaceholderTitle(cleanTitle)) {
            fallbackRealSender = cleanTitle;
        }
        if (!fallbackRealSender.isEmpty()) {
            for (MessageRecord m : thread.messages) {
                if (m != null && !m.outgoing && ImAppCatalog.isGenericSenderPlaceholder(m.sender)) {
                    m.sender = fallbackRealSender;
                    m.cachedJson = null;
                    upgradedSenders++;
                }
            }
        }

        // Determine the best display title for the thread
        String displayTitle = cleanTitle;
        if ("群聊".equals(cleanTitle)) {
            displayTitle = buildEnrichedGenericGroupTitle(rawTitle, thread.messages);
        }
        boolean titleUpgraded = !displayTitle.equals(thread.title) || migratedFromThreadId != null;
        thread.title = displayTitle;

        long latestKnownMsgTs = 0L;
        if (!thread.messages.isEmpty()) {
            for (MessageRecord m : thread.messages) {
                if (m != null && m.timeKnown && m.timestampMs > latestKnownMsgTs) {
                    latestKnownMsgTs = m.timestampMs;
                }
            }
            MessageRecord tail = thread.messages.get(thread.messages.size() - 1);
            if (tail != null) {
                lastSender = tail.outgoing ? "我" : (tail.sender == null || tail.sender.isEmpty() ? displayTitle : tail.sender);
                if (tail.outgoing) {
                    lastBody = "我: " + tail.text;
                } else if ((thread.isGroup || isServiceContainer) && !lastSender.isEmpty() && !lastSender.equals(displayTitle)) {
                    lastBody = lastSender + ": " + tail.text;
                } else {
                    lastBody = tail.text;
                }
            }
        }
        if (latestKnownMsgTs > thread.lastTimeMs) {
            thread.lastTimeMs = latestKnownMsgTs;
        }
        if ((added > 0 || thread.lastTimeMs <= 0L) && !thread.messages.isEmpty()) {
            thread.lastTimeMs = Math.max(thread.lastTimeMs, nowMs);
        }

        boolean notificationsUpgraded = upgradeNotificationsForThread(safePkg, migratedFromThreadId, thread);
        boolean contentChanged = added > 0
            || upgradedSenders > 0
            || titleUpgraded
            || notificationsUpgraded
            || avatarsChanged
            || !lastSender.equals(thread.lastSender)
            || !lastBody.equals(thread.lastMessage);
        thread.lastSender = lastSender;
        thread.lastMessage = lastBody;
        // A screen observation must not turn an old conversation into today's activity.
        lastObservedMs = nowMs;
        thread.unreadCount = 0;
        thread.canReply = hasReplyHandle(threadId);

        threadsById.put(threadId, thread);
        trimThreadsCapacity();
        if (contentChanged) {
            threadsDirty = true;
            revision++;
            saveToDisk();
        }

        if (matchedMerchantOutcome != null) {
            return matchedMerchantOutcome;
        }

        return new CaptureMergeOutcome(
            safePkg, safeAppName, thread.title, threadId, migratedFromThreadId,
            added, upgradedSenders, revision, thread
        );
    }

    public synchronized int mergeCapturedConversationList(
            String packageName,
            List<CapturedConversationRow> rows) {
        return mergeCapturedConversationList(packageName, "", rows, true);
    }

    public synchronized int mergeCapturedConversationList(
            String packageName,
            String appName,
            List<CapturedConversationRow> rows) {
        return mergeCapturedConversationList(packageName, appName, rows, true);
    }

    /**
     * Uses visible conversation rows from the IM app's main conversation list (e.g. WeChat LauncherUI)
     * to resolve generic "群聊" thread titles and "微信用户" senders when preview snippets overlap,
     * upserts all visible rows with their real user/group avatars into threadsById, and prunes
     * threads that were deleted from the phone's conversation list.
     */
    public synchronized int mergeCapturedConversationList(
            String packageName,
            String appName,
            List<CapturedConversationRow> rows,
            boolean atTopOfList) {
        if (rows == null || rows.isEmpty()) return 0;
        String safePkg = packageName == null || packageName.isEmpty() ? "com.tencent.mm" : packageName.trim();
        if (ImAppCatalog.isNativeSmsPackage(safePkg)) return 0;
        String safeAppName = (appName == null || appName.isEmpty())
            ? ImAppCatalog.appDisplayName(null, safePkg)
            : appName;
        int upgradedThreads = 0;
        boolean anyChanged = false;
        long nowMs = System.currentTimeMillis();
        java.util.Set<String> visibleThreadIds = new java.util.LinkedHashSet<String>();
        java.util.Set<String> visibleCleanTitles = new java.util.LinkedHashSet<String>();
        long minVisibleLastTimeMs = Long.MAX_VALUE;
        int validRowCount = 0;

        for (CapturedConversationRow row : rows) {
            if (row == null || row.rawTitle.isEmpty()) continue;
            String cleanRowTitle = ImAppCatalog.cleanConversationTitle(safePkg, row.rawTitle);
            if (cleanRowTitle.isEmpty()
                    || "微信".equals(cleanRowTitle)
                    || "WeChat".equalsIgnoreCase(cleanRowTitle)) {
                continue;
            }
            validRowCount++;
            String rowThreadId = ImAppCatalog.buildThreadId(safePkg, cleanRowTitle);
            visibleThreadIds.add(rowThreadId);
            visibleCleanTitles.add(cleanRowTitle);

            long deletedAt = getDeletedAtMs(rowThreadId, safePkg, cleanRowTitle);
            if (deletedAt > 0L) {
                if (row.unreadCount > 0) {
                    deletedThreads.remove(rowThreadId);
                    deletedThreads.remove(canonicalThreadKey(rowThreadId, safePkg, cleanRowTitle));
                } else {
                    continue;
                }
            }

            boolean isServiceRow = isServiceNotificationContainerTitle(cleanRowTitle);
            boolean isGroupRow = !isServiceRow && (
                ImAppCatalog.hasGroupMemberCountSuffix(row.rawTitle)
                || "群聊".equals(cleanRowTitle)
                || (!row.senderHint.isEmpty() && !row.senderHint.equals(cleanRowTitle))
            );
            ImAppCatalog.SenderBody sb = ImAppCatalog.extractSenderAndBody(
                safePkg, cleanRowTitle, row.previewText, true
            );
            String sender = !row.senderHint.isEmpty() ? row.senderHint : sb.sender;
            String body = !sb.body.isEmpty() ? sb.body : row.previewText;
            String rowAvatar = row.avatarB64 == null ? "" : row.avatarB64.trim();

            // 1. Check if a generic "群聊" thread has a message matching this row's preview body
            if (!isServiceRow && !body.isEmpty()) {
                String genericThreadId = ImAppCatalog.buildThreadId(safePkg, "群聊");
                ThreadRecord genericThread = threadsById.get(genericThreadId);
                if (genericThread != null && threadContainsMessageBody(genericThread, body)) {
                    boolean outgoing = "我".equals(sender) || row.previewText.startsWith("我:");
                    List<CapturedBubble> single = new ArrayList<CapturedBubble>(1);
                    single.add(new CapturedBubble(outgoing ? "我" : sender, body, outgoing, 0L, rowAvatar));
                    CaptureMergeOutcome out = mergeCapturedWindowMessagesOutcome(
                        safePkg, safeAppName, row.rawTitle, single, isGroupRow || genericThread.isGroup, genericThreadId, rowAvatar, null
                    );
                    if (out != null && out.thread != null) {
                        visibleThreadIds.add(out.thread.threadId);
                        if (out.thread.lastTimeMs > 0L) {
                            minVisibleLastTimeMs = Math.min(minVisibleLastTimeMs, out.thread.lastTimeMs);
                        }
                    }
                    upgradedThreads++;
                    anyChanged = true;
                    continue;
                }
            }

            // 2. Check if an existing thread with this row's title has a generic sender for this message
            ThreadRecord existing = threadsById.get(rowThreadId);
            if (existing == null) {
                for (ThreadRecord candidate : threadsById.values()) {
                    if (candidate != null && safePkg.equals(candidate.packageName)
                            && titlesReferToSameConversation(safePkg, candidate.title, cleanRowTitle)) {
                        existing = candidate;
                        rowThreadId = candidate.threadId;
                        visibleThreadIds.add(rowThreadId);
                        break;
                    }
                }
            }
            if (existing != null && existing.lastTimeMs > 0L) {
                minVisibleLastTimeMs = Math.min(minVisibleLastTimeMs, existing.lastTimeMs);
            }
            if (existing != null && !body.isEmpty()
                    && !sender.isEmpty() && !ImAppCatalog.isGenericSenderPlaceholder(sender)
                    && threadContainsMessageBody(existing, body)) {
                boolean outgoing = "我".equals(sender);
                List<CapturedBubble> single = new ArrayList<CapturedBubble>(1);
                single.add(new CapturedBubble(sender, body, outgoing, 0L, rowAvatar));
                mergeCapturedWindowMessagesOutcome(
                    safePkg, safeAppName, row.rawTitle, single, existing.isGroup || isGroupRow, rowThreadId, rowAvatar, null
                );
                upgradedThreads++;
                anyChanged = true;
                continue;
            }

            // 3. Find or create the ThreadRecord for every valid visible LauncherUI row
            boolean hadExistingMessages = existing != null && !existing.messages.isEmpty();
            if (existing == null) {
                existing = new ThreadRecord();
                existing.threadId = rowThreadId;
                existing.packageName = safePkg;
                existing.appName = safeAppName;
                existing.title = cleanRowTitle;
                existing.isGroup = isGroupRow;
                existing.unreadCount = row.unreadCount;
                existing.canReply = hasReplyHandle(rowThreadId);
                threadsById.put(rowThreadId, existing);
                upgradedThreads++;
                anyChanged = true;
            } else {
                existing.title = preferCleanerConversationTitle(safePkg, existing.title, cleanRowTitle);
                if (row.unreadCount > 0 && existing.unreadCount != row.unreadCount) {
                    existing.unreadCount = row.unreadCount;
                    anyChanged = true;
                }
            }

            if (!rowAvatar.isEmpty()) {
                if (!rowAvatar.equals(existing.avatarB64)) {
                    existing.avatarB64 = rowAvatar;
                    anyChanged = true;
                }
                existing.senderAvatars.put(cleanRowTitle, rowAvatar);
            }

            if (!body.isEmpty() && !isMessageDeleted(rowThreadId, null, body)
                    && (existing.messages.isEmpty() || !threadContainsMessageBody(existing, body))) {
                boolean isInferredPhoneOutgoing = !isGroupRow && !existing.isGroup && !isServiceRow
                    && row.unreadCount == 0
                    && PushNotificationService.isConnected()
                    && hadExistingMessages;
                boolean outgoing = "我".equals(sender)
                    || row.previewText.startsWith("我:")
                    || isInferredPhoneOutgoing;
                String msgSender = outgoing ? "我" : (!sender.isEmpty() ? sender : cleanRowTitle);
                MessageRecord msg = new MessageRecord();
                msg.id = ContextProvenance.captureId(msgSender, body, outgoing, 0);
                msg.sender = msgSender;
                msg.text = body;
                msg.timestampMs = 0L;
                msg.observedAtMs = nowMs;
                msg.timeKnown = false;
                msg.outgoing = outgoing;
                msg.deliveryStatus = outgoing ? "sent" : "observed";
                msg.via = "ui_list_preview";
                msg.clientMsgId = "";
                msg.recalled = body.contains("撤回了一条消息") || body.contains("recalled a message");
                existing.messages.add(msg);
                while (existing.messages.size() > MAX_MESSAGES_PER_THREAD) {
                    existing.messages.remove(0);
                    historyTruncated = true;
                }
                existing.lastSender = msgSender;
                existing.lastMessage = outgoing && !row.previewText.startsWith("我:")
                    ? ("我: " + body)
                    : (!row.previewText.isEmpty() ? row.previewText : body);
                anyChanged = true;
            }
        }

        // 4. Prune conversations for this package that were deleted on the phone (absent from visible LauncherUI list)
        if (validRowCount >= 1) {
            List<ThreadRecord> snapshotThreads = new ArrayList<ThreadRecord>(threadsById.values());
            for (ThreadRecord t : snapshotThreads) {
                if (t == null || !safePkg.equals(t.packageName)) continue;
                String tClean = ImAppCatalog.cleanConversationTitle(safePkg, t.title);
                if (visibleThreadIds.contains(t.threadId) || visibleCleanTitles.contains(tClean)) {
                    continue;
                }
                if (isServiceNotificationContainerTitle(t.title) || ImAppCatalog.isGenericPlaceholderTitle(t.title)) {
                    continue;
                }
                long effectiveLastUpdateMs = t.lastTimeMs;
                if (effectiveLastUpdateMs <= 0L && t.messages != null) {
                    for (MessageRecord m : t.messages) {
                        if (m != null) {
                            effectiveLastUpdateMs = Math.max(effectiveLastUpdateMs, Math.max(m.timestampMs, m.observedAtMs));
                        }
                    }
                }
                boolean recentLocalActivity = effectiveLastUpdateMs > 0L
                    && (nowMs - effectiveLastUpdateMs) >= 0L
                    && (nowMs - effectiveLastUpdateMs) <= 15_000L;
                if (recentLocalActivity) {
                    continue;
                }
                boolean unTimedStale = atTopOfList && t.lastTimeMs <= 0L
                    && (effectiveLastUpdateMs <= 0L
                        || (nowMs - effectiveLastUpdateMs) < 0L
                        || (nowMs - effectiveLastUpdateMs) > 60_000L);
                boolean shouldPrune = (atTopOfList && validRowCount < 8)
                    || unTimedStale
                    || (minVisibleLastTimeMs != Long.MAX_VALUE && t.lastTimeMs >= minVisibleLastTimeMs);
                if (shouldPrune) {
                    threadsById.remove(t.threadId);
                    deletedThreads.put(t.threadId, nowMs);
                    if (!tClean.isEmpty()) {
                        deletedThreads.put(ImAppCatalog.buildThreadId(safePkg, tClean), nowMs);
                    }
                    for (NotificationRecord n : notifications) {
                        if (n != null && safePkg.equals(n.packageName)
                                && (t.threadId.equals(n.threadId)
                                    || titlesReferToSameConversation(safePkg, n.title, t.title))) {
                            if (n.active || !n.dismissed || !n.read) {
                                n.active = false;
                                n.dismissed = true;
                                n.read = true;
                                n.cachedJson = null;
                                feedDirty = true;
                            }
                        }
                    }
                    anyChanged = true;
                }
            }
        }

        if (anyChanged) {
            lastObservedMs = nowMs;
            trimThreadsCapacity();
            threadsDirty = true;
            revision++;
            saveToDisk();
        }
        return upgradedThreads;
    }

    private ThreadRecord findMatchingGenericThreadToUpgrade(
            String safePkg,
            String targetThreadId,
            String cleanTitle,
            boolean isGroup,
            List<CapturedBubble> bubbles,
            String aliasTitleOrThreadId) {
        if (ImAppCatalog.isGenericPlaceholderTitle(cleanTitle)
                || isServiceNotificationContainerTitle(cleanTitle)) {
            return null;
        }
        if (aliasTitleOrThreadId != null && !aliasTitleOrThreadId.trim().isEmpty()) {
            String hint = aliasTitleOrThreadId.trim();
            String hintThreadId = hint.contains("::")
                ? hint
                : ImAppCatalog.buildThreadId(safePkg, ImAppCatalog.cleanConversationTitle(safePkg, hint));
            ThreadRecord hinted = threadsById.get(hintThreadId);
            if (hinted != null && !hinted.threadId.equals(targetThreadId)
                    && ImAppCatalog.isGenericPlaceholderTitle(hinted.title)) {
                return hinted;
            }
        }

        String genericGroupThreadId = ImAppCatalog.buildThreadId(safePkg, "群聊");
        ThreadRecord genericGroup = threadsById.get(genericGroupThreadId);
        if (genericGroup != null && !genericGroup.threadId.equals(targetThreadId)) {
            if (bubbles != null && !bubbles.isEmpty()) {
                for (CapturedBubble b : bubbles) {
                    if (b != null && b.text != null && threadContainsMessageBody(genericGroup, b.text.trim())) {
                        return genericGroup;
                    }
                }
            }
            if (isGroup && countGenericGroupThreads(safePkg) == 1 && !threadsById.containsKey(targetThreadId)) {
                return genericGroup;
            }
        }
        return null;
    }

    private int countGenericGroupThreads(String safePkg) {
        int count = 0;
        for (ThreadRecord t : threadsById.values()) {
            if (safePkg.equals(t.packageName) && ImAppCatalog.isGenericPlaceholderTitle(t.title)) {
                count++;
            }
        }
        return count;
    }

    private static boolean threadContainsMessageBody(ThreadRecord thread, String body) {
        if (thread == null || body == null || body.trim().isEmpty()) return false;
        String needle = body.trim();
        for (MessageRecord m : thread.messages) {
            if (m == null || m.text == null) continue;
            String existing = m.text.trim();
            if (needle.equals(existing)
                    || existing.endsWith(": " + needle)
                    || existing.endsWith("：" + needle)
                    || needle.endsWith(": " + existing)
                    || needle.endsWith("：" + existing)) {
                return true;
            }
        }
        return false;
    }

    private static void mergeSourceThreadIntoTarget(ThreadRecord source, ThreadRecord target) {
        if (source == null || target == null) return;
        target.isGroup = target.isGroup || source.isGroup;
        target.hasContentIntent = target.hasContentIntent || source.hasContentIntent;
        if (target.lastNotificationKey == null || target.lastNotificationKey.isEmpty()) {
            target.lastNotificationKey = source.lastNotificationKey;
        }
        if ((target.avatarB64 == null || target.avatarB64.isEmpty())
                && source.avatarB64 != null && !source.avatarB64.isEmpty()) {
            target.avatarB64 = source.avatarB64;
        }
        if (!source.senderAvatars.isEmpty()) {
            target.senderAvatars.putAll(source.senderAvatars);
        }
        target.lastTimeMs = Math.max(target.lastTimeMs, source.lastTimeMs);
        List<MessageRecord> merged = new ArrayList<MessageRecord>(source.messages);
        for (MessageRecord tm : target.messages) {
            boolean dup = false;
            for (MessageRecord sm : merged) {
                if (sm.outgoing == tm.outgoing && sm.text != null && sm.text.trim().equals(tm.text == null ? "" : tm.text.trim())) {
                    if (!tm.outgoing && ImAppCatalog.isGenericSenderPlaceholder(sm.sender) && !ImAppCatalog.isGenericSenderPlaceholder(tm.sender)) {
                        sm.sender = tm.sender;
                        sm.cachedJson = null;
                    }
                    dup = true;
                    break;
                }
            }
            if (!dup) {
                merged.add(tm);
            }
        }
        target.messages.clear();
        int start = Math.max(0, merged.size() - MAX_MESSAGES_PER_THREAD);
        for (int i = start; i < merged.size(); i++) {
            target.messages.add(merged.get(i));
        }
    }

    private static String buildEnrichedGenericGroupTitle(String rawTitle, List<MessageRecord> messages) {
        int memberCount = ImAppCatalog.extractGroupMemberCount(rawTitle);
        List<String> senders = new ArrayList<String>();
        if (messages != null) {
            for (MessageRecord m : messages) {
                if (m == null || m.outgoing || m.sender == null) continue;
                String s = m.sender.trim();
                if (s.isEmpty() || "我".equals(s) || ImAppCatalog.isGenericSenderPlaceholder(s)) continue;
                if (!senders.contains(s)) {
                    senders.add(s);
                }
            }
        }
        String base = memberCount >= 2 ? ("群聊(" + memberCount + ")") : "群聊";
        if (senders.isEmpty()) {
            return base;
        }
        StringBuilder sb = new StringBuilder(base).append(" · ").append(senders.get(0));
        if (senders.size() >= 2) {
            sb.append("、").append(senders.get(1));
        }
        if (senders.size() > 2) {
            sb.append("等");
        }
        return sb.toString();
    }

    private boolean upgradeNotificationsForThread(
            String safePkg,
            String migratedFromThreadId,
            ThreadRecord thread) {
        if (thread == null) return false;
        boolean anyChanged = false;
        String fallbackSender = findSingleKnownIncomingSender(thread.messages);
        for (NotificationRecord n : notifications) {
            if (n == null || !safePkg.equals(n.packageName)) continue;
            boolean sameThread = thread.threadId.equals(n.threadId)
                || (migratedFromThreadId != null && migratedFromThreadId.equals(n.threadId));
            String notifBody = extractNotificationPlainBody(n.text);
            MessageRecord matchedMsg = findMatchingIncomingMessage(thread.messages, notifBody);
            if (!sameThread && matchedMsg != null && ImAppCatalog.isGenericPlaceholderTitle(n.title)) {
                sameThread = true;
            }
            if (!sameThread) continue;

            boolean recChanged = false;
            if (!thread.threadId.equals(n.threadId)) {
                n.threadId = thread.threadId;
                recChanged = true;
            }
            if (thread.title != null && !thread.title.isEmpty()
                    && !thread.title.equals(n.title)
                    && (ImAppCatalog.isGenericPlaceholderTitle(n.title) || "群聊".equals(n.title))) {
                n.title = thread.title;
                recChanged = true;
            }
            if (thread.isGroup && !n.isGroup) {
                n.isGroup = true;
                recChanged = true;
            }
            String realSender = (matchedMsg != null && !ImAppCatalog.isGenericSenderPlaceholder(matchedMsg.sender))
                ? matchedMsg.sender
                : fallbackSender;
            if (realSender != null && !realSender.isEmpty() && !ImAppCatalog.isGenericSenderPlaceholder(realSender)) {
                if (ImAppCatalog.isGenericSenderPlaceholder(n.sender)) {
                    n.sender = realSender;
                    recChanged = true;
                }
                if (n.text != null && (n.text.startsWith("微信用户:") || n.text.startsWith("微信用户："))) {
                    n.text = realSender + ": " + notifBody;
                    recChanged = true;
                }
            }
            if (recChanged) {
                n.cachedJson = null;
                feedDirty = true;
                anyChanged = true;
            }
        }
        return anyChanged;
    }

    private static String extractNotificationPlainBody(String rawText) {
        if (rawText == null) return "";
        String s = rawText.trim();
        s = s.replaceFirst("^\\[\\d+条\\]\\s*", "");
        int colon = s.indexOf(':');
        int fullColon = s.indexOf('：');
        int idx = -1;
        if (colon > 0 && fullColon > 0) idx = Math.min(colon, fullColon);
        else if (colon > 0) idx = colon;
        else if (fullColon > 0) idx = fullColon;
        if (idx > 0 && idx <= 24 && idx + 1 < s.length()) {
            return s.substring(idx + 1).trim();
        }
        return s;
    }

    private static MessageRecord findMatchingIncomingMessage(List<MessageRecord> messages, String plainBody) {
        if (messages == null || plainBody == null || plainBody.isEmpty()) return null;
        for (int i = messages.size() - 1; i >= 0; i--) {
            MessageRecord m = messages.get(i);
            if (m != null && !m.outgoing && m.text != null && plainBody.equals(m.text.trim())) {
                return m;
            }
        }
        return null;
    }

    public synchronized List<String> getActiveDismissableKeys(String categoryFilter) {
        List<String> keys = new ArrayList<String>();
        String cat = categoryFilter == null ? "" : categoryFilter.trim();
        for (NotificationRecord n : notifications) {
            if (!n.active || n.ongoing) continue;
            if (!cat.isEmpty() && !"all".equalsIgnoreCase(cat) && !cat.equalsIgnoreCase(n.category)) {
                continue;
            }
            if (n.key != null && !n.key.isEmpty() && !keys.contains(n.key)) {
                keys.add(n.key);
            }
        }
        return keys;
    }

    public synchronized JSONObject openPush(Context context, String keyOrId, String fallbackPkg) throws Exception {
        String targetKey = keyOrId == null ? "" : keyOrId.trim();
        String resolvedPkg = fallbackPkg == null ? "" : fallbackPkg.trim();
        long nowMs = System.currentTimeMillis();

        for (NotificationRecord n : notifications) {
            if (targetKey.equals(n.key) || targetKey.equals(n.eventId)) {
                targetKey = n.key != null && !n.key.isEmpty() ? n.key : targetKey;
                if (resolvedPkg.isEmpty() && n.packageName != null) {
                    resolvedPkg = n.packageName;
                }
                n.read = true;
                n.cachedJson = null;
                feedDirty = true;
                if (n.threadId != null && threadsById.containsKey(n.threadId)) {
                    threadsById.get(n.threadId).unreadCount = 0;
                    threadsDirty = true;
                }
                break;
            }
        }

        boolean openedViaPendingIntent = false;
        PendingIntent pi = contentIntentsByKey.get(targetKey);
        if (pi == null) {
            pi = contentIntentsByThread.get(targetKey);
        }
        if (pi == null && resolvedGenericThreadAliases.containsKey(targetKey)) {
            pi = contentIntentsByThread.get(resolvedGenericThreadAliases.get(targetKey));
        }
        if (pi == null && !resolvedPkg.isEmpty() && !targetKey.isEmpty()) {
            String directTid = ImAppCatalog.buildThreadId(resolvedPkg, targetKey);
            pi = contentIntentsByThread.get(directTid);
            if (pi == null && resolvedGenericThreadAliases.containsKey(directTid)) {
                pi = contentIntentsByThread.get(resolvedGenericThreadAliases.get(directTid));
            }
            if (pi == null) {
                String cleanTid = ImAppCatalog.buildThreadId(
                    resolvedPkg,
                    ImAppCatalog.cleanConversationTitle(resolvedPkg, targetKey)
                );
                pi = contentIntentsByThread.get(cleanTid);
                if (pi == null && resolvedGenericThreadAliases.containsKey(cleanTid)) {
                    pi = contentIntentsByThread.get(resolvedGenericThreadAliases.get(cleanTid));
                }
            }
        }
        if (pi == null) {
            ThreadRecord matchedThread = getThreadRecord(
                (!resolvedPkg.isEmpty() && !targetKey.contains("::"))
                    ? ImAppCatalog.buildThreadId(resolvedPkg, targetKey)
                    : targetKey
            );
            if (matchedThread != null) {
                if (resolvedPkg.isEmpty() && matchedThread.packageName != null) {
                    resolvedPkg = matchedThread.packageName;
                }
                matchedThread.unreadCount = 0;
                threadsDirty = true;
                pi = contentIntentsByThread.get(matchedThread.threadId);
                if (pi == null && matchedThread.lastNotificationKey != null
                        && !matchedThread.lastNotificationKey.isEmpty()) {
                    pi = contentIntentsByKey.get(matchedThread.lastNotificationKey);
                }
            }
        }
        if (pi == null && handlesByKey.containsKey(targetKey)) {
            pi = handlesByKey.get(targetKey).contentIntent;
        }
        if (pi == null) {
            RemoteReplyHandle rh = getReplyHandle(targetKey);
            if (rh == null && !resolvedPkg.isEmpty() && !targetKey.contains("::")) {
                rh = getReplyHandle(ImAppCatalog.buildThreadId(resolvedPkg, targetKey));
            }
            if (rh != null && rh.contentIntent != null) {
                pi = rh.contentIntent;
            }
        }
        if (pi != null) {
            try {
                Bundle optionsBundle = buildBackgroundActivityLaunchOptions();
                if (context != null && optionsBundle != null) {
                    pi.send(context, 0, null, null, null, null, optionsBundle);
                } else {
                    pi.send();
                }
                openedViaPendingIntent = true;
            } catch (PendingIntent.CanceledException ignored) {
                contentIntentsByKey.remove(targetKey);
                contentIntentsByThread.remove(targetKey);
            } catch (Throwable fallbackErr) {
                try {
                    pi.send();
                    openedViaPendingIntent = true;
                } catch (PendingIntent.CanceledException ignored) {
                    contentIntentsByKey.remove(targetKey);
                    contentIntentsByThread.remove(targetKey);
                }
            }
        }

        if (!openedViaPendingIntent && context != null && !resolvedPkg.isEmpty()) {
            Intent launch = context.getPackageManager().getLaunchIntentForPackage(resolvedPkg);
            if (launch != null) {
                launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                Bundle optionsBundle = buildBackgroundActivityLaunchOptions();
                if (optionsBundle != null) {
                    context.startActivity(launch, optionsBundle);
                } else {
                    context.startActivity(launch);
                }
            }
        }

        revision++;
        saveToDisk();
        return new JSONObject()
            .put("ok", true)
            .put("key", targetKey)
            .put("packageName", resolvedPkg)
            .put("openedViaContentIntent", openedViaPendingIntent)
            .put("openedAtMs", nowMs)
            .put("revision", revision);
    }

    static Bundle buildBackgroundActivityLaunchOptions() {
        try {
            android.app.ActivityOptions options = android.app.ActivityOptions.makeBasic();
            try {
                java.lang.reflect.Method m = android.app.ActivityOptions.class.getMethod(
                    "setPendingIntentBackgroundActivityStartMode", int.class
                );
                m.invoke(options, 1); // MODE_BACKGROUND_ACTIVITY_START_ALLOWED = 1
            } catch (Throwable ignored) {
                try {
                    java.lang.reflect.Method mLegacy = android.app.ActivityOptions.class.getMethod(
                        "setPendingIntentBackgroundActivityLaunchAllowed", boolean.class
                    );
                    mLegacy.invoke(options, true);
                } catch (Throwable ignored2) {
                }
            }
            return options.toBundle();
        } catch (Throwable ignored) {
            return null;
        }
    }

    public synchronized boolean isUnchangedSnapshot(long sinceRevision) {
        return sinceRevision > 0 && sinceRevision == revision;
    }

    private boolean pruneLocked(long nowMs) {
        boolean modified = false;
        java.util.Iterator<Map.Entry<String, ThreadRecord>> it = threadsById.entrySet().iterator();
        while (it.hasNext()) {
            Map.Entry<String, ThreadRecord> entry = it.next();
            if (isNativeSmsThread(entry.getValue())) {
                it.remove();
                threadsDirty = true;
                modified = true;
            }
        }
        if (cachedSnapshotObj != null) {
            JSONArray cachedBanners = cachedSnapshotObj.optJSONArray("otpBanners");
            if (cachedBanners != null) {
                for (int i = 0; i < cachedBanners.length(); i++) {
                    JSONObject b = cachedBanners.optJSONObject(i);
                    if (b == null) continue;
                    long postTs = b.optLong("postTimeMs", 0L);
                    String bKey = b.optString("key", "");
                    boolean expired = nowMs > 0L && postTs > 0L && (nowMs - postTs) > OTP_TTL_MS;
                    boolean inactive = false;
                    if (!bKey.isEmpty()) {
                        for (int j = 0; j < notifications.size(); j++) {
                            NotificationRecord n = notifications.get(j);
                            if (n != null && bKey.equals(n.key)) {
                                if (!n.active || n.dismissed || n.read) {
                                    inactive = true;
                                }
                                break;
                            }
                        }
                    }
                    if (expired || inactive) {
                        cachedSnapshotRevision = -1L;
                        cachedSnapshotObj = null;
                        cachedSnapshotJson = null;
                        break;
                    }
                }
            }
        }
        if (modified) {
            revision++;
            saveToDisk();
        }
        return modified;
    }

    public synchronized JSONObject snapshot(long sinceRevision) throws JSONException {
        long now = System.currentTimeMillis();
        pruneLocked(now);
        boolean unchanged = isUnchangedSnapshot(sinceRevision);
        if (unchanged) {
            int unreadImCount = 0;
            int activePushCount = 0;
            if (cachedSnapshotRevision == revision && cachedSnapshotObj != null) {
                unreadImCount = cachedSnapshotObj.optInt("unreadImCount", 0);
                activePushCount = cachedSnapshotObj.optInt("activePushCount", 0);
            } else {
                for (ThreadRecord t : threadsById.values()) {
                    unreadImCount += t.unreadCount;
                }
                for (NotificationRecord n : notifications) {
                    if (n.active && !n.ongoing) {
                        activePushCount++;
                    }
                }
            }
            return new JSONObject()
                .put("ok", true)
                .put("revision", revision)
                .put("coverage", coverageJson())
                .put("unchanged", true)
                .put("threads", new JSONArray())
                .put("notifications", new JSONArray())
                .put("otpBanners", new JSONArray())
                .put("unreadImCount", unreadImCount)
                .put("activePushCount", activePushCount)
                .put("totalPushCount", notifications.size());
        }
        if (cachedSnapshotRevision == revision && cachedSnapshotObj != null) {
            JSONObject copy = new JSONObject();
            copy.put("ok", true);
            copy.put("revision", revision);
            copy.put("coverage", coverageJson());
            copy.put("unchanged", unchanged);
            copy.put("threads", cachedSnapshotObj.optJSONArray("threads"));
            copy.put("notifications", cachedSnapshotObj.optJSONArray("notifications"));
            copy.put("otpBanners", cachedSnapshotObj.optJSONArray("otpBanners"));
            copy.put("unreadImCount", cachedSnapshotObj.optInt("unreadImCount", 0));
            copy.put("activePushCount", cachedSnapshotObj.optInt("activePushCount", 0));
            copy.put("totalPushCount", cachedSnapshotObj.optInt("totalPushCount", 0));
            return copy;
        }

        JSONObject out = new JSONObject();
        out.put("ok", true);
        out.put("revision", revision);
        out.put("coverage", coverageJson());
        out.put("unchanged", unchanged);

        int[] remainingAvatarChars = new int[] { 220_000 };
        List<ThreadRecord> sortedThreads = getSortedThreadsList();
        JSONArray threadsArr = new JSONArray();
        int unreadImCount = 0;
        for (ThreadRecord t : sortedThreads) {
            if (isNativeSmsThread(t)) continue;
            t.canReply = hasReplyHandle(t.threadId);
            unreadImCount += t.unreadCount;
            boolean allowSenderAvatars = threadsArr.length() < 15 && remainingAvatarChars[0] > 40_000;
            threadsArr.put(t.toSnapshotJson(remainingAvatarChars, allowSenderAvatars));
        }

        JSONArray feedArr = new JSONArray();
        JSONArray otpBannersArr = new JSONArray();
        int activePushCount = 0;
        int inactiveIncluded = 0;
        for (NotificationRecord n : notifications) {
            if (n.active && !n.ongoing) {
                activePushCount++;
            }
            boolean nextCanReply = (n.threadId != null && hasReplyHandle(n.threadId))
                || (n.key != null && !n.key.isEmpty() && handlesByKey.containsKey(n.key));
            if (n.canReply != nextCanReply) {
                n.canReply = nextCanReply;
                n.cachedJson = null;
            }
            if (feedArr.length() < 80 && (n.active || inactiveIncluded < 60)) {
                if (!n.active) {
                    inactiveIncluded++;
                }
                boolean includeNotifAvatar = n.active
                    && feedArr.length() < 20
                    && n.avatarB64 != null
                    && !n.avatarB64.isEmpty()
                    && remainingAvatarChars[0] >= n.avatarB64.length();
                if (includeNotifAvatar) {
                    remainingAvatarChars[0] -= n.avatarB64.length();
                }
                feedArr.put(n.toSnapshotJson(includeNotifAvatar));
            }

            boolean withinOtpTtl = n.postTimeMs <= 0L || now <= 0L || (now - n.postTimeMs) <= OTP_TTL_MS;
            if (n.otpCode != null && !n.otpCode.isEmpty()
                    && n.active && !n.dismissed && !n.read
                    && withinOtpTtl
                    && otpBannersArr.length() < 10) {
                JSONObject banner = new JSONObject()
                    .put("code", n.otpCode)
                    .put("appName", n.appName)
                    .put("title", n.title)
                    .put("text", n.text)
                    .put("postTimeMs", n.postTimeMs)
                    .put("expiresAtMs", n.postTimeMs > 0L ? (n.postTimeMs + OTP_TTL_MS) : 0L)
                    .put("key", n.key);
                otpBannersArr.put(banner);
            }
        }

        out.put("threads", threadsArr);
        out.put("notifications", feedArr);
        out.put("otpBanners", otpBannersArr);
        out.put("unreadImCount", unreadImCount);
        out.put("activePushCount", activePushCount);
        out.put("totalPushCount", notifications.size());
        String serialized = out.toString();
        if (serialized.length() > 350_000) {
            for (int i = 0; i < threadsArr.length(); i++) {
                JSONObject tj = threadsArr.optJSONObject(i);
                if (tj != null) {
                    tj.remove("sender_avatars");
                }
            }
            for (int i = 0; i < feedArr.length(); i++) {
                JSONObject nj = feedArr.optJSONObject(i);
                if (nj != null) {
                    nj.remove("avatar_b64");
                }
            }
            serialized = out.toString();
            if (serialized.length() > 350_000) {
                for (int i = threadsArr.length() - 1; i >= 8; i--) {
                    JSONObject tj = threadsArr.optJSONObject(i);
                    if (tj != null) {
                        tj.remove("avatar_b64");
                    }
                }
                serialized = out.toString();
            }
        }
        cachedSnapshotRevision = revision;
        cachedSnapshotObj = out;
        cachedSnapshotJson = serialized;

        JSONObject copy = new JSONObject();
        copy.put("ok", true);
        copy.put("revision", revision);
        copy.put("coverage", coverageJson());
        copy.put("unchanged", unchanged);
        copy.put("threads", threadsArr);
        copy.put("notifications", feedArr);
        copy.put("otpBanners", otpBannersArr);
        copy.put("unreadImCount", unreadImCount);
        copy.put("activePushCount", activePushCount);
        copy.put("totalPushCount", notifications.size());
        return copy;
    }

    public synchronized JSONObject threadDetail(String threadId) throws JSONException {
        if (threadId == null || threadId.isEmpty()) {
            throw new IllegalArgumentException("缺少 threadId");
        }
        ThreadRecord thread = getThreadRecord(threadId);
        if (thread == null) {
            throw new IllegalStateException("未找到对应会话: " + threadId);
        }
        thread.canReply = hasReplyHandle(thread.threadId);
        JSONObject threadJson = thread.toJson();

        JSONObject out = new JSONObject();
        out.put("ok", true);
        out.put("revision", revision);
        out.put("threadId", thread.threadId);
        out.put("thread", threadJson);
        out.put("messages", threadJson.optJSONArray("messages"));
        return out;
    }

    public synchronized int deleteThreadRecord(
            Context context,
            String threadId,
            String packageName,
            String rawTitle) {
        String safeThreadId = threadId == null ? "" : threadId.trim();
        String safePkg = packageName == null ? "" : packageName.trim();
        String safeTitle = rawTitle == null ? "" : rawTitle.trim();
        if (safePkg.isEmpty() && safeThreadId.contains("::")) {
            safePkg = safeThreadId.substring(0, safeThreadId.indexOf("::")).trim();
        }
        if (safeTitle.isEmpty() && safeThreadId.contains("::")) {
            safeTitle = safeThreadId.substring(safeThreadId.indexOf("::") + 2).trim();
        }
        String canonicalKey = canonicalThreadKey(safeThreadId, safePkg, safeTitle);
        long nowMs = System.currentTimeMillis();

        if (!safeThreadId.isEmpty()) {
            deletedThreads.put(safeThreadId, nowMs);
        }
        if (!canonicalKey.isEmpty()) {
            deletedThreads.put(canonicalKey, nowMs);
        }

        List<String> toRemove = new ArrayList<String>();
        ThreadRecord direct = getThreadRecord(!safeThreadId.isEmpty() ? safeThreadId : canonicalKey);
        if (direct != null) {
            toRemove.add(direct.threadId);
            if (safePkg.isEmpty()) safePkg = direct.packageName;
            if (safeTitle.isEmpty()) safeTitle = direct.title;
        }
        for (ThreadRecord t : threadsById.values()) {
            if (t == null) continue;
            if (toRemove.contains(t.threadId)) continue;
            if ((!safeThreadId.isEmpty() && safeThreadId.equals(t.threadId))
                    || (!canonicalKey.isEmpty() && canonicalKey.equals(t.threadId))
                    || (!safePkg.isEmpty() && safePkg.equals(t.packageName)
                        && !safeTitle.isEmpty() && titlesReferToSameConversation(safePkg, t.title, safeTitle))) {
                toRemove.add(t.threadId);
            }
        }

        for (String tid : toRemove) {
            threadsById.remove(tid);
            handlesByThread.remove(tid);
            contentIntentsByThread.remove(tid);
            deletedThreads.put(tid, nowMs);
        }

        List<String> aliasKeysToRemove = new ArrayList<String>();
        for (Map.Entry<String, String> entry : resolvedGenericThreadAliases.entrySet()) {
            if (toRemove.contains(entry.getKey()) || toRemove.contains(entry.getValue())
                    || safeThreadId.equals(entry.getKey()) || canonicalKey.equals(entry.getValue())) {
                aliasKeysToRemove.add(entry.getKey());
            }
        }
        for (String k : aliasKeysToRemove) {
            resolvedGenericThreadAliases.remove(k);
        }

        for (NotificationRecord n : notifications) {
            if (n == null) continue;
            boolean match = (n.threadId != null && (toRemove.contains(n.threadId)
                    || safeThreadId.equals(n.threadId)
                    || canonicalKey.equals(n.threadId)))
                || (!safePkg.isEmpty() && safePkg.equals(n.packageName)
                    && !safeTitle.isEmpty() && titlesReferToSameConversation(safePkg, n.title, safeTitle));
            if (match) {
                if (n.active && n.key != null && !n.key.isEmpty()) {
                    try {
                        PushNotificationService.dismissNotification(context, n.key);
                    } catch (Throwable ignored) {
                    }
                }
                if (n.active || !n.dismissed || !n.read) {
                    n.active = false;
                    n.dismissed = true;
                    n.read = true;
                    n.cachedJson = null;
                    feedDirty = true;
                }
            }
        }

        threadsDirty = true;
        revision++;
        saveToDisk();
        return toRemove.size();
    }

    public synchronized JSONObject deleteThread(
            Context context,
            String threadId,
            String packageName,
            String rawTitle) throws JSONException {
        String safeThreadId = threadId == null ? "" : threadId.trim();
        String canonicalKey = canonicalThreadKey(safeThreadId, packageName, rawTitle);
        int removedCount = deleteThreadRecord(context, threadId, packageName, rawTitle);
        return new JSONObject()
            .put("ok", true)
            .put("event", "thread_deleted")
            .put("threadId", !canonicalKey.isEmpty() ? canonicalKey : safeThreadId)
            .put("removedThreads", removedCount)
            .put("revision", revision);
    }

    public synchronized ThreadRecord markThreadReadRecord(
            Context context,
            String threadId,
            String packageName,
            String rawTitle,
            boolean read) {
        String safeThreadId = threadId == null ? "" : threadId.trim();
        String safePkg = packageName == null ? "" : packageName.trim();
        String safeTitle = rawTitle == null ? "" : rawTitle.trim();
        String canonicalKey = canonicalThreadKey(safeThreadId, safePkg, safeTitle);

        ThreadRecord thread = getThreadRecord(!safeThreadId.isEmpty() ? safeThreadId : canonicalKey);
        if (thread != null) {
            thread.unreadCount = read ? 0 : Math.max(1, thread.unreadCount);
            if (safePkg.isEmpty()) safePkg = thread.packageName;
            if (safeTitle.isEmpty()) safeTitle = thread.title;
            threadsDirty = true;
        }

        for (NotificationRecord n : notifications) {
            if (n == null) continue;
            boolean match = (n.threadId != null && (safeThreadId.equals(n.threadId)
                    || canonicalKey.equals(n.threadId)
                    || (thread != null && thread.threadId.equals(n.threadId))))
                || (!safePkg.isEmpty() && safePkg.equals(n.packageName)
                    && !safeTitle.isEmpty() && titlesReferToSameConversation(safePkg, n.title, safeTitle));
            if (match) {
                if (read) {
                    if (n.active && n.key != null && !n.key.isEmpty()) {
                        try {
                            PushNotificationService.dismissNotification(context, n.key);
                        } catch (Throwable ignored) {
                        }
                    }
                    n.read = true;
                    n.active = false;
                    n.dismissed = true;
                } else {
                    n.read = false;
                }
                n.cachedJson = null;
                feedDirty = true;
            }
        }

        revision++;
        saveToDisk();
        return thread;
    }

    public synchronized JSONObject markThreadRead(
            Context context,
            String threadId,
            String packageName,
            String rawTitle,
            boolean read) throws JSONException {
        String safeThreadId = threadId == null ? "" : threadId.trim();
        String canonicalKey = canonicalThreadKey(safeThreadId, packageName, rawTitle);
        ThreadRecord thread = markThreadReadRecord(context, threadId, packageName, rawTitle, read);
        JSONObject res = new JSONObject()
            .put("ok", true)
            .put("event", "thread_read_changed")
            .put("threadId", thread != null ? thread.threadId : (!canonicalKey.isEmpty() ? canonicalKey : safeThreadId))
            .put("read", read)
            .put("unreadCount", thread != null ? thread.unreadCount : (read ? 0 : 1))
            .put("revision", revision);
        if (thread != null) {
            res.put("thread", thread.toJson());
        }
        return res;
    }

    public synchronized ThreadRecord clearThreadMessagesRecord(
            String threadId,
            String packageName,
            String rawTitle) {
        String safeThreadId = threadId == null ? "" : threadId.trim();
        String safePkg = packageName == null ? "" : packageName.trim();
        String safeTitle = rawTitle == null ? "" : rawTitle.trim();
        String canonicalKey = canonicalThreadKey(safeThreadId, safePkg, safeTitle);
        long nowMs = System.currentTimeMillis();

        if (!safeThreadId.isEmpty()) {
            clearedThreads.put(safeThreadId, nowMs);
        }
        if (!canonicalKey.isEmpty()) {
            clearedThreads.put(canonicalKey, nowMs);
        }

        ThreadRecord thread = getThreadRecord(!safeThreadId.isEmpty() ? safeThreadId : canonicalKey);
        if (thread != null) {
            clearedThreads.put(thread.threadId, nowMs);
            thread.messages.clear();
            thread.lastMessage = "";
            thread.lastSender = "";
            thread.unreadCount = 0;
            threadsDirty = true;
        }

        for (NotificationRecord n : notifications) {
            if (n == null) continue;
            boolean match = (n.threadId != null && (safeThreadId.equals(n.threadId)
                    || canonicalKey.equals(n.threadId)
                    || (thread != null && thread.threadId.equals(n.threadId))));
            if (match && (n.active || !n.dismissed || !n.read)) {
                n.active = false;
                n.dismissed = true;
                n.read = true;
                n.cachedJson = null;
                feedDirty = true;
            }
        }

        threadsDirty = true;
        revision++;
        saveToDisk();
        return thread;
    }

    public synchronized JSONObject clearThreadMessages(
            String threadId,
            String packageName,
            String rawTitle) throws JSONException {
        String safeThreadId = threadId == null ? "" : threadId.trim();
        String canonicalKey = canonicalThreadKey(safeThreadId, packageName, rawTitle);
        ThreadRecord thread = clearThreadMessagesRecord(threadId, packageName, rawTitle);
        JSONObject res = new JSONObject()
            .put("ok", true)
            .put("event", "thread_cleared")
            .put("threadId", thread != null ? thread.threadId : (!canonicalKey.isEmpty() ? canonicalKey : safeThreadId))
            .put("revision", revision);
        if (thread != null) {
            res.put("thread", thread.toJson());
        }
        return res;
    }

    public synchronized int deleteMessageRecord(
            String threadId,
            String messageId,
            String text) {
        String safeThreadId = threadId == null ? "" : threadId.trim();
        String safeMsgId = messageId == null ? "" : messageId.trim();
        String safeText = text == null ? "" : text.trim();

        if (!safeMsgId.isEmpty()) {
            deletedMessages.add(safeMsgId);
        }
        ThreadRecord thread = getThreadRecord(safeThreadId);
        String resolvedTid = thread != null ? thread.threadId : safeThreadId;
        if (!resolvedTid.isEmpty() && !safeText.isEmpty()) {
            deletedMessages.add(resolvedTid + "::" + safeText);
        }

        int removed = 0;
        if (thread != null) {
            for (int i = thread.messages.size() - 1; i >= 0; i--) {
                MessageRecord m = thread.messages.get(i);
                if (m == null) continue;
                String mText = m.text == null ? "" : m.text.trim();
                boolean idMatch = !safeMsgId.isEmpty()
                    && (safeMsgId.equals(m.id) || safeMsgId.equals(m.clientMsgId));
                boolean textMatch = safeMsgId.isEmpty() && !safeText.isEmpty() && safeText.equals(mText);
                if (idMatch || textMatch) {
                    if (m.id != null && !m.id.isEmpty()) {
                        deletedMessages.add(m.id);
                    }
                    thread.messages.remove(i);
                    removed++;
                    break;
                }
            }
            if (thread.messages.isEmpty()) {
                thread.lastSender = "";
                thread.lastMessage = "";
            } else {
                MessageRecord tail = thread.messages.get(thread.messages.size() - 1);
                thread.lastSender = tail.outgoing ? "我" : (tail.sender == null ? "" : tail.sender);
                if (tail.outgoing) {
                    thread.lastMessage = "我: " + tail.text;
                } else if (thread.isGroup && !thread.lastSender.isEmpty() && !thread.lastSender.equals(thread.title)) {
                    thread.lastMessage = thread.lastSender + ": " + tail.text;
                } else {
                    thread.lastMessage = tail.text;
                }
            }
            threadsDirty = true;
        }

        threadsDirty = true;
        revision++;
        saveToDisk();
        return removed;
    }

    public synchronized JSONObject deleteMessage(
            String threadId,
            String messageId,
            String text) throws JSONException {
        String safeThreadId = threadId == null ? "" : threadId.trim();
        String safeMsgId = messageId == null ? "" : messageId.trim();
        int removed = deleteMessageRecord(threadId, messageId, text);
        ThreadRecord thread = getThreadRecord(safeThreadId);
        String resolvedTid = thread != null ? thread.threadId : safeThreadId;
        JSONObject res = new JSONObject()
            .put("ok", true)
            .put("event", "message_deleted")
            .put("threadId", resolvedTid)
            .put("messageId", safeMsgId)
            .put("removed", removed)
            .put("revision", revision);
        if (thread != null) {
            res.put("thread", thread.toJson());
        }
        return res;
    }

    private List<ThreadRecord> getSortedThreadsList() {
        List<ThreadRecord> list = new ArrayList<ThreadRecord>(threadsById.values());
        Collections.sort(list, new Comparator<ThreadRecord>() {
            @Override
            public int compare(ThreadRecord a, ThreadRecord b) {
                return Long.compare(b.lastTimeMs, a.lastTimeMs);
            }
        });
        return list;
    }

    private void loadFromDisk() {
        if (storageDir == null) return;
        JSONObject[] stored;
        try {
            database = new PushDatabase(storageDir);
            stored = database.load(storageDir);
        } catch (Exception error) {
            storageLoadFailed = true; persistenceError = true; historyTruncated = true;
            android.util.Log.e("StepXPushStore", "Message database unavailable; original data retained", error);
            return;
        }
        try {
            {
                JSONObject root = stored[0];
                loadCoverage(root);
                revision = Math.max(revision, root.optLong("revision", 1L));
                JSONObject delObj = root.optJSONObject("deletedThreads");
                if (delObj != null) {
                    java.util.Iterator<String> keys = delObj.keys();
                    while (keys.hasNext()) {
                        String k = keys.next();
                        long v = delObj.optLong(k, 0L);
                        if (k != null && !k.isEmpty() && v > 0L) {
                            deletedThreads.put(k, v);
                        }
                    }
                }
                JSONObject clrObj = root.optJSONObject("clearedThreads");
                if (clrObj != null) {
                    java.util.Iterator<String> keys = clrObj.keys();
                    while (keys.hasNext()) {
                        String k = keys.next();
                        long v = clrObj.optLong(k, 0L);
                        if (k != null && !k.isEmpty() && v > 0L) {
                            clearedThreads.put(k, v);
                        }
                    }
                }
                JSONArray delMsgArr = root.optJSONArray("deletedMessages");
                if (delMsgArr != null) {
                    for (int i = 0; i < delMsgArr.length() && i < 500; i++) {
                        String mKey = delMsgArr.optString(i, "");
                        if (!mKey.isEmpty()) {
                            deletedMessages.add(mKey);
                        }
                    }
                }
                JSONArray arr = root.optJSONArray("threads");
                if (arr != null) {
                    if (arr.length() > MAX_THREADS) historyTruncated = true;
                    for (int i = 0; i < arr.length() && i < MAX_THREADS; i++) {
                        JSONObject rawThread = arr.getJSONObject(i);
                        JSONArray rawMessages = rawThread.optJSONArray("messages");
                        if (rawMessages != null && rawMessages.length() > MAX_MESSAGES_PER_THREAD) historyTruncated = true;
                        ThreadRecord t = ThreadRecord.fromJson(rawThread);
                        if (isNativeSmsThread(t)
                                || "com.android.launcher3".equals(t.packageName)
                                || "com.android.systemui".equals(t.packageName)
                                || "系统桌面".equals(t.title)
                                || "输入密码".equals(t.title)
                                || "微信".equals(t.title)) {
                            threadsDirty = true;
                            continue;
                        }
                        if (t.messages != null) {
                            List<MessageRecord> cleanedMsgs = new ArrayList<MessageRecord>();
                            for (int mi = 0; mi < t.messages.size(); mi++) {
                                MessageRecord m = t.messages.get(mi);
                                if (m == null || m.text == null) continue;
                                if ("cap_out_sticker_1".equals(m.id) || "cap_out_sticker_2".equals(m.id)) {
                                    threadsDirty = true;
                                    continue;
                                }
                                String mt = m.text.trim();
                                if ("转文字".equals(mt) || "重发".equals(mt) || "撤回".equals(mt) || "引用".equals(mt)) {
                                    threadsDirty = true;
                                    continue;
                                }
                                String normText = ChatUiBridge.normalizeVoiceBubbleText(mt);
                                if (!normText.equals(m.text)) {
                                    m.text = normText;
                                    threadsDirty = true;
                                }
                                if (!cleanedMsgs.isEmpty()) {
                                    boolean dupOfNotification = false;
                                    boolean isTrailingListPreview = (mi == t.messages.size() - 1)
                                        && !t.isGroup
                                        && t.unreadCount == 0
                                        && ("ui_capture".equals(m.via) || "ui_list_preview".equals(m.via));
                                    if ("ui_capture".equals(m.via) && !m.timeKnown && !isTrailingListPreview) {
                                        for (int pi = 0; pi < cleanedMsgs.size(); pi++) {
                                            MessageRecord prev = cleanedMsgs.get(pi);
                                            if (prev.outgoing == m.outgoing && normText.equals(prev.text)) {
                                                dupOfNotification = true;
                                                break;
                                            }
                                        }
                                    }
                                    if (dupOfNotification) {
                                        threadsDirty = true;
                                        continue;
                                    }
                                }
                                cleanedMsgs.add(m);
                            }
                            t.messages.clear();
                            t.messages.addAll(cleanedMsgs);
                        }
                        if (!t.threadId.isEmpty() && getDeletedAtMs(t.threadId, t.packageName, t.title) <= 0L) {
                            ThreadRecord existingAlias = null;
                            for (ThreadRecord cand : threadsById.values()) {
                                if (cand != null && t.packageName.equals(cand.packageName)
                                        && titlesReferToSameConversation(t.packageName, cand.title, t.title)) {
                                    existingAlias = cand;
                                    break;
                                }
                            }
                            if (existingAlias != null) {
                                String cleanerTitle = preferCleanerConversationTitle(t.packageName, existingAlias.title, t.title);
                                String canonicalId = ImAppCatalog.buildThreadId(t.packageName, cleanerTitle);
                                threadsById.remove(existingAlias.threadId);
                                existingAlias.threadId = canonicalId;
                                existingAlias.title = cleanerTitle;
                                if (!existingAlias.isGroup && existingAlias.unreadCount == 0
                                        && existingAlias.messages.size() == 1 && t.messages.size() > 1
                                        && existingAlias.lastTimeMs > t.lastTimeMs) {
                                    MessageRecord previewMsg = existingAlias.messages.get(0);
                                    List<MessageRecord> combined = new ArrayList<MessageRecord>(t.messages);
                                    combined.add(previewMsg);
                                    existingAlias.messages.clear();
                                    existingAlias.messages.addAll(combined);
                                } else {
                                    for (MessageRecord m : t.messages) {
                                        existingAlias.messages.add(m);
                                    }
                                }
                                if (t.lastTimeMs >= existingAlias.lastTimeMs) {
                                    existingAlias.lastTimeMs = t.lastTimeMs;
                                    existingAlias.lastMessage = t.lastMessage;
                                }
                                if (!existingAlias.messages.isEmpty()) {
                                    MessageRecord lm = existingAlias.messages.get(existingAlias.messages.size() - 1);
                                    existingAlias.lastMessage = lm.text;
                                    existingAlias.lastSender = lm.outgoing ? "我" : lm.sender;
                                }
                                threadsById.put(existingAlias.threadId, existingAlias);
                                threadsDirty = true;
                            } else {
                                threadsById.put(t.threadId, t);
                            }
                        }
                    }
                    // Loading a cache must never infer authorship from read state
                    // or manufacture replies from another person's message.

                }
            }
        } catch (Throwable ignored) {
            historyTruncated = true; persistenceError = true; storageLoadFailed = true;
        }

        try {
            {
                JSONObject root = stored[1];
                loadCoverage(root);
                revision = Math.max(revision, root.optLong("revision", 1L));
                nextSeqId = Math.max(nextSeqId, root.optLong("nextSeqId", 1L));
                JSONArray arr = root.optJSONArray("notifications");
                if (arr != null) {
                    if (arr.length() > MAX_NOTIFICATIONS) historyTruncated = true;
                    for (int i = 0; i < arr.length() && i < MAX_NOTIFICATIONS; i++) {
                        JSONObject rawNotif = arr.getJSONObject(i);
                        if (rawNotif.has("avatar_b64") || rawNotif.has("avatarB64")) {
                            feedDirty = true;
                        }
                        NotificationRecord n = NotificationRecord.fromJson(rawNotif);
                        if (n != null && ImAppCatalog.isNativeSmsPackage(n.packageName) && n.threadId != null && !n.threadId.isEmpty()) {
                            n.threadId = null;
                            feedDirty = true;
                        }
                        notifications.add(n);
                    }
                }
            }
        } catch (Throwable ignored) {
            historyTruncated = true; persistenceError = true; storageLoadFailed = true;
        }
        pruneLocked(System.currentTimeMillis());
        if (threadsDirty || feedDirty) {
            flushToDiskSync();
        }
    }

    private long persistenceGeneration = 0;
    private void saveToDisk() {
        persistenceGeneration++;
        if (batchDepth > 0) return;
        if (storageDir == null) {
            persistenceError = true;
            threadsDirty = false;
            feedDirty = false;
            return;
        }
        if (!threadsDirty && !feedDirty) return;
        if (pendingSaveFuture != null && !pendingSaveFuture.isDone()) {
            return;
        }
        try {
            pendingSaveFuture = DISK_WRITER.schedule(
                new Runnable() {
                    @Override
                    public void run() {
                        flushToDiskSync();
                    }
                },
                1800L,
                TimeUnit.MILLISECONDS
            );
        } catch (Throwable ignored) {
            persistenceError = true;
        }
    }

    public void flushToDiskSync() {
        synchronized (diskCommitLock) {
            JSONObject threadsRoot, feedRoot;
            long capturedRevision;
            synchronized (this) {
                pendingSaveFuture = null;
                if (batchDepth > 0 || (!threadsDirty && !feedDirty)) return;
                if (storageDir == null || database == null || storageLoadFailed) {persistenceError = true; return;}
                capturedRevision = persistenceGeneration;
                try {
                    threadsRoot = new JSONObject().put("revision",revision).put("coverage",coverageJson());
                    JSONArray threadsArr = new JSONArray();
                    for (ThreadRecord t : getSortedThreadsList()) threadsArr.put(t.toJson());
                    threadsRoot.put("threads",threadsArr);
                    JSONObject deleted = new JSONObject(), cleared = new JSONObject();
                    for (Map.Entry<String,Long> e : deletedThreads.entrySet()) deleted.put(e.getKey(),e.getValue());
                    for (Map.Entry<String,Long> e : clearedThreads.entrySet()) cleared.put(e.getKey(),e.getValue());
                    threadsRoot.put("deletedThreads",deleted).put("clearedThreads",cleared);
                    JSONArray removed = new JSONArray(); for (String id : deletedMessages) removed.put(id);
                    threadsRoot.put("deletedMessages",removed);
                    feedRoot = new JSONObject().put("revision",revision).put("coverage",coverageJson()).put("nextSeqId",nextSeqId);
                    JSONArray feedArr = new JSONArray();
                    for (NotificationRecord n : notifications) feedArr.put(n.toJson(false));
                    feedRoot.put("notifications",feedArr);
                } catch (JSONException error) {persistenceError = true; return;}
            }
            // No PushStore monitor during SQLite transactions, fsync or checkpoint.
            boolean committed = false;
            try {database.save(threadsRoot,feedRoot,false);committed = true;}
            catch (Exception error) {android.util.Log.e("StepXPushStore","Message commit failed; will retry",error);}
            synchronized (this) {
                persistenceError = !committed;
                cachedSnapshotRevision = -1;
                if (committed && persistenceGeneration == capturedRevision) {threadsDirty = false;feedDirty = false;}
                if (threadsDirty || feedDirty) {
                    if (committed) saveToDisk();
                    else if (pendingSaveFuture == null || pendingSaveFuture.isDone()) {
                        pendingSaveFuture = DISK_WRITER.schedule(new Runnable(){public void run(){flushToDiskSync();}},30L,TimeUnit.SECONDS);
                    }
                }
            }
        }
    }
}
