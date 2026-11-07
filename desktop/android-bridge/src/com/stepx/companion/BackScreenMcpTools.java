package com.stepx.companion;

import android.content.Context;
import android.os.IBinder;
import android.os.Parcel;
import android.os.PowerManager;
import android.os.SystemClock;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.Arrays;

/**
 * Phone-local Binder animation driver; this historical class name is not an MCP service.
 * Runs as shell (uid=2000) without requiring com.stepx.companion.app or CompanionService.
 * Uses hardware display_mode=always (basicAlwaysOnGateBypass=true) so phone posture on a
 * desk never falsely pauses animations with "请将背屏朝向自己".
 */
final class BackScreenMcpTools {
    private static final String DESCRIPTOR = "android.hardware.display.IStepOSDisplayManager";
    private static final File DIR = new File(Bridge.ROOT);
    private static final File ANIM_FILE = new File(DIR, "backscreen-anim.json");
    private static final File JOB_FILE = new File(DIR, "backscreen-anim.job");
    private static final File RESTORE_FILE = new File(DIR, "backscreen-restore.json");
    private static final File STATUS_FILE = new File(DIR, "backscreen-mcp-status.json");

    private static IBinder getDisplayExtBinder() throws Exception {
        IBinder binder = (IBinder) Class.forName("android.os.ServiceManager")
                .getMethod("getService", String.class)
                .invoke(null, "display_ext");
        if (binder == null || !DESCRIPTOR.equals(binder.getInterfaceDescriptor())) {
            throw new IOException("当前固件未暴露 StepOS 背屏 display_ext 接口");
        }
        return binder;
    }

    private static Parcel transact(IBinder binder, int code, Integer number, String text, int[] frame, boolean frameCall) throws Exception {
        Parcel data = Parcel.obtain();
        Parcel reply = Parcel.obtain();
        try {
            data.writeInterfaceToken(DESCRIPTOR);
            if (number != null) data.writeInt(number);
            if (text != null) data.writeString(text);
            if (frameCall) {
                if (frame != null) {
                    data.writeIntArray(frame);
                } else {
                    data.writeInt(-1);
                }
                data.writeInt(0);
            }
            if (!binder.transact(code, data, reply, 0)) {
                throw new IOException("固件未实现背屏接口 transaction=" + code);
            }
            reply.readException();
            return reply;
        } catch (Exception e) {
            reply.recycle();
            throw e;
        } finally {
            data.recycle();
        }
    }

    private static int getInt(IBinder binder, int code) throws Exception {
        Parcel r = transact(binder, code, null, null, null, false);
        try {
            return r.readInt();
        } finally {
            r.recycle();
        }
    }

    private static String getString(IBinder binder, int code) throws Exception {
        Parcel r = transact(binder, code, null, null, null, false);
        try {
            return r.readString();
        } finally {
            r.recycle();
        }
    }

    private static void setInt(IBinder binder, int code, int value) throws Exception {
        transact(binder, code, value, null, null, false).recycle();
    }

    private static void setString(IBinder binder, int code, String value) throws Exception {
        transact(binder, code, null, value, null, false).recycle();
    }

    static void pushFrame(IBinder binder, int[] frame) throws Exception {
        if (frame != null && frame.length != 391) {
            throw new IOException("背屏帧必须为 391 槽");
        }
        transact(binder, 5, null, null, frame, true).recycle();
    }

    static void clearFrame(IBinder binder) throws Exception {
        transact(binder, 5, null, null, null, true).recycle();
    }

    private static void saveRestoreStateIfNeeded(IBinder binder) {
        try {
            if (RESTORE_FILE.isFile()) return;
            DIR.mkdirs();
            JSONObject saved = new JSONObject()
                    .put("mode", getInt(binder, 20))
                    .put("brightness", getInt(binder, 16))
                    .put("automatic", getInt(binder, 14))
                    .put("rotate", getInt(binder, 32))
                    .put("type", getString(binder, 26));
            writeAtomic(RESTORE_FILE, saved.toString());
        } catch (Throwable ignored) {
        }
    }

    static boolean restoreSavedState(IBinder binder, boolean deleteRecord) {
        try {
            clearFrame(binder);
            if (!RESTORE_FILE.isFile()) return true;
            String raw = new String(Files.readAllBytes(RESTORE_FILE.toPath()), StandardCharsets.UTF_8);
            JSONObject saved = new JSONObject(raw);
            // Desktop and phone used different field names for the same file.
            // Read both schemas without replacing missing values with defaults.
            int brightness = saved.getInt(saved.has("manualBrightness") ? "manualBrightness" : "brightness");
            int automatic = saved.getInt(saved.has("autoBrightness") ? "autoBrightness" : "automatic");
            int mode = saved.getInt(saved.has("displayMode") ? "displayMode" : "mode");
            String type = saved.getString(saved.has("basicType") ? "basicType" : "type");
            if (brightness < 1 || brightness > 255 || automatic < 0 || automatic > 1
                    || mode < 0 || mode > 2 || !("agent".equals(type) || "time".equals(type) || "text".equals(type))) {
                return false;
            }
            setInt(binder, 15, brightness);
            setInt(binder, 13, automatic);
            if (saved.has("rotate")) setInt(binder, 33, saved.getInt("rotate"));
            setString(binder, 27, type);
            setInt(binder, 21, mode);
            if (getInt(binder, 16) != brightness || getInt(binder, 14) != automatic
                    || getInt(binder, 20) != mode || !type.equals(getString(binder, 26))
                    || (saved.has("rotate") && getInt(binder, 32) != saved.getInt("rotate"))) {
                return false;
            }
            if (deleteRecord) {
                if (!RESTORE_FILE.delete()) return false;
            }
            return true;
        } catch (Throwable ignored) {
            // Keep the original snapshot so the user can retry from Desktop.
            return false;
        }
    }

    static void prepareAlwaysOnOutput(IBinder binder, int brightnessLevel) throws Exception {
        if (getInt(binder, 17) == 0) {
            // Enable backscreen if disabled
            try {
                android.provider.Settings.System.putInt(Bridge.context.getContentResolver(), "stepos_back_screen_display", 1);
            } catch (Throwable ignored) {
            }
        }
        saveRestoreStateIfNeeded(binder);
        int clampedLevel = Math.max(1, Math.min(8, brightnessLevel));
        int manualVal = Math.max(16, Math.min(255, clampedLevel * 255 / 8));
        setInt(binder, 13, 0);          // auto brightness off
        setInt(binder, 15, manualVal);  // manual brightness
        setInt(binder, 21, 1);          // display_mode = 1 (always -> basicAlwaysOnGateBypass=true)
    }

    private static void writeAtomic(File target, String content) throws IOException {
        DIR.mkdirs();
        File tmp = new File(DIR, target.getName() + ".tmp." + System.nanoTime());
        Files.write(tmp.toPath(), content.getBytes(StandardCharsets.UTF_8));
        Files.move(tmp.toPath(), target.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
    }

    private static String readActiveJobId() {
        try {
            if (!JOB_FILE.isFile()) return "";
            return new String(Files.readAllBytes(JOB_FILE.toPath()), StandardCharsets.UTF_8).trim();
        } catch (Throwable ignored) {
            return "";
        }
    }

    private static void writeStatus(boolean running, String mode, String jobId, String title, int fps, int brightness, long frameCount, String pause, String error) {
        try {
            JSONObject st = new JSONObject()
                    .put("protocol", 2)
                    .put("engine", "phone-binder-display-ext")
                    .put("uid", android.os.Process.myUid())
                    .put("pid", android.os.Process.myPid())
                    .put("running", running)
                    .put("mode", mode)
                    .put("jobId", jobId == null ? "" : jobId)
                    .put("appliedTitle", title == null ? "" : title)
                    .put("fps", fps)
                    .put("brightness", brightness)
                    .put("frameCount", frameCount)
                    .put("pausedReason", pause == null ? "" : pause)
                    .put("lastError", error == null ? "" : error)
                    .put("updatedAtMs", System.currentTimeMillis())
                    .put("uptimeMs", SystemClock.elapsedRealtime());
            writeAtomic(STATUS_FILE, st.toString());
        } catch (Throwable ignored) {
        }
    }

    static JSONObject status() throws Exception {
        IBinder binder = getDisplayExtBinder();
        boolean enabled = getInt(binder, 17) != 0;
        int displayMode = getInt(binder, 20);
        int brightness = getInt(binder, 16);
        int autoBrightness = getInt(binder, 14);
        String basicType = getString(binder, 26);
        JSONObject st = new JSONObject();
        if (STATUS_FILE.isFile()) {
            try {
                st = new JSONObject(new String(Files.readAllBytes(STATUS_FILE.toPath()), StandardCharsets.UTF_8));
            } catch (Throwable ignored) {
            }
        }
        st.put("enabled", enabled);
        st.put("displayMode", displayMode == 1 ? "always" : displayMode == 2 ? "custom" : "auto");
        st.put("brightnessRaw", brightness);
        st.put("autoBrightness", autoBrightness != 0);
        st.put("basicType", basicType);
        return st;
    }

    static JSONObject stop() throws Exception {
        JOB_FILE.delete();
        ANIM_FILE.delete();
        IBinder binder = getDisplayExtBinder();
        if (!restoreSavedState(binder, true)) {
            writeStatus(false, "stopped", "", "", 0, 5, 0, "", "原背屏设置恢复失败，快照已保留，请重试");
            throw new IOException("原背屏设置恢复失败，快照已保留，请重试");
        }
        writeStatus(false, "stopped", "", "", 0, 5, 0, "", "");
        return new JSONObject()
                .put("stopped", true)
                .put("message", "已停止手机本地点阵驱动，保留原背屏设置");
    }

    /**
     * Background loop entry point invoked via:
     * app_process -Djava.class.path=/data/local/tmp/stepx-companion/stepx-bridge.jar /system/bin com.stepx.companion.Bridge --backscreen-loop <jobId>
     */
    static void runLoop(String[] args) throws Exception {
        String expectedJob = args.length >= 2 ? args[1].trim() : readActiveJobId();
        if (expectedJob.isEmpty() || !ANIM_FILE.isFile()) return;
        String raw = new String(Files.readAllBytes(ANIM_FILE.toPath()), StandardCharsets.UTF_8);
        JSONObject cfg = new JSONObject(raw);
        String jobId = cfg.optString("jobId", expectedJob);
        if (!jobId.equals(expectedJob)) return;
        String title = cfg.optString("title", "背屏图案");
        int fps = Math.max(1, Math.min(12, cfg.optInt("fps", 6)));
        int brightness = Math.max(1, Math.min(8, cfg.optInt("brightness", 5)));
        int durationSecs = Math.max(0, Math.min(3600, cfg.optInt("durationSecs", 0)));
        JSONArray framesArr = cfg.getJSONArray("frames");
        int count = framesArr.length();
        if (count < 1 || count > 180) return;
        int[][] frames = new int[count][391];
        for (int i = 0; i < count; i++) {
            JSONArray row = framesArr.getJSONArray(i);
            for (int p = 0; p < 391; p++) {
                frames[i][p] = row.getInt(p);
            }
        }

        IBinder binder = getDisplayExtBinder();
        prepareAlwaysOnOutput(binder, brightness);

        // Single-frame static badge/image: latch directly in firmware display_ext and exit
        if (count == 1 && durationSecs == 0) {
            pushFrame(binder, frames[0]);
            writeStatus(true, "mcp_static", jobId, title, fps, brightness, 1, "", "");
            return;
        }

        int effectiveDurationSecs = (durationSecs <= 0 || durationSecs > 120) ? 60 : durationSecs;
        long endAtMs = System.currentTimeMillis() + effectiveDurationSecs * 1000L;

        PowerManager power = Bridge.context != null ? Bridge.context.getSystemService(PowerManager.class) : null;
        PowerManager.WakeLock wakeLock = null;
        if (power != null) {
            try {
                wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "StepX:BackScreenMcpLoop");
                wakeLock.setReferenceCounted(false);
            } catch (Throwable ignored) {
            }
        }

        long startedMs = SystemClock.elapsedRealtime();
        long lastWakeRenew = 0;
        long lastStatusWrite = 0;
        long lastThermalCheck = 0;
        long frameCount = 0;
        int[] prevFrame = null;
        long intervalMs = Math.max(80L, 1000L / fps);

        try {
            while (true) {
                String activeJob = readActiveJobId();
                if (!jobId.equals(activeJob)) {
                    // Superseded by another job or stopped
                    return;
                }
                long now = SystemClock.elapsedRealtime();
                long elapsedMs = now - startedMs;
                if (System.currentTimeMillis() >= endAtMs || elapsedMs >= effectiveDurationSecs * 1000L) {
                    boolean restored = restoreSavedState(binder, true);
                    JOB_FILE.delete();
                    writeStatus(false, "stopped", jobId, title, fps, brightness, frameCount, "",
                            restored ? "" : "原背屏设置恢复失败，快照已保留，请重试");
                    return;
                }
                if (power != null && (lastThermalCheck == 0 || now - lastThermalCheck >= 3_000L)) {
                    lastThermalCheck = now;
                    try {
                        if (power.isPowerSaveMode()
                                || power.getCurrentThermalStatus() >= PowerManager.THERMAL_STATUS_MODERATE) {
                            boolean restored = restoreSavedState(binder, true);
                            JOB_FILE.delete();
                            writeStatus(false, "stopped", jobId, title, fps, brightness, frameCount, "thermal_or_power_save",
                                    restored ? "" : "原背屏设置恢复失败，快照已保留，请重试");
                            break;
                        }
                    } catch (Throwable ignored) {
                    }
                }
                if (wakeLock != null && (!wakeLock.isHeld() || now - lastWakeRenew >= 25_000L)) {
                    try {
                        wakeLock.acquire(60_000L);
                        lastWakeRenew = now;
                    } catch (Throwable ignored) {
                    }
                }
                int idx = (int) (((elapsedMs * fps) / 1000L) % count);
                int[] nextFrame = frames[idx];
                if (!Arrays.equals(prevFrame, nextFrame)) {
                    pushFrame(binder, nextFrame);
                    prevFrame = nextFrame;
                    frameCount++;
                }
                if (frameCount == 1 || now - lastStatusWrite >= 30_000L) {
                    writeStatus(true, "mcp_animation", jobId, title, fps, brightness, frameCount, "", "");
                    lastStatusWrite = now;
                }
                long nextTick = startedMs + ((elapsedMs / intervalMs) + 1) * intervalMs;
                long sleepMs = Math.max(15L, Math.min(intervalMs, nextTick - SystemClock.elapsedRealtime()));
                Thread.sleep(sleepMs);
            }
        } finally {
            if (wakeLock != null && wakeLock.isHeld()) {
                try {
                    wakeLock.release();
                } catch (Throwable ignored) {
                }
            }
        }
    }
}
