package com.stepx.companion.app;

import android.app.Notification;
import android.app.Person;
import android.content.Context;
import android.os.Bundle;
import android.telephony.PhoneStateListener;
import android.telephony.TelephonyManager;
import org.json.JSONObject;

/** Ephemeral call state. Every transition travels over the existing push connection. */
final class PhoneCallMonitor {
    private static String id = "", number = "", name = "", state = "unknown";
    private static long startedMs;
    private static boolean available;
    private static boolean multipleCalls;
    private String dialerPackage = "";
    private final PushNotificationService service;
    private TelephonyManager manager;
    private PhoneStateListener listener;

    PhoneCallMonitor(PushNotificationService service) { this.service = service; }

    void start() {
        try {
            manager = service.getSystemService(TelephonyManager.class);
            dialerPackage = service.getSystemService(android.telecom.TelecomManager.class).getDefaultDialerPackage();
            listener = new PhoneStateListener() {
                @Override public void onCallStateChanged(int value, String incomingNumber) {
                    update(value, incomingNumber);
                }
            };
            manager.listen(listener, PhoneStateListener.LISTEN_CALL_STATE);
        } catch (Exception denied) {
            synchronized (PhoneCallMonitor.class) { available = false; state = "unknown"; }
        }
    }

    void stop() {
        try { if (manager != null && listener != null) manager.listen(listener, PhoneStateListener.LISTEN_NONE); }
        catch (Exception ignored) { }
        synchronized (PhoneCallMonitor.class) { available = false; state = "unknown"; }
    }

    private void update(int value, String incomingNumber) {
        String next = value == TelephonyManager.CALL_STATE_RINGING ? "ringing"
            : value == TelephonyManager.CALL_STATE_OFFHOOK ? "active" : "idle";
        boolean changed;
        synchronized (PhoneCallMonitor.class) {
            String incoming = incomingNumber == null ? "" : incomingNumber.trim();
            changed = !available || !state.equals(next) || (!incoming.isEmpty() && !incoming.equals(number));
            if ("idle".equals(next)) { id = ""; number = ""; name = ""; startedMs = 0; multipleCalls = false; }
            else {
                boolean nextCaller = "ringing".equals(next) && (!"ringing".equals(state)
                    || (!incoming.isEmpty() && !number.isEmpty() && !incoming.equals(number)));
                if ("active".equals(state) && "ringing".equals(next)) multipleCalls = true;
                if (id.isEmpty() || "idle".equals(state) || "unknown".equals(state) || nextCaller) {
                    startedMs = System.currentTimeMillis();
                    id = java.util.UUID.randomUUID().toString();
                    number = ""; name = "";
                }
                if (!incoming.isEmpty()) number = incoming;
            }
            state = next;
            available = true;
        }
        if (changed) publish();
    }

    void notification(String packageName, Notification notification) {
        if (dialerPackage == null || !dialerPackage.equals(packageName)) return;
        if (!Notification.CATEGORY_CALL.equals(notification.category)) return;
        Bundle extras = notification.extras;
        if (extras == null) return;
        Person person = extras.getParcelable("android.callPerson", Person.class);
        if (person == null) return;
        boolean changed = false;
        synchronized (PhoneCallMonitor.class) {
            if (!"ringing".equals(state) && !"active".equals(state)) return;
            String uri = person.getUri();
            if (uri != null && uri.startsWith("tel:") && !number.isEmpty()
                && !android.telephony.PhoneNumberUtils.compare(number, android.net.Uri.decode(uri.substring(4)))) return;
            if (uri != null && uri.startsWith("tel:") && !uri.substring(4).equals(number)) {
                number = android.net.Uri.decode(uri.substring(4)); changed = true;
            }
            String label = person.getName() == null ? "" : person.getName().toString().trim();
            if (!label.isEmpty() && !label.equals(name)) { name = label; changed = true; }
        }
        if (changed) publish();
    }

    private void publish() {
        try {
            PushStore store = PushStore.get(service);
            store.callStateChanged();
            service.broadcastDelta(new JSONObject().put("type", "call_state").put("revision", store.getRevision()));
        } catch (Exception ignored) { }
    }

    static synchronized JSONObject snapshot() {
        JSONObject result = new JSONObject();
        try {
            result.put("available", available).put("id", id).put("state", state)
                .put("number", number).put("name", name).put("started_ms", startedMs)
                .put("multiple_calls", multipleCalls)
                .put("observed_ms", System.currentTimeMillis());
        } catch (Exception ignored) { }
        return result;
    }
}
