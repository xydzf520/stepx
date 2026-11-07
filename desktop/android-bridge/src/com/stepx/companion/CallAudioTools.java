package com.stepx.companion;

import android.content.AttributionSource;
import android.content.Context;
import android.media.AudioFormat;
import android.media.AudioManager;
import android.media.AudioAttributes;
import android.media.AudioRecord;
import android.media.AudioTrack;
import android.os.Bundle;
import android.util.Base64;
import org.json.JSONObject;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;

/** PCM over the already authenticated ADB stream; never opens a network listener or records a file. */
final class CallAudioTools {
    static AudioManager audio() throws Exception {
        return AudioManager.class.getConstructor(Context.class).newInstance(DialTools.callingContext());
    }

    static JSONObject inspect() throws Exception {
        AudioManager manager = audio();
        boolean permission = Bridge.context.checkSelfPermission("android.permission.CALL_AUDIO_INTERCEPTION") == 0;
        boolean supported = permission && (Boolean)AudioManager.class
            .getMethod("isPstnCallAudioInterceptable").invoke(manager);
        return new JSONObject().put("pstn_interceptable", supported).put("mode", manager.getMode())
            .put("interception_permission", permission).put("sample_rate", 16000);
    }

    /** Keep one external provider reference, and bind the stream to a single live call UUID. */
    static final class CallLease implements AutoCloseable {
        final Class<?> iam = Class.forName("android.app.IActivityManager");
        final Object service = Class.forName("android.app.ActivityManager").getMethod("getService").invoke(null);
        final int user = (Integer)Class.forName("android.app.ActivityManager").getMethod("getCurrentUser").invoke(null);
        final android.os.IBinder token = new android.os.Binder();
        final String authority = "com.stepx.companion.app.push";
        final Object provider;
        final AttributionSource attribution = new AttributionSource.Builder(android.os.Process.myUid())
            .setPackageName("com.android.shell").build();
        CallLease() throws Exception {
            Object holder = iam.getMethod("getContentProviderExternal", String.class, int.class,
                android.os.IBinder.class, String.class).invoke(service, authority, user, token, "*cmd*");
            if (holder == null) throw new IllegalStateException("手机通话状态不可用");
            provider = holder.getClass().getField("provider").get(holder);
        }
        boolean active(String id) throws Exception {
            String request = Base64.encodeToString("{\"op\":\"call_state\"}".getBytes(java.nio.charset.StandardCharsets.UTF_8), Base64.NO_WRAP);
            Bundle response = (Bundle)Class.forName("android.content.IContentProvider").getMethod("call",
                AttributionSource.class, String.class, String.class, String.class, Bundle.class)
                .invoke(provider, attribution, authority, "request", request, null);
            JSONObject call = new JSONObject(new String(Base64.decode(response.getString("result"), Base64.DEFAULT), java.nio.charset.StandardCharsets.UTF_8));
            return call.optBoolean("available") && id.equals(call.optString("id"))
                && "active".equals(call.optString("state")) && !call.optBoolean("multiple_calls");
        }
        public void close() throws Exception {
            iam.getMethod("removeContentProviderExternalAsUser", String.class, android.os.IBinder.class, int.class)
                .invoke(service, authority, token, user);
        }
    }

    static void run(String id) throws Exception {
        if (!id.matches("[a-f0-9-]{36}")) throw new IllegalArgumentException("通话标识无效");
        AudioTrack track = null;
        AudioRecord record = null;
        AtomicBoolean running = new AtomicBoolean(true);
        AtomicLong lastInput = new AtomicLong(android.os.SystemClock.elapsedRealtime());
        try (CallLease lease = new CallLease()) {
            if (!lease.active(id)) throw new IllegalStateException("这次通话已结束或变化");
            AudioManager manager = audio();
            int mode = manager.getMode();
            if (!(Boolean)AudioManager.class.getMethod("isPstnCallAudioInterceptable").invoke(manager)
                || (mode != 2 && mode != 4 && mode != 5))
                throw new IllegalStateException("当前不是可转接的手机通话");
            AudioFormat output = new AudioFormat.Builder().setSampleRate(16000)
                .setEncoding(AudioFormat.ENCODING_PCM_16BIT).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build();
            AudioFormat input = new AudioFormat.Builder().setSampleRate(16000)
                .setEncoding(AudioFormat.ENCODING_PCM_16BIT).setChannelMask(AudioFormat.CHANNEL_IN_MONO).build();
            // AudioManager's convenience methods omit Builder.setContext(). In app_process
            // that attributes shell UID 2000 to package "android", rejected by AudioFlinger.
            // Use the same PSTN redirection builders with the real shell attribution.
            AudioAttributes.Builder attributes = new AudioAttributes.Builder().setContentType(AudioAttributes.CONTENT_TYPE_SPEECH);
            AudioAttributes.Builder.class.getMethod("setSystemUsage", int.class).invoke(attributes, 17); // USAGE_CALL_ASSISTANT
            AudioTrack.Builder trackBuilder = new AudioTrack.Builder().setAudioFormat(output).setAudioAttributes(attributes.build());
            AudioTrack.Builder.class.getMethod("setContext", Context.class).invoke(trackBuilder, DialTools.callingContext());
            AudioTrack.Builder.class.getMethod("setCallRedirectionMode", int.class).invoke(trackBuilder, 1); // PSTN
            track = trackBuilder.build();
            AudioRecord.Builder recordBuilder = new AudioRecord.Builder().setAudioFormat(input).setAudioSource(android.media.MediaRecorder.AudioSource.VOICE_DOWNLINK);
            AudioRecord.Builder.class.getMethod("setContext", Context.class).invoke(recordBuilder, DialTools.callingContext());
            AudioRecord.Builder.class.getMethod("setCallRedirectionMode", int.class).invoke(recordBuilder, 1);
            record = recordBuilder.build();
            final AudioTrack uplink = track;
            final AudioRecord downlink = record;
            record.startRecording(); track.play();
            if (record.getRecordingState() != AudioRecord.RECORDSTATE_RECORDING || track.getPlayState() != AudioTrack.PLAYSTATE_PLAYING)
                throw new IllegalStateException("手机通话音频未启动");
            if (!lease.active(id)) throw new IllegalStateException("通话已变化");
            Thread upload = new Thread(() -> {
                byte[] bytes = new byte[640];
                try { int count; while (running.get() && (count = System.in.read(bytes)) > 0) {
                    lastInput.set(android.os.SystemClock.elapsedRealtime());
                    int offset = 0;
                    while (offset < count && running.get()) {
                        int written = uplink.write(bytes, offset, count - offset);
                        if (written <= 0) throw new java.io.IOException("通话上行音频中断");
                        offset += written;
                    }
                }} catch (Throwable ignored) { } finally { running.set(false); }
            }, "stepx-call-uplink");
            Thread download = new Thread(() -> {
                byte[] bytes = new byte[640];
                try { while (running.get()) {
                    int count = downlink.read(bytes, 0, bytes.length);
                    if (count <= 0) throw new java.io.IOException("通话下行音频中断");
                    System.out.write(bytes, 0, count); System.out.flush();
                }} catch (Throwable ignored) { } finally { running.set(false); }
            }, "stepx-call-downlink");
            upload.setDaemon(true); download.setDaemon(true);
            System.err.println("STEPX_CALL_AUDIO_READY"); System.err.flush();
            upload.start(); download.start();
            // Closing the host ADB process cannot deliver EOF during a Wi-Fi partition.
            // Release the phone's audio redirection even if the TCP peer silently vanishes.
            while (running.get() && android.os.SystemClock.elapsedRealtime() - lastInput.get() < 5000
                && lease.active(id)) Thread.sleep(300);
        } finally {
            running.set(false);
            if (record != null) { try { record.stop(); } catch (Exception ignored) { } record.release(); }
            if (track != null) { try { track.stop(); } catch (Exception ignored) { } track.release(); }
        }
    }
}
