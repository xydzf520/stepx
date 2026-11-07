package com.stepx.companion.app;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.os.BatteryManager;
import android.os.SystemClock;
import org.json.JSONObject;

/** System battery broadcasts update memory only: no timers, wake locks or network work. */
final class BatteryStateCache {
    private static Context registeredContext;
    private static int level = -1;
    private static int status = -1;
    private static int plugged = -1;
    private static long checkedAt = -1L;
    private static long verifiedAt = -1L;
    private static final long VERIFY_INTERVAL_MS = 30_000L;
    private static final BroadcastReceiver receiver = new BroadcastReceiver() {
        @Override public void onReceive(Context context, Intent intent) { accept(intent); }
    };

    private static synchronized void accept(Intent intent) {
        if (intent == null || !Intent.ACTION_BATTERY_CHANGED.equals(intent.getAction())) return;
        int raw = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1);
        int scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1);
        level = raw >= 0 && scale > 0 && raw <= scale ? (int) (100L * raw / scale) : -1;
        status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1);
        plugged = intent.getIntExtra(BatteryManager.EXTRA_PLUGGED, -1);
    }

    static synchronized void start(Context context) {
        if (registeredContext != null || context == null) return;
        Context app = context.getApplicationContext();
        try {
            Intent sticky = app.registerReceiver(receiver, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
            registeredContext = app;
            accept(sticky);
            checkedAt = SystemClock.elapsedRealtime();
            if (level >= 0) verifiedAt = checkedAt;
        } catch (RuntimeException ignored) { level = -1; }
    }

    static synchronized void stop() {
        if (registeredContext != null) {
            try { registeredContext.unregisterReceiver(receiver); } catch (RuntimeException ignored) {}
        }
        registeredContext = null;
        level = -1;
        status = -1;
        plugged = -1;
        checkedAt = verifiedAt = -1L;
    }

    static synchronized JSONObject snapshot(Context context) throws Exception {
        start(context);
        long now = SystemClock.elapsedRealtime();
        // A sleeping phone can defer receiver delivery. Check the system's
        // sticky value at most once per 30s, only on an existing desktop read.
        // No independent timer, polling connection, or phone wake-up is used.
        if (checkedAt < 0L || now - checkedAt >= VERIFY_INTERVAL_MS) {
            checkedAt = now;
            try {
                Intent sticky = context.getApplicationContext().registerReceiver(
                    null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
                if (sticky != null) {
                    accept(sticky);
                    if (level >= 0) verifiedAt = now;
                }
            } catch (RuntimeException ignored) { /* Keep the last known reading. */ }
        }
        boolean fresh = verifiedAt >= 0L && now - verifiedAt < 3 * VERIFY_INTERVAL_MS;
        return new JSONObject()
            .put("level", level >= 0 && fresh ? level : JSONObject.NULL)
            .put("status", status)
            .put("plugged", plugged);
    }
}
