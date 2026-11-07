package stepx;

import android.content.Context;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.PermissionInfo;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.security.MessageDigest;
import java.util.Arrays;
import org.json.JSONObject;

/** Fixed-firmware, certificate-pinned grant. Does not alter package/SELinux identities. */
public final class CompanionGrant {
    private static final String PACKAGE = "com.stepx.companion.app";
    private static final String PERMISSION = "com.stepos.permission.MANAGE_BACK_SCREEN";
    private static final String[] CONNECTION_PERMISSIONS = {
        "android.permission.MANAGE_DEBUGGING", "android.permission.WRITE_SECURE_SETTINGS"
    };
    private static final String[] WAKE_PERMISSIONS = {"com.stepos.cloud.push.permission.MESSAGE"};
    private static Object field(Object object, String name) throws Exception {
        Field value = object.getClass().getDeclaredField(name);
        value.setAccessible(true);
        return value.get(object);
    }
    private static String sha256(byte[] value) throws Exception {
        StringBuilder result = new StringBuilder();
        for (byte b : MessageDigest.getInstance("SHA-256").digest(value))
            result.append(String.format("%02x", b & 255));
        return result.toString();
    }
    public static String run(String expectedSigner, boolean grant, int restoreFlags, int expectedUid) {
        JSONObject result = new JSONObject();
        try {
            if (expectedSigner == null || !expectedSigner.matches("[0-9a-f]{64}"))
                throw new SecurityException("Missing pinned signing certificate");
            ClassLoader loader = Thread.currentThread().getContextClassLoader();
            Class<?> local = Class.forName("com.android.server.LocalServices", false, loader);
            Class<?> api = Class.forName("com.android.server.pm.permission.PermissionManagerServiceInterface", false, loader);
            Object manager = local.getMethod("getService", Class.class).invoke(null, api);
            Object service = field(manager, "service"), policy = field(manager, "policy");
            Object lock = field(service, "stateLock"), persistence = field(service, "persistence"), allPolicy = field(service, "policy");
            Context context = (Context)service.getClass().getMethod("getContext").invoke(service);
            PackageManager pm = context.getPackageManager();
            PackageInfo pkg = pm.getPackageInfo(PACKAGE, PackageManager.GET_PERMISSIONS | PackageManager.GET_SIGNING_CERTIFICATES);
            if (pkg.signingInfo == null || pkg.signingInfo.getApkContentsSigners().length != 1 ||
                !expectedSigner.equals(sha256(pkg.signingInfo.getApkContentsSigners()[0].toByteArray())))
                throw new SecurityException("Installed companion signing certificate mismatch");
            int uid = pkg.applicationInfo.uid;
            if (uid < 10000 || uid >= 100000) throw new SecurityException("Expected user 0 application UID");
            if (uid != expectedUid) throw new SecurityException("Companion UID changed after preflight");
            String[] uidPackages = pm.getPackagesForUid(uid);
            if (uidPackages == null || uidPackages.length != 1 || !PACKAGE.equals(uidPackages[0]))
                throw new SecurityException("Shared companion UID is not supported");
            if (pkg.requestedPermissions == null || !Arrays.asList(pkg.requestedPermissions).contains(PERMISSION))
                throw new SecurityException("Companion did not declare backscreen permission");
            PermissionInfo permission = pm.getPermissionInfo(PERMISSION, 0);
            if (!"android".equals(permission.packageName) ||
                (permission.protectionLevel & PermissionInfo.PROTECTION_MASK_BASE) != PermissionInfo.PROTECTION_SIGNATURE)
                throw new SecurityException("Unexpected backscreen permission owner/protection");
            if (!grant) CompanionUpdateRuntime.stopInstalled();
            Class<?> access = Class.forName("com.android.server.permission.access.AccessState", false, loader);
            Class<?> mutable = Class.forName("com.android.server.permission.access.MutableAccessState", false, loader);
            Class<?> getScope = Class.forName("com.android.server.permission.access.GetStateScope", false, loader);
            Class<?> mutateScope = Class.forName("com.android.server.permission.access.MutateStateScope", false, loader);
            Constructor<?> getConstructor = getScope.getConstructor(access), mutateConstructor = mutateScope.getConstructor(access, mutable);
            Method get = policy.getClass().getMethod("getPermissionFlags", getScope, int.class, int.class, String.class);
            Method set = policy.getClass().getMethod("setPermissionFlags", mutateScope, int.class, int.class, String.class, int.class);
            Method write = persistence.getClass().getMethod("write", access);
            Method notify = allPolicy.getClass().getMethod("onStateMutated", getScope);
            Field state = service.getClass().getDeclaredField("state"); state.setAccessible(true);
            synchronized (lock) {
                Object old = state.get(service), readable = getConstructor.newInstance(old);
                int before = (Integer)get.invoke(policy, readable, uid, 0, PERMISSION);
                int after = grant ? ((before & ~2) | 4) : restoreFlags;
                if (!grant && before != ((restoreFlags & ~2) | 4) && before != restoreFlags)
                    throw new SecurityException("Permission changed since grant; refusing to overwrite");
                result.put("before_flags", before).put("after_flags", after);
                if (before != after) {
                    Object next = old.getClass().getMethod("toMutable").invoke(old);
                    set.invoke(policy, mutateConstructor.newInstance(old, next), uid, 0, PERMISSION, after);
                    write.invoke(persistence, next);
                    state.set(service, next);
                    notify.invoke(allPolicy, getConstructor.newInstance(next));
                }
            }
            int checked = pm.checkPermission(PERMISSION, PACKAGE);
            result.put("package", PACKAGE).put("permission", PERMISSION).put("uid", uid)
                .put("granted", checked == PackageManager.PERMISSION_GRANTED)
                .put("signature_verified", true).put("status", "success")
                .put("system_flag", (pm.getApplicationInfo(PACKAGE, 0).flags & 1) != 0);
            if (grant && checked != PackageManager.PERMISSION_GRANTED) throw new SecurityException("Grant read-back failed");
        } catch (Throwable failure) {
            try {
                Throwable cause = failure;
                while (cause.getCause() != null) cause = cause.getCause();
                result.put("status", "failed").put("error", cause.getClass().getSimpleName() + ": " + cause.getMessage());
            } catch (Exception ignored) {}
        }
        return result.toString();
    }

    /** Separate fixed permission group; never reuses or replaces the legacy backscreen baseline. */
    public static String runScoped(String expectedSigner, String operation, String baselineJson, int expectedUid) {
        return runFixed(expectedSigner, operation, baselineJson, expectedUid, 0);
    }

    /** Only the existing protected CloudWake receiver permission; separate recovery baseline. */
    public static String runWake(String expectedSigner, String operation, String baselineJson, int expectedUid) {
        return runFixed(expectedSigner, operation, baselineJson, expectedUid, 1);
    }

    /** Boot restoration is confined to the same four permissions and pinned companion identity. */
    public static String runBoot(String expectedSigner, String operation, String baselineJson, int expectedUid) {
        if((!"StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys".equals(android.os.Build.FINGERPRINT)&&!"StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/12:user/release-keys".equals(android.os.Build.FINGERPRINT))||android.os.Build.VERSION.SDK_INT!=36)
            return "{\"status\":\"failed\",\"error\":\"Unsupported boot firmware\"}";
        String raw=runFixed(expectedSigner, operation, baselineJson, expectedUid, 2);
        try {
            JSONObject result=new JSONObject(raw);
            if("grant".equals(operation)&&"success".equals(result.optString("status"))&&result.optBoolean("all_granted")){
                result.put("runtime",CompanionRuntime.ensure(expectedSigner,expectedUid));
                PairingDaemon.installSystem(expectedSigner,expectedUid);
                result.put("boot_pairing_started",true);
                result.put("companion_update",CompanionUpdateRuntime.install(expectedSigner,expectedUid,baselineJson));
                try { result.put("google_login",GoogleLoginRuntime.install()); }
                catch(Exception optional) { result.put("google_login",new JSONObject().put("status","needs_google_repair").put("error",optional.getClass().getSimpleName())); }
            }
            if("revoke".equals(operation)&&"success".equals(result.optString("status"))) {
                GoogleLoginRuntime.stopInstalled();
                result.put("runtime",CompanionRuntime.restore(expectedSigner,expectedUid));
            }
            return result.toString();
        } catch(Exception error){try{return new JSONObject(raw).put("status","failed").put("error","Boot pairing: "+error.toString()).toString();}catch(Exception ignored){return raw;}}
    }

    // Called only by the system package-update observer installed after runBoot.
    // No boot hook, power exemption, Google action, or app preference is changed.
    static String restoreAfterUpdate(String signer,String baseline,int uid) {
        return runFixed(signer,"grant",baseline,uid,2);
    }

    // Permission names/owners are never supplied by the caller.
    private static String runFixed(String expectedSigner, String operation, String baselineJson, int expectedUid, int group) {
        String[] permissions = group == 2 ? new String[]{PERMISSION, CONNECTION_PERMISSIONS[0], CONNECTION_PERMISSIONS[1], WAKE_PERMISSIONS[0]} : group == 1 ? WAKE_PERMISSIONS : CONNECTION_PERMISSIONS;
        
        String scope = group == 2 ? "companion_boot_v1" : group == 1 ? "wake_permission_v1" : "connection_permissions_v1";
        JSONObject result = new JSONObject();
        try {
            if (expectedSigner == null || !expectedSigner.matches("[0-9a-f]{64}"))
                throw new SecurityException("Missing pinned signing certificate");
            if (!Arrays.asList("inspect", "grant", "revoke").contains(operation))
                throw new SecurityException("Unsupported fixed permission operation");
            boolean inspect = operation.equals("inspect"), grant = operation.equals("grant");
            JSONObject baseline = new JSONObject(baselineJson);
            if (!inspect && baseline.length() != permissions.length)
                throw new SecurityException("Exact fixed permission baseline is required");
            ClassLoader loader = Thread.currentThread().getContextClassLoader();
            Class<?> local = Class.forName("com.android.server.LocalServices", false, loader);
            Class<?> api = Class.forName("com.android.server.pm.permission.PermissionManagerServiceInterface", false, loader);
            Object manager = local.getMethod("getService", Class.class).invoke(null, api);
            Object service = field(manager, "service"), policy = field(manager, "policy");
            Object lock = field(service, "stateLock"), persistence = field(service, "persistence"), allPolicy = field(service, "policy");
            Context context = (Context)service.getClass().getMethod("getContext").invoke(service);
            PackageManager pm = context.getPackageManager();
            PackageInfo pkg = pm.getPackageInfo(PACKAGE, PackageManager.GET_PERMISSIONS | PackageManager.GET_SIGNING_CERTIFICATES);
            if (pkg.signingInfo == null || pkg.signingInfo.getApkContentsSigners().length != 1 ||
                !expectedSigner.equals(sha256(pkg.signingInfo.getApkContentsSigners()[0].toByteArray())))
                throw new SecurityException("Installed companion signing certificate mismatch");
            int uid = pkg.applicationInfo.uid;
            if (uid < 10000 || uid >= 100000 || uid != expectedUid)
                throw new SecurityException("Companion UID changed or is outside user 0");
            String[] uidPackages = pm.getPackagesForUid(uid);
            if (uidPackages == null || uidPackages.length != 1 || !PACKAGE.equals(uidPackages[0]))
                throw new SecurityException("Shared companion UID is not supported");
            for (String name : permissions) {
                if (pkg.requestedPermissions == null || !Arrays.asList(pkg.requestedPermissions).contains(name))
                    throw new SecurityException("Companion did not declare " + name);
                PermissionInfo info = pm.getPermissionInfo(name, 0);
                if (!(name.equals(WAKE_PERMISSIONS[0]) ? "com.stepos.pushservice" : "android").equals(info.packageName) ||
                    (info.protectionLevel & PermissionInfo.PROTECTION_MASK_BASE) != PermissionInfo.PROTECTION_SIGNATURE)
                    throw new SecurityException("Unexpected permission owner/protection: " + name);
                if (!inspect) {
                    Object value = baseline.get(name);
                    if (!(value instanceof Integer)) throw new SecurityException("Invalid permission baseline flags");
                }
            }
            // Retire the observer before revocation, including one-group rollbacks.
            // A queued package update must not undo an explicit revoke operation.
            if (operation.equals("revoke")) CompanionUpdateRuntime.stopInstalled();
            Class<?> access = Class.forName("com.android.server.permission.access.AccessState", false, loader);
            Class<?> mutable = Class.forName("com.android.server.permission.access.MutableAccessState", false, loader);
            Class<?> getScope = Class.forName("com.android.server.permission.access.GetStateScope", false, loader);
            Class<?> mutateScope = Class.forName("com.android.server.permission.access.MutateStateScope", false, loader);
            Constructor<?> getConstructor = getScope.getConstructor(access), mutateConstructor = mutateScope.getConstructor(access, mutable);
            Method get = policy.getClass().getMethod("getPermissionFlags", getScope, int.class, int.class, String.class);
            Method set = policy.getClass().getMethod("setPermissionFlags", mutateScope, int.class, int.class, String.class, int.class);
            Method write = persistence.getClass().getMethod("write", access);
            Method notify = allPolicy.getClass().getMethod("onStateMutated", getScope);
            Field state = service.getClass().getDeclaredField("state"); state.setAccessible(true);
            JSONObject details = new JSONObject();
            synchronized (lock) {
                Object old = state.get(service), readable = getConstructor.newInstance(old);
                int[] before = new int[permissions.length], after = new int[permissions.length];
                boolean changed = false;
                // Validate every permission before performing even one mutation.
                for (int i = 0; i < permissions.length; i++) {
                    String name = permissions[i];
                    before[i] = (Integer)get.invoke(policy, readable, uid, 0, name);
                    if (inspect) after[i] = before[i];
                    else {
                        int original = baseline.getInt(name), expectedGranted = (original & ~2) | 4;
                        if (before[i] != original && before[i] != expectedGranted)
                            throw new SecurityException("Permission changed since baseline: " + name);
                        after[i] = grant ? expectedGranted : original;
                    }
                    changed |= before[i] != after[i];
                    details.put(name, new JSONObject().put("before_flags", before[i]).put("after_flags", after[i])
                        .put("owner", name.equals(WAKE_PERMISSIONS[0]) ? "com.stepos.pushservice" : "android").put("declared", true).put("signature_permission", true));
                }
                if (changed) {
                    Object next = old.getClass().getMethod("toMutable").invoke(old);
                    Object mutation = mutateConstructor.newInstance(old, next);
                    for (int i = 0; i < permissions.length; i++)
                        if (before[i] != after[i]) set.invoke(policy, mutation, uid, 0, permissions[i], after[i]);
                    write.invoke(persistence, next);
                    state.set(service, next);
                    notify.invoke(allPolicy, getConstructor.newInstance(next));
                }
                Object verified = getConstructor.newInstance(state.get(service));
                for (int i = 0; i < permissions.length; i++) {
                    int actual = (Integer)get.invoke(policy, verified, uid, 0, permissions[i]);
                    if (actual != after[i]) throw new SecurityException("Permission flags read-back failed");
                }
            }
            boolean allGranted = true;
            for (String name : permissions) {
                boolean granted = pm.checkPermission(name, PACKAGE) == PackageManager.PERMISSION_GRANTED;
                allGranted &= granted;
                details.getJSONObject(name).put("granted", granted);
                if (grant && !granted) throw new SecurityException("Permission grant read-back failed: " + name);
            }
            result.put("status", "success").put("operation", operation).put("scope", scope)
                .put("package", PACKAGE).put("uid", uid).put("signature_verified", true)
                .put("permissions", details).put("all_granted", allGranted)
                .put("system_flag", (pm.getApplicationInfo(PACKAGE, 0).flags & 1) != 0);
        } catch (Throwable failure) {
            try {
                Throwable cause = failure;
                while (cause.getCause() != null) cause = cause.getCause();
                result.put("status", "failed").put("error", cause.getClass().getSimpleName() + ": " + cause.getMessage());
            } catch (Exception ignored) {}
        }
        return result.toString();
    }
}
