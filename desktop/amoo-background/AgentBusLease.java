package stepx;

import android.app.Application;
import android.app.Service;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.os.PowerManager;
import android.os.SystemClock;
import org.json.JSONObject;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.HashMap;
import java.util.Map;

/** Process-scoped adapter for one verified AgentBus build. No display or account mutation. */
public final class AgentBusLease {
    private static final String PACKAGE = "com.stepx.companion.app";
    private static final String SOCKET = "stepx-amoo-lease-v1";
    private static final long MAX_TTL = 90_000L;
    private final Application app;
    private final Object gateway;
    private final String certificate;
    private final int uid;
    private final String wakeMethod;
    private final String jobActiveMethod;
    private final LocalServerSocket server;
    private final Map<String, Long> leases = new HashMap<>();
    private volatile boolean running = true;
    private boolean ownsOverride;
    private boolean pendingClose;
    private long lastReconnect;
    private String lastError = "";

    private AgentBusLease(Application app, Object gateway, String certificate, int uid) throws Exception {
        this.app = app; this.gateway = gateway; this.certificate = certificate; this.uid = uid;
        verifyCompanion();
        String[] methods = validateGatewayAbi(gateway);
        this.wakeMethod = methods[0];
        this.jobActiveMethod = methods[1];
        this.server = new LocalServerSocket(SOCKET);
    }

    static String[] validateGatewayAbi(Object gateway) throws Exception {
        String gatewayType = gateway.getClass().getName();
        boolean v12 = gatewayType.equals("jb.r1");
        if (!gatewayType.equals("o4.t1") && !v12) throw new SecurityException("Unsupported gateway: " + gatewayType);
        String wake = v12 ? "v" : "w";
        String jobActive = v12 ? "c" : "b";
        for (String name : new String[]{"F", "G", "I"}) {
            if (field(gateway, name).getType() != boolean.class) throw new SecurityException("Gateway ABI mismatch");
        }
        // Verify every native method before creating a server or changing any state.
        if (gateway.getClass().getMethod("f").getReturnType() != boolean.class)
            throw new SecurityException("Gateway pending ABI mismatch");
        if (field(gateway, "J").getType().getMethod(jobActive).getReturnType() != boolean.class)
            throw new SecurityException("Gateway job ABI mismatch");
        for (String name : new String[]{"x", "y"}) {
            Object connection = field(gateway, name).get(gateway);
            connection.getClass().getMethod(wake, boolean.class);
            connection.getClass().getMethod("f");
            connectionState(connection);
        }
        return new String[]{wake, jobActive};
    }

    static Object resolveGateway(Object service) throws Exception {
        for (String containerFieldName : new String[]{"c", "h"}) {
            Field containerField;
            try {
                containerField = field(service, containerFieldName);
            } catch (NoSuchFieldException ignored) {
                continue;
            }
            if (java.lang.reflect.Modifier.isStatic(containerField.getModifiers())) continue;
            Object container = containerField.get(service);
            if (container == null) continue;
            try {
                Object candidate = field(container, "f0").get(container);
                if (candidate != null) return candidate;
            } catch (NoSuchFieldException ignored) {
            }
        }
        return null;
    }

    public static String run(String certificate, int uid) {
        try {
            if (!certificate.matches("[0-9a-f]{64}") || uid < 10000 || uid >= 100000)
                throw new SecurityException("Invalid companion identity");
            Class<?> at = Class.forName("android.app.ActivityThread");
            Application app = (Application) at.getMethod("currentApplication").invoke(null);
            if (app == null || !app.getPackageName().equals("com.stepos.agentbus"))
                throw new SecurityException("Wrong target process");
            Object thread = at.getMethod("currentActivityThread").invoke(null);
            Map<?, ?> services = (Map<?, ?>) field(thread, "mServices").get(thread);
            Object gateway = null;
            for (Object value : services.values()) {
                if (value instanceof Service && value.getClass().getName().equals("com.stepos.agentbus.platform.AgentBusService")) {
                    gateway = resolveGateway(value);
                }
            }
            if (gateway == null) throw new IllegalStateException("AgentBus gateway not ready");
            AgentBusLease bridge = new AgentBusLease(app, gateway, certificate, uid);
            bridge.start();
            return bridge.status().put("installed", true).put("signature_verified", true).toString();
        } catch (Throwable error) {
            return failure(error).toString();
        }
    }

    private static Field field(Object target, String name) throws Exception {
        Class<?> type = target.getClass();
        while (type != null) {
            try { Field result = type.getDeclaredField(name); result.setAccessible(true); return result; }
            catch (NoSuchFieldException missing) { type = type.getSuperclass(); }
        }
        throw new NoSuchFieldException(name);
    }

    private static String hex(byte[] bytes) {
        StringBuilder result = new StringBuilder();
        for (byte value : bytes) result.append(String.format(java.util.Locale.ROOT, "%02x", value & 255));
        return result.toString();
    }

    private void verifyCompanion() throws Exception {
        PackageManager pm = app.getPackageManager();
        PackageInfo info = pm.getPackageInfo(PACKAGE, PackageManager.GET_SIGNING_CERTIFICATES);
        if (info.applicationInfo == null || info.applicationInfo.uid != uid || info.signingInfo == null)
            throw new SecurityException("Companion UID mismatch");
        String[] packages = pm.getPackagesForUid(uid);
        if (packages == null || packages.length != 1 || !PACKAGE.equals(packages[0]))
            throw new SecurityException("Shared companion UID is not allowed");
        Signature[] signers = info.signingInfo.getApkContentsSigners();
        if (signers.length != 1 || !certificate.equals(hex(MessageDigest.getInstance("SHA-256").digest(signers[0].toByteArray()))))
            throw new SecurityException("Companion certificate mismatch");
    }

    private boolean bool(String name) throws Exception { return field(gateway, name).getBoolean(gateway); }
    private boolean pending() throws Exception { return (Boolean) gateway.getClass().getMethod("f").invoke(gateway); }
    private boolean nativeWakeActive() throws Exception {
        Object job = field(gateway, "J").get(gateway);
        return job != null && (Boolean) job.getClass().getMethod(jobActiveMethod).invoke(job);
    }
    private static String connectionState(Object connection) throws Exception {
        Object flow = field(connection, "b").get(connection);
        Method getValue = flow.getClass().getMethod("getValue"); getValue.setAccessible(true);
        return String.valueOf(getValue.invoke(flow));
    }

    private synchronized void tick() throws Exception {
        long now = SystemClock.elapsedRealtime();
        leases.entrySet().removeIf(entry -> entry.getValue() <= now);
        if (!leases.isEmpty()) {
            // I is the existing native screen-off connection allowance. F/G remain real screen state.
            if (!bool("I")) {
                field(gateway, "I").setBoolean(gateway, true);
                ownsOverride = true;
            }
            pendingClose = false;
            if (now - lastReconnect >= 5_000L) {
                lastReconnect = now;
                for (String name : new String[]{"x", "y"}) {
                    Object connection = field(gateway, name).get(gateway);
                    String state = connectionState(connection);
                    if (!state.equals("ONLINE") && !state.equals("CONNECTING") && !state.equals("AUTHENTICATING"))
                        connection.getClass().getMethod(wakeMethod, boolean.class).invoke(connection, true);
                }
            }
        } else {
            restore();
        }
    }

    private void restore() throws Exception {
        if (ownsOverride) {
            // Never cancel J or a native CloudWake lease. Native code owns its timer and pending IDs.
            if (!nativeWakeActive()) {
                field(gateway, "I").setBoolean(gateway, false);
                // Preserve a genuine CloudWake which arrived concurrently with cleanup.
                if (nativeWakeActive()) field(gateway, "I").setBoolean(gateway, true);
                else pendingClose = true;
            }
            ownsOverride = false;
        }
        if (pendingClose && !bool("I") && !bool("F") && !bool("G") && !pending()) {
            for (String name : new String[]{"x", "y"}) {
                Object connection = field(gateway, name).get(gateway);
                // Recheck native ownership immediately before each connection is allowed to idle.
                if (!bool("I") && !nativeWakeActive() && !pending())
                    connection.getClass().getMethod("f").invoke(connection);
            }
            pendingClose = false;
        } else if (bool("F") || bool("G") || nativeWakeActive()) {
            pendingClose = false;
        }
    }

    private synchronized JSONObject status() throws Exception {
        long now = SystemClock.elapsedRealtime();
        long expires = 0;
        for (long deadline : leases.values()) expires = Math.max(expires, deadline - now);
        return new JSONObject().put("ok", true).put("version", 1).put("socket", SOCKET)
                .put("package", PACKAGE).put("uid", uid).put("pid", android.os.Process.myPid())
                .put("active", expires > 0).put("expires_in_ms", Math.max(0, expires)).put("leases", leases.size())
                .put("operator", connectionState(field(gateway, "x").get(gateway)))
                .put("node", connectionState(field(gateway, "y").get(gateway)))
                .put("screen_desired_on", bool("F")).put("screen_effective_on", bool("G"))
                .put("interactive", ((PowerManager) app.getSystemService(Application.POWER_SERVICE)).isInteractive())
                .put("override", bool("I")).put("native_wake_active", nativeWakeActive())
                .put("pending", pending()).put("running", running).put("error", lastError);
    }

    private synchronized JSONObject handle(JSONObject request) throws Exception {
        leases.entrySet().removeIf(entry -> entry.getValue() <= SystemClock.elapsedRealtime());
        String op = request.getString("op");
        if (op.equals("pairing_status") || op.equals("pairing_enable") || op.equals("pairing_disable") ||
                op.equals("pairing_begin") || op.equals("pairing_cancel")) {
            // The same verified companion may use only these fixed, typed ADB
            // pairing operations. No arbitrary Binder, shell, or settings interface.
            return ScopedPairing.handle(app, request).put("ok", true).put("version", 1)
                .put("pid", android.os.Process.myPid()).put("uid", uid);
        }
        if (op.equals("acquire") || op.equals("renew") || op.equals("release")) {
            String id = request.getString("lease_id");
            if (!id.matches("[0-9A-Za-z_-]{16,128}")) throw new IllegalArgumentException("Invalid lease ID");
            if (op.equals("release")) leases.remove(id);
            else {
                long ttl = request.getLong("ttl_ms");
                if (ttl < 1000 || ttl > MAX_TTL) throw new IllegalArgumentException("Lease TTL must be 1000..90000 ms");
                if (op.equals("renew") && !leases.containsKey(id)) throw new IllegalArgumentException("Unknown lease");
                if (!leases.containsKey(id) && leases.size() >= 16) throw new IllegalArgumentException("Too many leases");
                leases.put(id, SystemClock.elapsedRealtime() + ttl);
            }
        } else if (op.equals("shutdown")) {
            ScopedPairing.shutdown();
            leases.clear(); running = false;
        } else if (!op.equals("status")) throw new IllegalArgumentException("Unsupported operation");
        tick();
        return status();
    }

    private static JSONObject failure(Throwable error) {
        try { return new JSONObject().put("ok", false).put("error", error.getClass().getSimpleName() + ": " + String.valueOf(error.getMessage())); }
        catch (Exception impossible) { return new JSONObject(); }
    }

    private void start() {
        Thread watchdog = new Thread(() -> {
            while (true) {
                try {
                    synchronized (this) {
                        if (!running && !pendingClose) break;
                        tick();
                    }
                    Thread.sleep(1000);
                } catch (Throwable error) {
                    synchronized (this) {
                        lastError = error.getClass().getSimpleName(); leases.clear();
                        try { restore(); } catch (Throwable ignored) { }
                        running = false;
                    }
                    try { server.close(); } catch (Exception ignored) { }
                    break;
                }
            }
        }, "StepXAmooLeaseWatchdog");
        watchdog.setDaemon(true); watchdog.start();
        Thread listener = new Thread(() -> {
            try {
                while (running) {
                    try (LocalSocket client = server.accept()) {
                        client.setSoTimeout(2000);
                        if (client.getPeerCredentials().getUid() != uid) continue;
                        verifyCompanion();
                        JSONObject answer;
                        try {
                            InputStream input = client.getInputStream();
                            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                            int next;
                            while ((next = input.read()) != -1 && next != '\n') {
                                if (bytes.size() >= 2048) throw new IllegalArgumentException("Request too large");
                                bytes.write(next);
                            }
                            answer = handle(new JSONObject(bytes.toString("UTF-8")));
                        } catch (Throwable error) {
                            answer = failure(error).put("version", 1)
                                .put("pid", android.os.Process.myPid()).put("uid", uid);
                        }
                        OutputStream output = client.getOutputStream();
                        output.write((answer.toString() + "\n").getBytes(StandardCharsets.UTF_8)); output.flush();
                    } catch (Throwable error) {
                        synchronized (this) { lastError = error.getClass().getSimpleName(); }
                    }
                }
            } finally {
                synchronized (this) {
                    leases.clear();
                    try { restore(); } catch (Throwable ignored) { }
                    running = false;
                }
                try { server.close(); } catch (Exception ignored) { }
            }
        }, "StepXAmooLeaseListener");
        listener.setDaemon(true); listener.start();
    }
}
