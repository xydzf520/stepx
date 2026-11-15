package com.stepx.companion;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;

/** Bounded read-only windows; pure so cursor policy is testable without a device. */
final class ContextReadPolicy {
    static final long DAY_MS = 86_400_000L;
    static final int RECENT_CALLS_LIMIT = 10;
    static final int RECENT_RECORDINGS_LIMIT = 10;
    static final Set<String> AUDIO_EXT = new HashSet<>(Arrays.asList(
        "m4a", "mp3", "wav", "aac", "amr", "ogg", "flac", "3gp", "opus", "wma"
    ));

    static int pageLimit(int requested) { return Math.max(1, Math.min(1000, requested)); }
    static long calendarStart(long now) { return Math.max(0L, now - 7L * DAY_MS); }
    static long calendarEnd(long now) { return now + 32L * DAY_MS; }
    static void checkCalendarWindow(long start, long end) {
        if (start < 0L || end <= start || end - start > 366L * DAY_MS) {
            throw new IllegalArgumentException("日历读取范围必须为 1 年内的有效时间段");
        }
    }

    static int alarmFlag(Object raw, int defaultValue) {
        if (raw == null) return defaultValue;
        if (raw instanceof Boolean) return ((Boolean) raw) ? 1 : 0;
        if (raw instanceof Number) return ((Number) raw).intValue() != 0 ? 1 : 0;
        String text = String.valueOf(raw).trim();
        if ("1".equals(text) || "true".equalsIgnoreCase(text) || "yes".equalsIgnoreCase(text) || "on".equalsIgnoreCase(text)) {
            return 1;
        }
        if ("0".equals(text) || "false".equalsIgnoreCase(text) || "no".equalsIgnoreCase(text) || "off".equalsIgnoreCase(text)) {
            return 0;
        }
        return defaultValue;
    }

    static int resolveSkipHolidays(Object skipHolidaysRaw, Object statutoryHolidaysRaw) {
        if (skipHolidaysRaw != null) return alarmFlag(skipHolidaysRaw, 0);
        return alarmFlag(statutoryHolidaysRaw, 0);
    }

    static int resolveStatutoryHolidays(Object skipHolidaysRaw, Object statutoryHolidaysRaw) {
        int skip = resolveSkipHolidays(skipHolidaysRaw, statutoryHolidaysRaw);
        if (statutoryHolidaysRaw != null) return alarmFlag(statutoryHolidaysRaw, skip);
        return skip;
    }

    static int resolveDailyEnabled(Object dailyEnabledRaw) {
        return alarmFlag(dailyEnabledRaw, 0);
    }

    static boolean isAudioRecordingName(String name) {
        if (name == null) return false;
        String trimmed = name.trim();
        if (trimmed.isEmpty() || trimmed.startsWith(".")) return false;
        String ext = fileExtension(trimmed);
        return !ext.isEmpty() && AUDIO_EXT.contains(ext);
    }

    static String fileExtension(String name) {
        if (name == null) return "";
        String trimmed = name.trim();
        int slash = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
        String base = slash >= 0 ? trimmed.substring(slash + 1) : trimmed;
        int dot = base.lastIndexOf('.');
        if (dot <= 0 || dot >= base.length() - 1) return "";
        return base.substring(dot + 1).toLowerCase(Locale.ROOT);
    }

    static String readBoundedLine(InputStream in, int maxBytes) throws IOException {
        if (in == null || maxBytes <= 0) return null;
        ByteArrayOutputStream out = new ByteArrayOutputStream(Math.min(maxBytes, 1024));
        int b;
        boolean any = false;
        while ((b = in.read()) != -1) {
            any = true;
            if (b == '\n') break;
            if (out.size() >= maxBytes) {
                throw new IllegalArgumentException("请求为空或过长");
            }
            out.write(b);
        }
        if (!any && out.size() == 0) return null;
        byte[] bytes = out.toByteArray();
        int len = bytes.length;
        if (len > 0 && bytes[len - 1] == '\r') len--;
        return new String(bytes, 0, len, StandardCharsets.UTF_8);
    }
}
