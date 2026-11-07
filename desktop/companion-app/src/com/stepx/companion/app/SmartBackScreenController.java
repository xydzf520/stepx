package com.stepx.companion.app;

import android.content.ContentResolver;
import android.content.Context;
import android.content.SharedPreferences;
import android.os.PowerManager;
import android.provider.Settings;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.File;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.util.Locale;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.TimeUnit;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Autonomous on-device Smart BackScreen controller ("背屏智能显示").
 *
 * Synthesizes a 10-second (10 frames @ 1 fps) 17x23 LED notification animation directly
 * inside com.stepx.companion.app when new unread priority notifications arrive, then
 * immediately restores control to StepOS native display_ext firmware (display_mode = auto,
 * basic_type = agent, 0 WakeLocks).
 */
public final class SmartBackScreenController {
    public static final String PREFS_NAME = "smart_backscreen";
    public static final int WINDOW_SECONDS = 10;
    private static final long DEBOUNCE_MS = 350L;
    private static final long WAKE_LOCK_TIMEOUT_MS = 12_000L;
    private static final Pattern OTP_DIGITS_PATTERN = Pattern.compile("(?<!\\d)(\\d{4,8})(?!\\d)");

    private static volatile SmartBackScreenController singleton;

    public static SmartBackScreenController get(Context context) {
        if (singleton == null) {
            synchronized (SmartBackScreenController.class) {
                if (singleton == null) {
                    singleton = new SmartBackScreenController(
                        context != null ? context.getApplicationContext() : null
                    );
                }
            }
        } else if (context != null && singleton.appContext == null) {
            singleton.appContext = context.getApplicationContext();
        }
        return singleton;
    }

    private final ScheduledExecutorService executor = Executors.newSingleThreadScheduledExecutor(
        new ThreadFactory() {
            @Override
            public Thread newThread(Runnable r) {
                Thread t = new Thread(r, "StepXSmartBackScreen");
                t.setDaemon(true);
                return t;
            }
        }
    );

    private volatile Context appContext;
    private boolean prefsLoaded = false;
    private volatile boolean enabled = true;
    private volatile boolean autoNotifyFlash = true;
    private volatile int brightness = 6;
    private volatile boolean invert = false;

    private volatile String lastSignature = "";
    private volatile long activeWindowUntilMs = 0L;
    private volatile String appliedTitle = "";
    private volatile String appliedKind = "idle";
    private volatile String appliedBadge = "";
    private volatile int totalUnreadCount = 0;
    private volatile boolean activeHoldingBackScreen = false;
    private volatile String lastError = "";
    private volatile long lastTriggeredAtMs = 0L;
    private volatile long lastReleasedAtMs = 0L;

    private BackScreen screen;
    private PowerManager.WakeLock wakeLock;
    private ScheduledFuture<?> debounceFuture;
    private ScheduledFuture<?> playbackFuture;
    private int[][] currentFrames;
    private int currentFrameIndex = 0;

    private SmartBackScreenController(Context context) {
        this.appContext = context;
        if (context != null) {
            loadPrefs(context);
        }
    }

    private synchronized void loadPrefs(Context context) {
        if (context == null || prefsLoaded) return;
        try {
            SharedPreferences sp = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE);
            enabled = sp.getBoolean("enabled", true);
            autoNotifyFlash = sp.getBoolean("autoNotifyFlash", true);
            brightness = clampBrightness(sp.getInt("brightness", 6));
            invert = sp.getBoolean("invert", false);
            prefsLoaded = true;
        } catch (Throwable ignored) {
        }
    }

    private static int clampBrightness(int b) {
        return Math.max(1, Math.min(8, b));
    }

    public synchronized JSONObject configure(Context context, JSONObject q) {
        if (context != null && appContext == null) {
            appContext = context.getApplicationContext();
        }
        Context ctx = context != null ? context : appContext;
        loadPrefs(ctx);
        if (q != null) {
            if (q.has("enabled")) {
                enabled = q.optBoolean("enabled", enabled);
            }
            if (q.has("autoNotifyFlash") || q.has("auto_notify_flash")) {
                autoNotifyFlash = q.optBoolean(
                    "autoNotifyFlash",
                    q.optBoolean("auto_notify_flash", autoNotifyFlash)
                );
            }
            if (q.has("brightness")) {
                brightness = clampBrightness(q.optInt("brightness", brightness));
            }
            if (q.has("invert")) {
                invert = q.optBoolean("invert", invert);
            }
            if (q.optBoolean("resetSignature", q.optBoolean("reset_signature", false))) {
                lastSignature = "";
            }
            if (ctx != null) {
                try {
                    ctx.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
                        .edit()
                        .putBoolean("enabled", enabled)
                        .putBoolean("autoNotifyFlash", autoNotifyFlash)
                        .putInt("brightness", brightness)
                        .putBoolean("invert", invert)
                        .apply();
                } catch (Throwable ignored) {
                }
            }
        }
        if (ctx != null) {
            onNotificationsUpdated(ctx, PushStore.get(ctx));
        }
        return getStatusJson(ctx);
    }

    public void onNotificationsUpdated(final Context context, final PushStore store) {
        if (context != null && appContext == null) {
            appContext = context.getApplicationContext();
        }
        final Context ctx = context != null ? context.getApplicationContext() : appContext;
        if (ctx == null) return;
        loadPrefs(ctx);
        synchronized (this) {
            if (debounceFuture != null && !debounceFuture.isDone()) {
                debounceFuture.cancel(false);
            }
            debounceFuture = executor.schedule(
                new Runnable() {
                    @Override
                    public void run() {
                        evaluateNotificationsInternal(ctx, store != null ? store : PushStore.get(ctx));
                    }
                },
                DEBOUNCE_MS,
                TimeUnit.MILLISECONDS
            );
        }
    }

    private void evaluateNotificationsInternal(final Context context, PushStore store) {
        if (context == null || store == null) return;
        loadPrefs(context);

        if (!enabled || !autoNotifyFlash) {
            if (activeHoldingBackScreen) {
                releaseToNative(context);
            }
            return;
        }

        try {
            PowerManager pm = context.getSystemService(PowerManager.class);
            if (pm != null) {
                boolean powerSave = pm.isPowerSaveMode();
                int thermal = pm.getCurrentThermalStatus();
                if (powerSave || thermal >= PowerManager.THERMAL_STATUS_MODERATE) {
                    if (activeHoldingBackScreen) {
                        releaseToNative(context);
                    }
                    return;
                }
            }
        } catch (Throwable ignored) {
        }

        Candidate candidate = inspectStoreCandidate(store);
        totalUnreadCount = candidate.totalUnread;

        if (candidate.totalUnread <= 0) {
            lastSignature = "";
            if (activeHoldingBackScreen) {
                releaseToNative(context);
            }
            return;
        }

        if (candidate.signature.equals(lastSignature)) {
            return;
        }

        lastSignature = candidate.signature;
        startNotificationWindow(context, candidate);
    }

    private void startNotificationWindow(final Context context, Candidate candidate) {
        cancelPlaybackFuture();
        int b = clampBrightness(brightness);
        boolean inv = invert;
        int[][] frames = buildTenSecondFrames(candidate.kind, candidate.badgeText, b, inv);

        try {
            PowerManager pm = context.getSystemService(PowerManager.class);
            if (pm != null) {
                if (wakeLock == null) {
                    wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "StepX:SmartBackScreen");
                    wakeLock.setReferenceCounted(false);
                }
                wakeLock.acquire(WAKE_LOCK_TIMEOUT_MS);
            }

            if (screen == null) {
                screen = new BackScreen(context);
            }
            // Ensure saved restore target always returns to native auto mode (0) and agent type
            SharedPreferences restorePrefs = context.getSharedPreferences("backscreen_restore", 0);
            if (!restorePrefs.getBoolean("pending", false)) {
                screen.begin(b, false);
                restorePrefs.edit().putInt("mode", 0).putString("type", "agent").putInt("automatic", 1).commit();
            } else {
                restorePrefs.edit().putInt("mode", 0).putString("type", "agent").putInt("automatic", 1).commit();
                screen.begin(b, false);
            }

            currentFrames = frames;
            currentFrameIndex = 0;
            appliedKind = candidate.kind;
            appliedTitle = candidate.title;
            appliedBadge = candidate.badgeText;
            lastError = "";
            long now = System.currentTimeMillis();
            lastTriggeredAtMs = now;
            activeWindowUntilMs = now + (WINDOW_SECONDS * 1000L);
            activeHoldingBackScreen = true;

            screen.push(currentFrames[0]);
            currentFrameIndex = 1;
            writeStatusFiles(context);

            playbackFuture = executor.scheduleAtFixedRate(
                new Runnable() {
                    @Override
                    public void run() {
                        stepPlaybackFrame(context);
                    }
                },
                1000L,
                1000L,
                TimeUnit.MILLISECONDS
            );
        } catch (Throwable e) {
            lastError = e.getClass().getSimpleName() + ": " + String.valueOf(e.getMessage());
            releaseToNative(context);
        }
    }

    private void stepPlaybackFrame(Context context) {
        try {
            if (!activeHoldingBackScreen || currentFrames == null) {
                releaseToNative(context);
                return;
            }
            if (!enabled || !autoNotifyFlash) {
                releaseToNative(context);
                return;
            }
            PowerManager pm = context != null ? context.getSystemService(PowerManager.class) : null;
            if (pm != null && (pm.isPowerSaveMode() || pm.getCurrentThermalStatus() >= PowerManager.THERMAL_STATUS_MODERATE)) {
                releaseToNative(context);
                return;
            }
            if (currentFrameIndex >= currentFrames.length || System.currentTimeMillis() >= activeWindowUntilMs) {
                releaseToNative(context);
                return;
            }
            if (screen != null) {
                screen.push(currentFrames[currentFrameIndex]);
            }
            currentFrameIndex++;
        } catch (Throwable e) {
            lastError = e.getClass().getSimpleName() + ": " + String.valueOf(e.getMessage());
            releaseToNative(context);
        }
    }

    private synchronized void cancelPlaybackFuture() {
        if (playbackFuture != null) {
            playbackFuture.cancel(false);
            playbackFuture = null;
        }
    }

    public void releaseToNative(Context context) {
        Context ctx = context != null ? context : appContext;
        cancelPlaybackFuture();
        currentFrames = null;
        currentFrameIndex = 0;
        activeWindowUntilMs = 0L;
        boolean wasHolding = activeHoldingBackScreen;
        activeHoldingBackScreen = false;
        lastReleasedAtMs = System.currentTimeMillis();

        if (ctx != null) {
            try {
                SharedPreferences restorePrefs = ctx.getSharedPreferences("backscreen_restore", 0);
                if (restorePrefs.getBoolean("pending", false)) {
                    restorePrefs.edit()
                        .putInt("mode", 0)
                        .putInt("automatic", 1)
                        .putString("type", "agent")
                        .commit();
                } else if (wasHolding) {
                    restorePrefs.edit()
                        .putBoolean("pending", true)
                        .putInt("mode", 0)
                        .putInt("brightness", 127)
                        .putInt("automatic", 1)
                        .putInt("rotate", 1)
                        .putString("type", "agent")
                        .commit();
                }
                if (screen == null && restorePrefs.getBoolean("pending", false)) {
                    screen = new BackScreen(ctx);
                }
                if (screen != null) {
                    screen.restore(true);
                }
            } catch (Throwable e) {
                lastError = e.getClass().getSimpleName() + ": " + String.valueOf(e.getMessage());
            }
            try {
                ContentResolver cr = ctx.getContentResolver();
                if (cr != null) {
                    Settings.System.putInt(cr, "stepos_back_screen_display_mode", 0);
                }
            } catch (Throwable ignored) {
            }
        }

        try {
            if (wakeLock != null && wakeLock.isHeld()) {
                wakeLock.release();
            }
        } catch (Throwable ignored) {
        }

        if (ctx != null && wasHolding) {
            writeStatusFiles(ctx);
        }
    }

    public JSONObject getStatusJson(Context context) {
        Context ctx = context != null ? context : appContext;
        loadPrefs(ctx);
        long now = System.currentTimeMillis();
        boolean windowActive = activeHoldingBackScreen && activeWindowUntilMs > now;
        int remainingSecs = windowActive
            ? (int) Math.max(0L, (activeWindowUntilMs - now + 999L) / 1000L)
            : 0;
        boolean wlHeld = false;
        try {
            wlHeld = wakeLock != null && wakeLock.isHeld();
        } catch (Throwable ignored) {
        }
        JSONObject out = new JSONObject();
        try {
            out.put("ok", true);
            out.put("autonomous", true);
            out.put("enabled", enabled);
            out.put("autoNotifyFlash", autoNotifyFlash);
            out.put("brightness", brightness);
            out.put("invert", invert);
            out.put("active", windowActive);
            out.put("holdingBackScreen", activeHoldingBackScreen);
            out.put("wakeLockHeld", wlHeld);
            out.put("nativeDisplayModeTarget", "auto");
            out.put("activeWindowUntilMs", activeWindowUntilMs);
            out.put("remainingSecs", remainingSecs);
            out.put("lastSignature", lastSignature);
            out.put("appliedKind", appliedKind);
            out.put("appliedTitle", appliedTitle);
            out.put("appliedBadge", appliedBadge);
            out.put("totalUnread", totalUnreadCount);
            out.put("lastTriggeredAtMs", lastTriggeredAtMs);
            out.put("lastReleasedAtMs", lastReleasedAtMs);
            out.put("lastError", lastError);
            out.put("timestampMs", now);
        } catch (Throwable ignored) {
        }
        return out;
    }

    private void writeStatusFiles(Context context) {
        if (context == null) return;
        try {
            byte[] payload = getStatusJson(context).toString().getBytes(StandardCharsets.UTF_8);
            File extDir = context.getExternalFilesDir(null);
            if (extDir != null) {
                writeAtomic(new File(extDir, "smart_backscreen_status.json"), payload);
            }
            File tmpDir = new File("/data/local/tmp/stepx-companion");
            if (tmpDir.isDirectory() && tmpDir.canWrite()) {
                writeAtomic(new File(tmpDir, "backscreen-mcp-status.json"), payload);
            }
        } catch (Throwable ignored) {
        }
    }

    private static void writeAtomic(File file, byte[] data) throws Exception {
        File parent = file.getParentFile();
        if (parent != null && !parent.exists()) {
            parent.mkdirs();
        }
        File temp = new File(parent, file.getName() + ".tmp");
        FileOutputStream out = new FileOutputStream(temp);
        try {
            out.write(data);
            out.getFD().sync();
        } finally {
            out.close();
        }
        if (!temp.renameTo(file)) {
            FileOutputStream direct = new FileOutputStream(file);
            try {
                direct.write(data);
                direct.getFD().sync();
            } finally {
                direct.close();
            }
            temp.delete();
        }
    }

    static final class Candidate {
        final String kind;
        final int totalUnread;
        final long latestTimestampMs;
        final String badgeText;
        final String title;
        final String signature;

        Candidate(String kind, int totalUnread, long latestTimestampMs, String badgeText, String title) {
            this.kind = kind == null ? "idle" : kind;
            this.totalUnread = Math.max(0, totalUnread);
            this.latestTimestampMs = Math.max(0L, latestTimestampMs);
            this.badgeText = badgeText == null ? "" : badgeText;
            this.title = title == null ? "" : title;
            this.signature = this.kind + ":" + this.totalUnread + ":" + this.latestTimestampMs + ":" + this.badgeText;
        }
    }

    static Candidate inspectStoreCandidate(PushStore store) {
        if (store == null) {
            return new Candidate("idle", 0, 0L, "", "");
        }
        try {
            JSONObject snap = store.snapshot(0L);
            return inspectSnapshotJson(snap);
        } catch (Throwable ignored) {
            return new Candidate("idle", 0, 0L, "", "");
        }
    }

    static Candidate inspectSnapshotJson(JSONObject snap) {
        if (snap == null) {
            return new Candidate("idle", 0, 0L, "", "");
        }
        long nowMs = System.currentTimeMillis();

        String bestOtpCode = "";
        long bestOtpTimeMs = 0L;
        String bestOtpApp = "";

        int deliveryCount = 0;
        long latestDeliveryTimeMs = 0L;
        String deliveryCode = "";

        int callAlarmCount = 0;
        long latestCallAlarmTimeMs = 0L;
        String callAlarmKind = "call";

        // 1. Check active OTP banners
        JSONArray otpBanners = snap.optJSONArray("otpBanners");
        if (otpBanners != null) {
            for (int i = 0; i < otpBanners.length(); i++) {
                JSONObject b = otpBanners.optJSONObject(i);
                if (b == null) continue;
                String code = b.optString("code", "").trim();
                long ts = b.optLong("postTimeMs", 0L);
                String title = b.optString("title", "");
                String text = b.optString("text", "");
                String appName = b.optString("appName", "");
                if (!code.isEmpty() && (ts <= 0L || nowMs <= 0L || (nowMs - ts) <= PushStore.OTP_TTL_MS)) {
                    if (isDeliveryText(appName, title, text)) {
                        deliveryCount++;
                        if (ts >= latestDeliveryTimeMs) {
                            latestDeliveryTimeMs = ts;
                            deliveryCode = extractLast4Digits(code);
                        }
                    } else if (ts >= bestOtpTimeMs) {
                        bestOtpTimeMs = ts;
                        bestOtpCode = extractLast4Digits(code);
                        bestOtpApp = !appName.isEmpty() ? appName : title;
                    }
                }
            }
        }

        // 2. Check active unread notifications
        JSONArray notifications = snap.optJSONArray("notifications");
        if (notifications != null) {
            for (int i = 0; i < notifications.length(); i++) {
                JSONObject n = notifications.optJSONObject(i);
                if (n == null) continue;
                boolean active = n.optBoolean("active", false);
                boolean read = n.optBoolean("read", false);
                boolean dismissed = n.optBoolean("dismissed", false);
                boolean ongoing = n.optBoolean("ongoing", false);
                if (!active || read || dismissed) continue;

                String pkg = n.optString("packageName", "");
                String appName = n.optString("appName", "");
                String cat = n.optString("category", "");
                String title = n.optString("title", "");
                String text = n.optString("text", "");
                String otpCode = n.optString("otpCode", "").trim();
                long ts = n.optLong("postTimeMs", 0L);

                if (otpCode.isEmpty() && hasOtpKeyword(title, text)) {
                    otpCode = extractDigitsFromText(title + " " + text);
                }

                if (isDeliveryNotification(pkg, appName, cat, title, text)) {
                    if (!ongoing || ts > 0L) {
                        deliveryCount++;
                        if (ts >= latestDeliveryTimeMs) {
                            latestDeliveryTimeMs = ts;
                            if (!otpCode.isEmpty()) {
                                deliveryCode = extractLast4Digits(otpCode);
                            }
                        }
                    }
                    continue;
                }

                if (!otpCode.isEmpty() && (ts <= 0L || nowMs <= 0L || (nowMs - ts) <= PushStore.OTP_TTL_MS)) {
                    if (ts >= bestOtpTimeMs) {
                        bestOtpTimeMs = ts;
                        bestOtpCode = extractLast4Digits(otpCode);
                        bestOtpApp = !appName.isEmpty() ? appName : title;
                    }
                    continue;
                }

                if (!ongoing && isCallOrCalendarNotification(pkg, cat, title, text)) {
                    callAlarmCount++;
                    if (ts >= latestCallAlarmTimeMs) {
                        latestCallAlarmTimeMs = ts;
                        callAlarmKind = isCalendarNotification(pkg, cat, title, text) ? "calendar" : "call";
                    }
                }
            }
        }

        // 3. Check unread IM threads
        int imUnreadTotal = 0;
        long latestImTimeMs = 0L;
        String latestImPkg = "";
        String latestImTitle = "";
        JSONArray threads = snap.optJSONArray("threads");
        if (threads != null) {
            for (int i = 0; i < threads.length(); i++) {
                JSONObject t = threads.optJSONObject(i);
                if (t == null) continue;
                int unread = t.optInt("unreadCount", 0);
                if (unread <= 0) continue;
                String pkg = t.optString("packageName", "");
                String title = t.optString("title", "");
                long ts = t.optLong("lastTimestampMs", 0L);
                imUnreadTotal += unread;
                if (ts >= latestImTimeMs) {
                    latestImTimeMs = ts;
                    latestImPkg = pkg;
                    latestImTitle = title;
                }
            }
        }

        int totalUnread = (bestOtpCode.isEmpty() ? 0 : 1) + deliveryCount + callAlarmCount + imUnreadTotal;
        if (totalUnread <= 0) {
            return new Candidate("idle", 0, 0L, "", "");
        }

        // Priority order:
        // 1) Unread OTP verification code
        if (!bestOtpCode.isEmpty()) {
            String badge = bestOtpCode;
            String label = "验证码 · " + badge;
            return new Candidate("otp", totalUnread, bestOtpTimeMs, badge, label);
        }

        // 2) Missed Call / Calendar Alarm if newer than IM/Delivery
        if (callAlarmCount > 0 && latestCallAlarmTimeMs >= latestImTimeMs && latestCallAlarmTimeMs >= latestDeliveryTimeMs) {
            String badge = String.valueOf(Math.min(99, callAlarmCount));
            String label = ("calendar".equals(callAlarmKind) ? "日程提醒 · " : "未接来电 · ") + badge;
            return new Candidate(callAlarmKind, totalUnread, latestCallAlarmTimeMs, badge, label);
        }

        // 3) Express / Delivery if newer than IM
        if (deliveryCount > 0 && latestDeliveryTimeMs >= latestImTimeMs) {
            String badge = !deliveryCode.isEmpty() ? deliveryCode : String.valueOf(Math.min(99, deliveryCount));
            String label = !deliveryCode.isEmpty() ? ("取件码 · " + deliveryCode) : ("快递外卖 · " + badge);
            return new Candidate("delivery", totalUnread, latestDeliveryTimeMs, badge, label);
        }

        // 4) Unread IM Threads
        if (imUnreadTotal > 0) {
            String kind = classifyImKind(latestImPkg);
            String badge = String.valueOf(Math.min(99, imUnreadTotal));
            String appLabel = ImAppCatalog.appDisplayName(null, latestImPkg);
            String title = appLabel + (!latestImTitle.isEmpty() ? (" · " + latestImTitle) : "") + " (" + imUnreadTotal + ")";
            return new Candidate(kind, totalUnread, latestImTimeMs, badge, title);
        }

        if (deliveryCount > 0) {
            String badge = !deliveryCode.isEmpty() ? deliveryCode : String.valueOf(Math.min(99, deliveryCount));
            return new Candidate("delivery", totalUnread, latestDeliveryTimeMs, badge, "快递外卖 · " + badge);
        }

        String badge = String.valueOf(Math.min(99, callAlarmCount));
        return new Candidate(callAlarmKind, totalUnread, latestCallAlarmTimeMs, badge, "提醒 · " + badge);
    }

    private static String classifyImKind(String pkg) {
        if (pkg == null) return "im";
        String p = pkg.trim();
        if ("com.tencent.mm".equals(p)) return "wechat";
        if ("com.ss.android.lark".equals(p) || "com.larksuite.suite".equals(p)) return "feishu";
        if ("com.alibaba.android.rimet".equals(p)) return "dingtalk";
        if ("com.tencent.wework".equals(p)) return "wework";
        if ("com.tencent.mobileqq".equals(p) || "com.tencent.tim".equals(p)) return "qq";
        return "im";
    }

    private static boolean hasOtpKeyword(String title, String text) {
        String combined = ((title == null ? "" : title) + " " + (text == null ? "" : text)).toLowerCase(Locale.ROOT);
        return combined.contains("验证码")
            || combined.contains("校验码")
            || combined.contains("动态码")
            || combined.contains("取件码")
            || combined.contains("verification code")
            || combined.contains("security code")
            || combined.contains("otp")
            || combined.contains("passcode");
    }

    private static String extractDigitsFromText(String s) {
        if (s == null || s.isEmpty()) return "";
        Matcher m = OTP_DIGITS_PATTERN.matcher(s);
        if (m.find()) {
            return m.group(1);
        }
        return "";
    }

    private static String extractLast4Digits(String code) {
        if (code == null) return "";
        StringBuilder digits = new StringBuilder();
        for (int i = 0; i < code.length(); i++) {
            char c = code.charAt(i);
            if (c >= '0' && c <= '9') {
                digits.append(c);
            }
        }
        if (digits.length() == 0) return "";
        if (digits.length() <= 4) return digits.toString();
        return digits.substring(digits.length() - 4);
    }

    private static boolean isDeliveryNotification(String pkg, String appName, String category, String title, String text) {
        if (ImAppCatalog.CAT_DELIVERY_TRAVEL.equalsIgnoreCase(category == null ? "" : category)) {
            return true;
        }
        String p = pkg == null ? "" : pkg.toLowerCase(Locale.ROOT);
        if (p.contains("cainiao") || p.contains("meituan") || p.contains("eleme")
                || p.contains("sfexpress") || p.contains("jingdong") || p.contains("hippo")
                || p.contains("yaya.zone") || p.contains("samsclub")) {
            return true;
        }
        return isDeliveryText(appName, title, text);
    }

    private static boolean isDeliveryText(String appName, String title, String text) {
        String combined = (appName == null ? "" : appName) + " "
            + (title == null ? "" : title) + " "
            + (text == null ? "" : text);
        return combined.contains("菜鸟")
            || combined.contains("顺丰")
            || combined.contains("京东")
            || combined.contains("美团")
            || combined.contains("饿了么")
            || combined.contains("快递")
            || combined.contains("外卖")
            || combined.contains("取件码")
            || combined.contains("丰巢")
            || combined.contains("驿站")
            || combined.contains("派送")
            || combined.contains("骑手");
    }

    private static boolean isCallOrCalendarNotification(String pkg, String category, String title, String text) {
        if (isCalendarNotification(pkg, category, title, text)) return true;
        String cat = category == null ? "" : category.toLowerCase(Locale.ROOT);
        String p = pkg == null ? "" : pkg.toLowerCase(Locale.ROOT);
        String combined = (title == null ? "" : title) + " " + (text == null ? "" : text);
        return "call".equals(cat)
            || "missed_call".equals(cat)
            || p.contains("dialer")
            || p.contains("telecom")
            || p.contains("incallui")
            || combined.contains("未接来电")
            || combined.contains("Missed call");
    }

    private static boolean isCalendarNotification(String pkg, String category, String title, String text) {
        String cat = category == null ? "" : category.toLowerCase(Locale.ROOT);
        String p = pkg == null ? "" : pkg.toLowerCase(Locale.ROOT);
        String combined = (title == null ? "" : title) + " " + (text == null ? "" : text);
        return "event".equals(cat)
            || "alarm".equals(cat)
            || "reminder".equals(cat)
            || p.contains("calendar")
            || p.contains("deskclock")
            || combined.contains("日程提醒")
            || combined.contains("会议提醒");
    }

    /**
     * Synthesizes a 10-second (10 frames @ 1 fps) 17x23 (391 ints) notification animation:
     * - Top area (y = 1..8): 17x8 pixel icon for WeChat / Feishu / DingTalk / WeWork / OTP / Delivery / Call / General message.
     * - Middle area (y = 11..15): 3x5 pixel digits showing unread count (e.g. 1, 2, 12) or last 4 digits of OTP code.
     * - Bottom row (y = 21, x = 3..12): 10-pixel countdown progress bar shrinking by 1 pixel/sec (10 - sec).
     * - Masks the 4 corner pixels ((0,0), (16,0), (0,22), (16,22)) and hardware-masked corner LEDs to 0.
     */
    public static int[][] buildTenSecondFrames(String kind, String badgeText, int brightness, boolean invert) {
        int hi = Math.max(16, Math.min(255, clampBrightness(brightness) * 255 / 8));
        int lo = Math.max(12, hi * 3 / 5);
        int[][] frames = new int[WINDOW_SECONDS][391];
        String[] iconRows = iconRowsForKind(kind, false);
        String[] iconAltRows = iconRowsForKind(kind, true);

        for (int sec = 0; sec < WINDOW_SECONDS; sec++) {
            int[] f = new int[391];
            String[] rows = (sec % 2 == 0) ? iconRows : iconAltRows;

            // 1. Top area (y = 1..8): 17x8 pixel icon
            for (int r = 0; r < 8 && r < rows.length; r++) {
                String row = rows[r];
                int y = 1 + r;
                for (int x = 0; x < 17 && x < row.length(); x++) {
                    char ch = row.charAt(x);
                    if (ch == '1' || ch == '#') {
                        GameCore.put(f, x, y, hi);
                    } else if (ch == '.') {
                        GameCore.put(f, x, y, lo);
                    }
                }
            }

            // 2. Middle area (y = 11..15): 3x5 pixel digits for unread count or 4-digit OTP
            drawBadgeDigits(f, badgeText, 11, hi);

            // 3. Bottom row (y = 21, x = 3..12): 10-pixel countdown bar shrinking by 1 pixel/sec
            int barLen = Math.max(0, WINDOW_SECONDS - sec);
            for (int i = 0; i < barLen; i++) {
                GameCore.put(f, 3 + i, 21, hi);
            }

            // 4. Optional inversion + strict corner/hardware masking
            if (invert) {
                for (int idx = 0; idx < 391; idx++) {
                    int x = idx % 17;
                    int y = idx / 17;
                    if (!isCornerOrMasked(x, y)) {
                        f[idx] = f[idx] > 0 ? 0 : lo;
                    }
                }
            }
            for (int idx = 0; idx < 391; idx++) {
                int x = idx % 17;
                int y = idx / 17;
                if (isCornerOrMasked(x, y)) {
                    f[idx] = 0;
                }
            }
            frames[sec] = f;
        }
        return frames;
    }

    private static boolean isCornerOrMasked(int x, int y) {
        if ((x == 0 && y == 0) || (x == 16 && y == 0) || (x == 0 && y == 22) || (x == 16 && y == 22)) {
            return true;
        }
        return GameCore.masked(x, y);
    }

    private static void drawBadgeDigits(int[] f, String badgeText, int y, int v) {
        String clean = badgeText == null ? "" : badgeText.trim();
        StringBuilder digitsOnly = new StringBuilder();
        for (int i = 0; i < clean.length() && digitsOnly.length() < 4; i++) {
            char c = clean.charAt(i);
            if (c >= '0' && c <= '9') {
                digitsOnly.append(c);
            }
        }
        if (digitsOnly.length() == 0) {
            digitsOnly.append('1');
        }
        int len = digitsOnly.length();
        // Each digit is 3x5 with 1-column spacing: totalWidth = len * 3 + (len - 1)
        int totalWidth = len * 3 + (len - 1);
        int startX = (17 - totalWidth) / 2;
        for (int i = 0; i < len; i++) {
            int d = digitsOnly.charAt(i) - '0';
            GameCore.digit(f, d, startX + i * 4, y, v);
        }
    }

    private static String[] iconRowsForKind(String kind, boolean alt) {
        String k = kind == null ? "im" : kind.toLowerCase(Locale.ROOT);
        if ("otp".equals(k)) {
            // Key / Shield lock icon (17x8)
            return new String[] {
                "00000011111000000",
                "00000100000100000",
                "00000100000100000",
                "00001111111110000",
                "00001111.11110000",
                "00001111111110000",
                "00000111111100000",
                alt ? "00000011111000000" : "00000001110000000"
            };
        }
        if ("wechat".equals(k)) {
            // Dual chat bubble icon (17x8)
            return new String[] {
                "00011111100000000",
                "00111111110000000",
                "0011.11.110111100",
                "00111111111111110",
                "00011111111.1.110",
                "00011000011111110",
                "00000000001111100",
                alt ? "00000000000011000" : "00000000000110000"
            };
        }
        if ("feishu".equals(k)) {
            // Paper plane / wing icon (17x8)
            return new String[] {
                "00000000001111000",
                "00000000111110000",
                "00000011111100000",
                "00011111111000000",
                "00000111111110000",
                "00000001110111000",
                "00000001100011000",
                alt ? "00000001000000000" : "00000011000000000"
            };
        }
        if ("dingtalk".equals(k)) {
            // Lightning bolt / wing icon (17x8)
            return new String[] {
                "00000001111100000",
                "00000011111000000",
                "00000111110000000",
                "00001111111110000",
                "00000001111100000",
                "00000011110000000",
                "00000111000000000",
                alt ? "00001100000000000" : "00000110000000000"
            };
        }
        if ("delivery".equals(k)) {
            // Package box icon (17x8)
            return new String[] {
                "00000111111100000",
                "00001100100110000",
                "00011111111111000",
                "00011000100011000",
                "00011000100011000",
                "00011000000011000",
                "00011111111111000",
                alt ? "00001111111110000" : "00000111111100000"
            };
        }
        if ("call".equals(k) || "calendar".equals(k)) {
            // Bell / handset alert icon (17x8)
            return new String[] {
                "00000001110000000",
                "00000111111100000",
                "00001111111110000",
                "00001111111110000",
                "00011111111111000",
                "00111111111111100",
                "00000001110000000",
                alt ? "00000011111000000" : "00000000100000000"
            };
        }
        // Default IM / envelope / chat bubble icon (17x8)
        return new String[] {
            "00011111111111000",
            "00111000000011100",
            "00110110001101100",
            "00110001110001100",
            "00110000100001100",
            "00110000000001100",
            "00011111111111000",
            alt ? "00000011100000000" : "00000001100000000"
        };
    }
}
