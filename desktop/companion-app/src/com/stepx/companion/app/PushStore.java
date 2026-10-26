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

        public StyleMessage(String sender, String text, long timestampMs) {
            this.sender = sender == null ? "" : sender;
            this.text = text == null ? "" : text;
            this.timestampMs = timestampMs;
        }
    }

    public static final class CapturedBubble {
        public final String sender;
        public final String text;
        public final boolean outgoing;
        public final long timestampMs;

        public CapturedBubble(String sender, String text, boolean outgoing, long timestampMs) {
            this.sender = sender == null ? "" : sender;
            this.text = text == null ? "" : text;
            this.outgoing = outgoing;
            this.timestampMs = timestampMs;
        }
    }

    public static final class MessageRecord {
        public String id;
        public String sender;
        public String text;
        public long timestampMs;
        public boolean outgoing;
        public String deliveryStatus;
        public String via;
        public String clientMsgId;
        public boolean recalled;

        private transient JSONObject cachedJson;

        JSONObject toJson() throws JSONException {
            if (cachedJson != null) return cachedJson;
            JSONObject msg = new JSONObject();
            msg.put("id", id);
            msg.put("sender", sender);
            msg.put("text", text);
            msg.put("timestampMs", timestampMs);
            msg.put("outgoing", outgoing);
            msg.put("deliveryStatus", deliveryStatus);
            msg.put("via", via);
            msg.put("recalled", recalled);
            if (clientMsgId != null && !clientMsgId.isEmpty()) {
                msg.put("clientMsgId", clientMsgId);
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
            r.clientMsgId = m.optString("clientMsgId", m.optString("client_msg_id", ""));
            r.recalled = m.optBoolean("recalled", false);
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
            t.put("canUiReply", GuiAccessibilityService.instance != null);
            t.put("hasContentIntent", hasContentIntent);
            if (lastNotificationKey != null && !lastNotificationKey.isEmpty()) {
                t.put("notificationKey", lastNotificationKey);
            }
            JSONArray msgArr = new JSONArray();
            for (MessageRecord m : messages) {
                msgArr.put(m.toJson());
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
            JSONArray arr = j.optJSONArray("messages");
            if (arr != null) {
                for (int i = 0; i < arr.length() && i < MAX_MESSAGES_PER_THREAD; i++) {
                    t.messages.add(MessageRecord.fromJson(arr.getJSONObject(i)));
                }
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
            entry.put("active", active && !ongoing);
            entry.put("ongoing", ongoing);
            entry.put("isGroup", isGroup);
            entry.put("read", read);
            entry.put("dismissed", dismissed);
            entry.put("canReply", canReply);
            entry.put("hasContentIntent", hasContentIntent);
            if (otpCode != null && !otpCode.isEmpty()) {
                entry.put("otpCode", otpCode);
            }
            if (firstUrl != null && !firstUrl.isEmpty()) {
                entry.put("urlHint", firstUrl);
            }
            if (threadId != null && !threadId.isEmpty()) {
                entry.put("threadId", threadId);
            }
            cachedJson = entry;
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
            n.otpCode = j.optString("otpCode", null);
            n.firstUrl = j.optString("urlHint", j.optString("firstUrl", null));
            n.threadId = j.optString("threadId", null);
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
    private RemoteReplyDispatcher replyDispatcher = DEFAULT_DISPATCHER;
    private long revision = 1L;
    private long nextSeqId = 1L;
    private boolean threadsDirty = false;
    private boolean feedDirty = false;
    private int batchDepth = 0;
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

    // Bounded 64-entry idempotency map: clientMsgId -> ReplyOutcome
    private final Map<String, ReplyOutcome> recentClientMsgIds = new LinkedHashMap<String, ReplyOutcome>(64, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, ReplyOutcome> eldest) {
            return size() > MAX_CLIENT_MSG_IDS;
        }
    };

    private final Map<String, ThrottleState> ongoingThrottle = new HashMap<String, ThrottleState>();

    PushStore(File storageDir) {
        this.storageDir = storageDir;
        loadFromDisk();
    }

    synchronized void setReplyDispatcherForTest(RemoteReplyDispatcher dispatcher) {
        this.replyDispatcher = dispatcher == null ? DEFAULT_DISPATCHER : dispatcher;
    }

    public synchronized long getRevision() {
        return revision;
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
        return threadsById.get(threadId);
    }

    public synchronized NotificationRecord getNotificationRecordByKey(String key) {
        for (NotificationRecord n : notifications) {
            if (key != null && key.equals(n.key)) return n;
        }
        return null;
    }

    public synchronized boolean hasReplyHandle(String threadIdOrKey) {
        if (threadIdOrKey == null || threadIdOrKey.isEmpty()) return false;
        return handlesByThread.containsKey(threadIdOrKey) || handlesByKey.containsKey(threadIdOrKey);
    }

    public synchronized RemoteReplyHandle getReplyHandle(String threadIdOrKey) {
        if (threadIdOrKey == null || threadIdOrKey.isEmpty()) return null;
        RemoteReplyHandle h = handlesByThread.get(threadIdOrKey);
        return h != null ? h : handlesByKey.get(threadIdOrKey);
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
        boolean shouldCreateImThread = !isNoise && (
            ImAppCatalog.CAT_IM.equals(category)
            || canReply
            || "msg".equalsIgnoreCase(categoryHint == null ? "" : categoryHint)
            || (styleMessages != null && !styleMessages.isEmpty())
        );

        String threadId = null;
        String detectedSender = "";
        if (shouldCreateImThread && (!effectiveRawTitle.isEmpty() || !safeText.isEmpty())) {
            String fallbackTitle = effectiveRawTitle.isEmpty() ? safeAppName : effectiveRawTitle;
            threadId = ImAppCatalog.buildThreadId(safePkg, fallbackTitle);
            if (replyHandle != null) {
                RemoteReplyHandle bound = new RemoteReplyHandle(
                    replyHandle.actionIntent,
                    replyHandle.remoteInputs,
                    replyHandle.contentIntent != null ? replyHandle.contentIntent : contentIntent,
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
                contentIntent != null || (replyHandle != null && replyHandle.contentIntent != null),
                styleMessages
            );
            ThreadRecord tr = threadsById.get(threadId);
            if (tr != null) {
                detectedSender = tr.lastSender;
            }
        } else if (replyHandle != null && !safeKey.isEmpty()) {
            handlesByKey.put(safeKey, replyHandle);
        }

        boolean threadCanReply = (threadId != null && handlesByThread.containsKey(threadId))
            || (!safeKey.isEmpty() && handlesByKey.containsKey(safeKey));

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
        entry.sender = detectedSender;
        entry.text = safeText;
        entry.subText = safeSubText;
        entry.postTimeMs = nowMs;
        entry.active = true;
        entry.ongoing = ongoing;
        entry.isGroup = isGroup;
        entry.read = false;
        entry.dismissed = false;
        entry.canReply = threadCanReply;
        entry.hasContentIntent = contentIntent != null
            || (replyHandle != null && replyHandle.contentIntent != null);
        entry.otpCode = otpCode;
        entry.firstUrl = firstUrl;
        entry.threadId = threadId;

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
        NotificationRecord entry = ingestRecord(
            sbnKey, packageName, appName, categoryHint, title, conversationTitle,
            text, subText, postTimeMs, ongoing, isGroup, hasProgress,
            styleMessages, replyHandle, contentIntent
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
                        existing.cachedJson = null;
                    }
                }
            }
        }
        notifications.add(0, entry);
        while (notifications.size() > MAX_NOTIFICATIONS) {
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
            List<StyleMessage> styleMessages) {
        String cleanTitle = ImAppCatalog.cleanConversationTitle(packageName, rawTitle);
        if (cleanTitle.isEmpty()) cleanTitle = appName;

        ThreadRecord thread = threadsById.get(threadId);
        if (thread == null) {
            thread = new ThreadRecord();
            thread.threadId = threadId;
            thread.packageName = packageName;
            thread.appName = appName;
            thread.title = cleanTitle;
            thread.unreadCount = 0;
        }

        boolean isGroup = isGroupHint || thread.isGroup;
        int addedCount = 0;
        String lastSender = thread.lastSender.isEmpty() ? cleanTitle : thread.lastSender;
        String lastBody = thread.lastMessage.isEmpty() ? text : thread.lastMessage;
        long latestTs = Math.max(postTimeMs, thread.lastTimeMs);

        if (styleMessages != null && !styleMessages.isEmpty()) {
            if (styleMessages.size() > 1) {
                isGroup = isGroupHint || hasDistinctSenders(styleMessages, cleanTitle);
            }
            for (StyleMessage sm : styleMessages) {
                if (sm == null || sm.text.trim().isEmpty()) continue;
                String s = sm.sender.isEmpty() ? cleanTitle : ImAppCatalog.cleanConversationTitle(packageName, sm.sender);
                long ts = sm.timestampMs > 0 ? sm.timestampMs : postTimeMs;
                if (appendMessageIfNotDuplicate(thread.messages, s, sm.text.trim(), ts, false, "received", "notification", "")) {
                    addedCount++;
                }
                lastSender = s;
                lastBody = sm.text.trim();
                if (ts > latestTs) latestTs = ts;
            }
        } else if (!text.isEmpty()) {
            ImAppCatalog.SenderBody sb = ImAppCatalog.extractSenderAndBody(packageName, cleanTitle, text, isGroup);
            if (!sb.sender.isEmpty() && !sb.sender.equals(cleanTitle)) {
                isGroup = true;
            }
            String sender = sb.sender.isEmpty() ? cleanTitle : sb.sender;
            String body = sb.body.isEmpty() ? text : sb.body;
            if (appendMessageIfNotDuplicate(thread.messages, sender, body, postTimeMs, false, "received", "notification", "")) {
                addedCount++;
            }
            lastSender = sender;
            lastBody = body;
        }

        int unread = thread.unreadCount + Math.max(1, addedCount);
        boolean canReply = handlesByThread.containsKey(threadId);
        String preview = isGroup && !lastSender.isEmpty() && !lastSender.equals(cleanTitle)
            ? (lastSender + ": " + lastBody)
            : lastBody;

        thread.title = cleanTitle;
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

        threadsById.put(threadId, thread);
        trimThreadsCapacity();
        threadsDirty = true;
    }

    private static boolean hasDistinctSenders(List<StyleMessage> list, String cleanTitle) {
        for (StyleMessage sm : list) {
            if (sm != null && !sm.sender.isEmpty() && !sm.sender.equals(cleanTitle)) {
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
            if (m.outgoing == outgoing
                    && sender.equals(m.sender)
                    && text.equals(m.text)
                    && Math.abs(timestampMs - m.timestampMs) <= 60_000L) {
                return false;
            }
        }

        MessageRecord msg = new MessageRecord();
        msg.id = "m_" + timestampMs + "_" + Integer.toHexString((sender + ":" + text).hashCode());
        msg.sender = sender;
        msg.text = text;
        msg.timestampMs = timestampMs;
        msg.outgoing = outgoing;
        msg.deliveryStatus = deliveryStatus;
        msg.via = via;
        msg.clientMsgId = clientMsgId == null ? "" : clientMsgId;
        msg.recalled = recalled;
        messages.add(msg);

        while (messages.size() > MAX_MESSAGES_PER_THREAD) {
            messages.remove(0);
        }
        return true;
    }

    private void trimThreadsCapacity() {
        if (threadsById.size() <= MAX_THREADS) return;
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
                if (n.active || (userOrAppRead && !n.read)) {
                    n.active = false;
                    n.removedTimeMs = nowMs;
                    n.removeReason = reason;
                    if (userOrAppRead) {
                        n.read = true;
                        n.dismissed = true;
                    }
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

        // 2. Lookup soft-retained RemoteReplyHandle by threadId or notification key
        RemoteReplyHandle handle = handlesByThread.get(threadId);
        if (handle == null) {
            handle = handlesByKey.get(threadId);
        }
        if (handle == null) {
            throw new IllegalStateException("当前会话暂无后台快捷回复句柄，请解锁手机使用看屏发送或等待对方发来新消息");
        }

        String resolvedThreadId = !handle.threadId.isEmpty() ? handle.threadId : threadId;

        // 3. Dispatch PendingIntent with RemoteInput results (do NOT cancel notification!)
        try {
            replyDispatcher.send(context, handle, trimmedText);
        } catch (PendingIntent.CanceledException canceled) {
            handlesByThread.remove(resolvedThreadId);
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
            handle.title,
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

        ThreadRecord thread = threadsById.get(threadId);
        if (thread == null) {
            thread = new ThreadRecord();
            thread.threadId = threadId;
            thread.packageName = packageName;
            thread.appName = appName;
            thread.title = cleanTitle;
        }

        appendMessageIfNotDuplicate(thread.messages, "我", text, nowMs, true, "sent", via, clientMsgId);

        thread.lastSender = "我";
        thread.lastMessage = "我: " + text;
        thread.lastTimeMs = nowMs;
        thread.unreadCount = 0;
        thread.canReply = handlesByThread.containsKey(threadId);

        threadsById.put(threadId, thread);
        trimThreadsCapacity();
        threadsDirty = true;
        revision++;
        saveToDisk();
        return thread;
    }

    /**
     * Merges chat bubbles captured from the active foreground window by ChatUiBridge.
     */
    public synchronized JSONObject mergeCapturedWindowMessages(
            String packageName,
            String appName,
            String rawTitle,
            List<CapturedBubble> bubbles) throws JSONException {
        String safePkg = packageName == null ? "" : packageName;
        String safeAppName = (appName == null || appName.isEmpty())
            ? ImAppCatalog.appDisplayName(null, safePkg)
            : appName;
        String cleanTitle = ImAppCatalog.cleanConversationTitle(safePkg, rawTitle);
        if (cleanTitle.isEmpty()) cleanTitle = safeAppName;
        String threadId = ImAppCatalog.buildThreadId(safePkg, cleanTitle);
        long nowMs = System.currentTimeMillis();

        ThreadRecord thread = threadsById.get(threadId);
        if (thread == null) {
            thread = new ThreadRecord();
            thread.threadId = threadId;
            thread.packageName = safePkg;
            thread.appName = safeAppName;
            thread.title = cleanTitle;
            thread.unreadCount = 0;
        }

        int added = 0;
        String lastBody = thread.lastMessage;
        String lastSender = thread.lastSender.isEmpty() ? cleanTitle : thread.lastSender;
        if (bubbles != null) {
            for (CapturedBubble b : bubbles) {
                if (b == null || b.text.trim().isEmpty()) continue;
                String sender = b.outgoing ? "我" : (b.sender.isEmpty() ? cleanTitle : b.sender);
                long ts = b.timestampMs > 0 ? b.timestampMs : nowMs;
                if (appendMessageIfNotDuplicate(
                        thread.messages,
                        sender,
                        b.text.trim(),
                        ts,
                        b.outgoing,
                        b.outgoing ? "sent" : "received",
                        "ui_bridge",
                        "")) {
                    added++;
                }
                lastSender = sender;
                lastBody = b.outgoing ? ("我: " + b.text.trim()) : b.text.trim();
            }
        }

        thread.lastSender = lastSender;
        thread.lastMessage = lastBody;
        thread.lastTimeMs = nowMs;
        thread.unreadCount = 0;
        thread.canReply = handlesByThread.containsKey(threadId);

        threadsById.put(threadId, thread);
        trimThreadsCapacity();
        threadsDirty = true;
        revision++;
        saveToDisk();

        JSONObject threadJson = thread.toJson();
        JSONObject result = new JSONObject();
        result.put("ok", true);
        result.put("packageName", safePkg);
        result.put("appName", safeAppName);
        result.put("title", cleanTitle);
        result.put("threadId", threadId);
        result.put("capturedCount", added);
        result.put("messages", threadJson.optJSONArray("messages"));
        result.put("thread", threadJson);
        result.put("revision", revision);
        return result;
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
        if (pi == null && handlesByKey.containsKey(targetKey)) {
            pi = handlesByKey.get(targetKey).contentIntent;
        }
        if (pi != null) {
            try {
                pi.send();
                openedViaPendingIntent = true;
            } catch (PendingIntent.CanceledException ignored) {
                contentIntentsByKey.remove(targetKey);
            }
        }

        if (!openedViaPendingIntent && context != null && !resolvedPkg.isEmpty()) {
            Intent launch = context.getPackageManager().getLaunchIntentForPackage(resolvedPkg);
            if (launch != null) {
                launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                context.startActivity(launch);
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

    public synchronized boolean isUnchangedSnapshot(long sinceRevision) {
        return sinceRevision > 0 && sinceRevision == revision;
    }

    public synchronized JSONObject snapshot(long sinceRevision) throws JSONException {
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
        out.put("unchanged", unchanged);

        List<ThreadRecord> sortedThreads = getSortedThreadsList();
        JSONArray threadsArr = new JSONArray();
        int unreadImCount = 0;
        for (ThreadRecord t : sortedThreads) {
            t.canReply = handlesByThread.containsKey(t.threadId);
            unreadImCount += t.unreadCount;
            threadsArr.put(t.toJson());
        }

        JSONArray feedArr = new JSONArray();
        JSONArray otpBannersArr = new JSONArray();
        int activePushCount = 0;
        for (NotificationRecord n : notifications) {
            if (n.active && !n.ongoing) {
                activePushCount++;
            }
            boolean nextCanReply = (n.threadId != null && handlesByThread.containsKey(n.threadId))
                || (n.key != null && !n.key.isEmpty() && handlesByKey.containsKey(n.key));
            if (n.canReply != nextCanReply) {
                n.canReply = nextCanReply;
                n.cachedJson = null;
            }
            feedArr.put(n.toJson());

            if (n.otpCode != null && !n.otpCode.isEmpty() && !n.dismissed && otpBannersArr.length() < 10) {
                JSONObject banner = new JSONObject()
                    .put("code", n.otpCode)
                    .put("appName", n.appName)
                    .put("title", n.title)
                    .put("text", n.text)
                    .put("postTimeMs", n.postTimeMs)
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
        cachedSnapshotRevision = revision;
        cachedSnapshotObj = out;
        cachedSnapshotJson = out.toString();

        JSONObject copy = new JSONObject();
        copy.put("ok", true);
        copy.put("revision", revision);
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
        ThreadRecord thread = threadsById.get(threadId);
        if (thread == null) {
            throw new IllegalStateException("未找到对应会话: " + threadId);
        }
        thread.canReply = handlesByThread.containsKey(threadId);
        JSONObject threadJson = thread.toJson();

        JSONObject out = new JSONObject();
        out.put("ok", true);
        out.put("revision", revision);
        out.put("threadId", threadId);
        out.put("thread", threadJson);
        out.put("messages", threadJson.optJSONArray("messages"));
        return out;
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
        try {
            File threadsFile = new File(storageDir, "im_threads.json");
            if (threadsFile.isFile()) {
                String raw = new String(Files.readAllBytes(threadsFile.toPath()), StandardCharsets.UTF_8);
                JSONObject root = new JSONObject(raw);
                revision = Math.max(revision, root.optLong("revision", 1L));
                JSONArray arr = root.optJSONArray("threads");
                if (arr != null) {
                    for (int i = 0; i < arr.length() && i < MAX_THREADS; i++) {
                        ThreadRecord t = ThreadRecord.fromJson(arr.getJSONObject(i));
                        if (!t.threadId.isEmpty()) {
                            threadsById.put(t.threadId, t);
                        }
                    }
                }
            }
        } catch (Throwable ignored) {
        }

        try {
            File feedFile = new File(storageDir, "push_feed.json");
            if (feedFile.isFile()) {
                String raw = new String(Files.readAllBytes(feedFile.toPath()), StandardCharsets.UTF_8);
                JSONObject root = new JSONObject(raw);
                revision = Math.max(revision, root.optLong("revision", 1L));
                nextSeqId = Math.max(nextSeqId, root.optLong("nextSeqId", 1L));
                JSONArray arr = root.optJSONArray("notifications");
                if (arr != null) {
                    for (int i = 0; i < arr.length() && i < MAX_NOTIFICATIONS; i++) {
                        notifications.add(NotificationRecord.fromJson(arr.getJSONObject(i)));
                    }
                }
            }
        } catch (Throwable ignored) {
        }
    }

    private void saveToDisk() {
        if (batchDepth > 0) return;
        if (storageDir == null) {
            threadsDirty = false;
            feedDirty = false;
            return;
        }
        if (!threadsDirty && !feedDirty) return;
        try {
            storageDir.mkdirs();

            if (threadsDirty) {
                JSONObject threadsRoot = new JSONObject();
                threadsRoot.put("revision", revision);
                JSONArray threadsArr = new JSONArray();
                for (ThreadRecord t : getSortedThreadsList()) {
                    threadsArr.put(t.toJson());
                }
                threadsRoot.put("threads", threadsArr);
                writeAtomic(new File(storageDir, "im_threads.json"), threadsRoot.toString());
                threadsDirty = false;
            }

            if (feedDirty) {
                JSONObject feedRoot = new JSONObject();
                feedRoot.put("revision", revision);
                feedRoot.put("nextSeqId", nextSeqId);
                JSONArray feedArr = new JSONArray();
                for (NotificationRecord n : notifications) {
                    feedArr.put(n.toJson());
                }
                feedRoot.put("notifications", feedArr);
                writeAtomic(new File(storageDir, "push_feed.json"), feedRoot.toString());
                feedDirty = false;
            }
        } catch (Throwable ignored) {
        }
    }

    private static void writeAtomic(File target, String content) throws Exception {
        File tmp = new File(target.getParentFile(), target.getName() + ".tmp");
        Files.write(tmp.toPath(), content.getBytes(StandardCharsets.UTF_8));
        Files.move(
            tmp.toPath(),
            target.toPath(),
            StandardCopyOption.REPLACE_EXISTING,
            StandardCopyOption.ATOMIC_MOVE
        );
    }
}
