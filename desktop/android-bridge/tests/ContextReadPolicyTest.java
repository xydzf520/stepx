package com.stepx.companion;

public final class ContextReadPolicyTest {
    private static void check(boolean ok) { if (!ok) throw new AssertionError(); }
    public static void main(String[] args) {
        check(ContextReadPolicy.pageLimit(-1) == 1);
        check(ContextReadPolicy.pageLimit(500) == 500);
        check(ContextReadPolicy.pageLimit(5000) == 1000);
        long now = 2_000_000_000_000L;
        check(ContextReadPolicy.calendarStart(now) == now - 7L * ContextReadPolicy.DAY_MS);
        check(ContextReadPolicy.calendarEnd(now) > now + 31L * ContextReadPolicy.DAY_MS);
        ContextReadPolicy.checkCalendarWindow(ContextReadPolicy.calendarStart(now), ContextReadPolicy.calendarEnd(now));
        for (long[] range : new long[][]{{-1,10},{10,10},{20,10},{0,367L*ContextReadPolicy.DAY_MS}}) {
            boolean rejected = false;
            try { ContextReadPolicy.checkCalendarWindow(range[0], range[1]); }
            catch (IllegalArgumentException expected) { rejected = true; }
            check(rejected);
        }
        check(ContextReadPolicy.resolveSkipHolidays(null, null) == 0);
        check(ContextReadPolicy.resolveStatutoryHolidays(null, null) == 0);
        check(ContextReadPolicy.resolveSkipHolidays(Boolean.TRUE, null) == 1);
        check(ContextReadPolicy.resolveStatutoryHolidays(Boolean.TRUE, null) == 1);
        check(ContextReadPolicy.resolveSkipHolidays(1, null) == 1);
        check(ContextReadPolicy.resolveStatutoryHolidays(null, 1) == 1);
        check(ContextReadPolicy.resolveSkipHolidays(Boolean.FALSE, 1) == 0);
        check(ContextReadPolicy.resolveDailyEnabled(null) == 0);
        check(ContextReadPolicy.resolveDailyEnabled(Boolean.TRUE) == 1);
        check(ContextReadPolicy.resolveDailyEnabled(1) == 1);
        check(ContextReadPolicy.resolveDailyEnabled(Boolean.FALSE) == 0);
        check(ContextReadPolicy.alarmFlag(Boolean.TRUE, 0) == 1);
        check(ContextReadPolicy.alarmFlag("1", 0) == 1);
        check(ContextReadPolicy.alarmFlag(0, 1) == 0);
        check(ContextReadPolicy.isAudioRecordingName("call_20261031.m4a"));
        check(ContextReadPolicy.isAudioRecordingName("meeting.MP3"));
        check(!ContextReadPolicy.isAudioRecordingName(".hidden.m4a"));
        check(!ContextReadPolicy.isAudioRecordingName("notes.txt"));
        System.out.println("ContextReadPolicyTest: 26 assertions passed");
    }
}
