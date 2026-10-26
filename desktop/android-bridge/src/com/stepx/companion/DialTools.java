package com.stepx.companion;

import android.Manifest;
import android.content.Context;
import android.content.ContextWrapper;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Bundle;
import android.telecom.PhoneAccount;
import android.telecom.PhoneAccountHandle;
import android.telecom.TelecomManager;
import android.telephony.SubscriptionInfo;
import android.telephony.SubscriptionManager;
import android.telephony.TelephonyManager;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.List;

/** Genuine system calling, with an explicit prepare/confirm boundary. */
final class DialTools {
    private static volatile java.lang.reflect.Constructor<TelephonyManager> telephonyConstructor;

    private static java.lang.reflect.Constructor<TelephonyManager> getTelephonyConstructor() throws Exception {
        java.lang.reflect.Constructor<TelephonyManager> c = telephonyConstructor;
        if (c == null) {
            c = TelephonyManager.class.getConstructor(Context.class);
            telephonyConstructor = c;
        }
        return c;
    }

    static Context callingContext() {
        // createPackageContext retains ActivityThread's system op-package under
        // app_process. Attribute Telecom operations to the actual shell UID.
        return new ContextWrapper(Bridge.context) {
            @Override public String getOpPackageName() { return "com.android.shell"; }
            @Override public String getPackageName() { return "com.android.shell"; }
            @Override public Context getApplicationContext() { return this; }
            @Override public android.content.AttributionSource getAttributionSource() {
                return new android.content.AttributionSource.Builder(android.os.Process.myUid())
                    .setPackageName("com.android.shell").build();
            }
        };
    }

    static TelecomManager telecom() throws Exception {
        Bridge.capabilities(); // Initializes telephony services for app_process.
        if (Bridge.context.checkSelfPermission(Manifest.permission.CALL_PHONE) != PackageManager.PERMISSION_GRANTED)
            throw new SecurityException("手机未授予调试通道拨号权限");
        TelecomManager manager = TelecomManager.class.getConstructor(Context.class).newInstance(callingContext());
        if (manager == null) throw new IllegalStateException("手机通话服务不可用");
        return manager;
    }

    static String accountKey(PhoneAccountHandle handle) {
        return handle.getComponentName().flattenToString() + ":" + handle.getId() + ":" + handle.getUserHandle();
    }

    static int subscription(PhoneAccountHandle handle) throws Exception {
        TelephonyManager manager = getTelephonyConstructor().newInstance(callingContext())
            .createForPhoneAccountHandle(handle);
        return manager == null ? SubscriptionManager.INVALID_SUBSCRIPTION_ID : manager.getSubscriptionId();
    }

    static List<PhoneAccountHandle> accounts(TelecomManager manager) throws Exception {
        List<PhoneAccountHandle> result = new ArrayList<>();
        for (PhoneAccountHandle handle : manager.getCallCapablePhoneAccounts()) {
            PhoneAccount account = manager.getPhoneAccount(handle);
            if (account != null && account.isEnabled() && account.hasCapabilities(PhoneAccount.CAPABILITY_SIM_SUBSCRIPTION)
                && account.supportsUriScheme(PhoneAccount.SCHEME_TEL) && subscription(handle) >= 0) result.add(handle);
        }
        return result;
    }

    static String label(PhoneAccountHandle handle, TelecomManager manager) throws Exception {
        return label(handle, subscription(handle), manager);
    }

    static String label(PhoneAccountHandle handle, int sub, TelecomManager manager) throws Exception {
        SubscriptionInfo info = Bridge.context.getSystemService(SubscriptionManager.class).getActiveSubscriptionInfo(sub);
        if (info != null) return "卡槽 " + (info.getSimSlotIndex() + 1) + " · " + info.getDisplayName();
        return String.valueOf(manager.getPhoneAccount(handle).getLabel());
    }

    // Read-only diagnostic; deliberately has no phone-number argument.
    static JSONObject inspectAccounts() throws Exception {
        TelecomManager manager = telecom();
        JSONArray result = new JSONArray();
        for (PhoneAccountHandle handle : manager.getCallCapablePhoneAccounts()) {
            PhoneAccount account = manager.getPhoneAccount(handle);
            if (account != null && account.isEnabled() && account.hasCapabilities(PhoneAccount.CAPABILITY_SIM_SUBSCRIPTION)
                && account.supportsUriScheme(PhoneAccount.SCHEME_TEL)) {
                int sub = subscription(handle);
                if (sub >= 0) {
                    result.put(new JSONObject().put("subscription", sub).put("label", label(handle, sub, manager)));
                }
            }
        }
        return new JSONObject().put("accounts", result).put("default_subscription", SubscriptionManager.getDefaultVoiceSubscriptionId())
            .put("in_call", manager.isInCall());
    }

    static String number(JSONObject request) throws Exception {
        String number = request.getString("number");
        if (!number.matches("\\+?[0-9]{2,40}")) throw new IllegalArgumentException("电话号码无效；服务代码请在手机拨号");
        if (Bridge.context.getSystemService(TelephonyManager.class).isEmergencyNumber(number))
            throw new IllegalArgumentException("紧急呼叫请直接使用手机拨号界面");
        return number;
    }

    static PhoneAccountHandle select(JSONObject request, TelecomManager manager) throws Exception {
        List<PhoneAccountHandle> available = accounts(manager);
        int selected = request.optInt("subscription", -1);
        PhoneAccountHandle defaultAccount = manager.getDefaultOutgoingPhoneAccount(PhoneAccount.SCHEME_TEL);
        for (PhoneAccountHandle handle : available) {
            if (selected >= 0 ? subscription(handle) == selected : handle.equals(defaultAccount)) return handle;
        }
        if (selected < 0 && available.size() == 1) return available.get(0);
        if (available.isEmpty()) throw new IllegalStateException("没有可用的通话 SIM 卡，请检查手机 SIM 状态");
        throw new IllegalStateException("请选择可用的通话 SIM 卡后重试");
    }

    static JSONObject prepare(JSONObject request) throws Exception {
        TelecomManager manager = telecom();
        number(request);
        if (manager.isInCall()) throw new IllegalStateException("手机已有通话，请结束后再拨号");
        PhoneAccountHandle handle = select(request, manager);
        int sub = subscription(handle);
        return new JSONObject().put("subscription", sub).put("subscription_label", label(handle, sub, manager))
            .put("account_key", accountKey(handle));
    }

    static JSONObject call(JSONObject request) throws Exception {
        String id = request.getString("id");
        if (!id.matches("[a-f0-9]{32}")) throw new IllegalArgumentException("呼叫请求标识无效");
        File stateFile = new File(Bridge.ROOT, "call-" + id + ".json");
        if (stateFile.exists()) return readState(stateFile); // Never place the same request twice.
        TelecomManager manager = telecom();
        String number = number(request);
        if (manager.isInCall()) throw new IllegalStateException("手机已有通话，请结束后再拨号");
        PhoneAccountHandle handle = select(request, manager);
        if (!accountKey(handle).equals(request.getString("account_key")))
            throw new IllegalStateException("手机通话 SIM 已变更，请重新确认呼叫");
        if (!stateFile.createNewFile()) return readState(stateFile);
        JSONObject state = new JSONObject().put("state", "unknown");
        Bridge.writeStatus(stateFile, state);
        Bundle extras = new Bundle();
        extras.putParcelable(TelecomManager.EXTRA_PHONE_ACCOUNT_HANDLE, handle);
        manager.placeCall(Uri.fromParts(PhoneAccount.SCHEME_TEL, number, null), extras);
        state.put("state", "submitted");
        Bridge.writeStatus(stateFile, state);
        return state;
    }

    static JSONObject readState(File file) throws Exception {
        if (file.length() == 0) return new JSONObject().put("state", "unknown");
        return new JSONObject(new String(Files.readAllBytes(file.toPath()), StandardCharsets.UTF_8));
    }
}
