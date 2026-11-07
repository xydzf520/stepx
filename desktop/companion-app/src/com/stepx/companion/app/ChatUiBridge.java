package com.stepx.companion.app;

import android.accessibilityservice.AccessibilityService;
import android.accessibilityservice.GestureDescription;
import android.content.Context;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Path;
import android.graphics.Rect;
import android.graphics.RectF;
import android.os.Binder;
import android.os.Bundle;
import android.os.IBinder;
import android.os.Parcel;
import android.os.PowerManager;
import android.os.RemoteException;
import android.os.SystemClock;
import android.util.Base64;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityWindowInfo;
import android.widget.EditText;
import android.widget.TextView;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Multi-tier UI inspection and reply bridge for IM apps (WeChat, QQ, DingTalk, Feishu, etc.):
 * - Tier 1A: StepOS in-process ViewRootImpl bridge (StepOSViewContentUtils.apexRequest -> ChatUiBridge.handleApexRequest),
 *   which executes directly on the target app's UI thread even while the phone is locked or screen is off,
 *   and extracts text from custom IM views (NeatTextView, NoMeasuredTextView, MMTextView) + avatar descriptions.
 * - Tier 1B: StepOS system ability bridge (/system_ext/priv-app/ability/ability.apk RequestMain.handleApexRequest),
 *   providing a trusted system-partition DEX fallback when Android 16 restricts non-system DEX paths.
 * - Tier 2: GuiAccessibilityService window inspection & gesture/action fallback.
 */
public final class ChatUiBridge {
    private static final String APEX_CALLBACK_DESCRIPTOR = "com.stepx.companion.app.IApexCallback";
    private static final String READONLY_BRIDGE_DEX_PATH = "/data/local/tmp/stepx-companion-bridge.apk";

    private static volatile Boolean sApexAvailable;
    private static volatile boolean sSelfApexBlocked = false;
    private static volatile ClassLoader sAbilityClassLoader;

    private ChatUiBridge() {}

    /**
     * Returns true if StepOS's WindowManager-level ViewRootImpl bridge (StepOSViewContentUtils.apexRequest)
     * is present on this device, allowing inspection and interaction even when locked.
     */
    public static boolean isApexBridgeAvailable() {
        Boolean cached = sApexAvailable;
        if (Boolean.TRUE.equals(cached)) return true;
        try {
            Class<?> cls = Class.forName("android.view.StepOSViewContentUtils");
            Method m = cls.getMethod("apexRequest", Intent.class);
            boolean ok = (m != null);
            if (ok) {
                sApexAvailable = Boolean.TRUE;
            }
            return ok;
        } catch (Throwable ignored) {
            return false;
        }
    }

    public static final class UiBounds {
        public final int left;
        public final int top;
        public final int right;
        public final int bottom;

        public UiBounds(int left, int top, int right, int bottom) {
            this.left = left;
            this.top = top;
            this.right = right;
            this.bottom = bottom;
        }

        public static UiBounds fromRect(Rect r) {
            return r != null ? new UiBounds(r.left, r.top, r.right, r.bottom) : new UiBounds(0, 0, 0, 0);
        }

        public int width() {
            return right - left;
        }

        public int height() {
            return bottom - top;
        }

        public int centerX() {
            return (left + right) / 2;
        }

        public int centerY() {
            return (top + bottom) / 2;
        }
    }

    public static final class UiNode {
        public final String text;
        public final String desc;
        public final String hint;
        public final String className;
        public final String viewId;
        public final UiBounds bounds;
        public final boolean clickable;
        public final boolean editable;
        public final boolean scrollable;
        public final boolean visible;
        public final int depth;
        public final AccessibilityNodeInfo a11yNode;
        public String avatarB64 = "";
        public String mediaB64 = "";
        public final List<UiNode> children = new ArrayList<UiNode>();

        public UiNode(
                String text,
                String desc,
                String hint,
                String className,
                String viewId,
                UiBounds bounds,
                boolean clickable,
                boolean editable,
                boolean scrollable,
                boolean visible,
                int depth,
                AccessibilityNodeInfo a11yNode) {
            this(text, desc, hint, className, viewId, bounds, clickable, editable, scrollable, visible, depth, a11yNode, "");
        }

        public UiNode(
                String text,
                String desc,
                String hint,
                String className,
                String viewId,
                UiBounds bounds,
                boolean clickable,
                boolean editable,
                boolean scrollable,
                boolean visible,
                int depth,
                AccessibilityNodeInfo a11yNode,
                String avatarB64) {
            this.text = text == null ? "" : text.trim();
            this.desc = desc == null ? "" : desc.trim();
            this.hint = hint == null ? "" : hint.trim();
            this.className = className == null ? "" : className;
            this.viewId = viewId == null ? "" : viewId;
            this.bounds = bounds != null ? bounds : new UiBounds(0, 0, 0, 0);
            this.clickable = clickable;
            this.editable = editable;
            this.scrollable = scrollable;
            this.visible = visible;
            this.depth = depth;
            this.a11yNode = a11yNode;
            this.avatarB64 = avatarB64 == null ? "" : avatarB64.trim();
        }

        public JSONObject toJson() throws Exception {
            JSONObject j = new JSONObject();
            if (!text.isEmpty()) j.put("t", text);
            if (!desc.isEmpty()) j.put("d", desc);
            if (!hint.isEmpty()) j.put("h", hint);
            if (!className.isEmpty()) j.put("c", className);
            if (!viewId.isEmpty()) j.put("id", viewId);
            j.put("l", bounds.left);
            j.put("tp", bounds.top);
            j.put("r", bounds.right);
            j.put("b", bounds.bottom);
            if (clickable) j.put("ck", true);
            if (editable) j.put("ed", true);
            if (scrollable) j.put("sc", true);
            j.put("dp", depth);
            if (avatarB64 != null && !avatarB64.isEmpty()) {
                j.put("av", avatarB64);
            }
            if (mediaB64 != null && !mediaB64.isEmpty()) {
                j.put("mb", mediaB64);
            }
            if (!children.isEmpty()) {
                JSONArray arr = new JSONArray();
                for (UiNode child : children) {
                    arr.put(child.toJson());
                }
                j.put("ch", arr);
            }
            return j;
        }

        public static UiNode fromJson(JSONObject j) {
            if (j == null) return null;
            UiBounds r = new UiBounds(
                j.optInt("l", 0),
                j.optInt("tp", 0),
                j.optInt("r", 0),
                j.optInt("b", 0)
            );
            UiNode node = new UiNode(
                j.optString("t", ""),
                j.optString("d", ""),
                j.optString("h", ""),
                j.optString("c", ""),
                j.optString("id", ""),
                r,
                j.optBoolean("ck", false),
                j.optBoolean("ed", false),
                j.optBoolean("sc", false),
                true,
                j.optInt("dp", 0),
                null
            );
            node.avatarB64 = j.optString("av", j.optString("avatarB64", ""));
            node.mediaB64 = j.optString("mb", j.optString("mediaB64", ""));
            JSONArray arr = j.optJSONArray("ch");
            if (arr != null) {
                for (int i = 0; i < arr.length(); i++) {
                    UiNode c = fromJson(arr.optJSONObject(i));
                    if (c != null) node.children.add(c);
                }
            }
            return node;
        }
    }

    public static final class UiWindowSnapshot {
        public final String packageName;
        public final String windowTitle;
        public final int screenWidth;
        public final int screenHeight;
        public final UiNode root;
        public final String sourceTier;
        public final boolean actionResult;

        public UiWindowSnapshot(
                String packageName,
                String windowTitle,
                int screenWidth,
                int screenHeight,
                UiNode root,
                String sourceTier,
                boolean actionResult) {
            this.packageName = packageName == null ? "" : packageName;
            this.windowTitle = windowTitle == null ? "" : windowTitle;
            this.screenWidth = screenWidth > 0 ? screenWidth : 1080;
            this.screenHeight = screenHeight > 0 ? screenHeight : 2400;
            this.root = root;
            this.sourceTier = sourceTier == null ? "" : sourceTier;
            this.actionResult = actionResult;
        }
    }

    public static final class ParsedChatWindow {
        public final String rawTitle;
        public final String cleanTitle;
        public final boolean isGroup;
        public final boolean isConversationList;
        public final List<PushStore.CapturedBubble> bubbles;
        public final List<PushStore.CapturedConversationRow> conversationRows;
        public final String avatarB64;
        public final Map<String, String> senderAvatars;

        public ParsedChatWindow(
                String rawTitle,
                String cleanTitle,
                boolean isGroup,
                boolean isConversationList,
                List<PushStore.CapturedBubble> bubbles,
                List<PushStore.CapturedConversationRow> conversationRows) {
            this(rawTitle, cleanTitle, isGroup, isConversationList, bubbles, conversationRows, "", null);
        }

        public ParsedChatWindow(
                String rawTitle,
                String cleanTitle,
                boolean isGroup,
                boolean isConversationList,
                List<PushStore.CapturedBubble> bubbles,
                List<PushStore.CapturedConversationRow> conversationRows,
                String avatarB64,
                Map<String, String> senderAvatars) {
            this.rawTitle = rawTitle == null ? "" : rawTitle;
            this.cleanTitle = cleanTitle == null ? "" : cleanTitle;
            this.isGroup = isGroup;
            this.isConversationList = isConversationList;
            this.bubbles = bubbles != null ? bubbles : new ArrayList<PushStore.CapturedBubble>();
            this.conversationRows = conversationRows != null
                ? conversationRows
                : new ArrayList<PushStore.CapturedConversationRow>();
            this.avatarB64 = avatarB64 == null ? "" : avatarB64;
            this.senderAvatars = senderAvatars != null ? senderAvatars : new HashMap<String, String>();
        }
    }

    /**
     * In-process entry point invoked on the target app's UI thread by StepOSViewCaptureManager.apexRequest.
     */
    public static void handleApexRequest(View rootView, Intent intent) {
        if (rootView == null || intent == null) return;
        Bundle extras = intent.getExtras();
        if (extras == null) return;
        IBinder callback = extras.getBinder("targetDexBinder");
        if (callback == null) return;

        String op = extras.getString("stepxOp", "snapshot");
        int x = extras.getInt("stepxX", 0);
        int y = extras.getInt("stepxY", 0);
        String text = extras.getString("stepxText", "");
        boolean actionOk = true;

        try {
            if ("click".equals(op)) {
                actionOk = performInProcessClick(rootView, x, y);
            } else if ("setText".equals(op)) {
                actionOk = performInProcessSetText(rootView, x, y, text);
            } else if ("scrollDown".equals(op)) {
                actionOk = performInProcessScrollDown(rootView, x, y);
            }

            Context ctx = rootView.getContext();
            String pkg = ctx != null ? ctx.getPackageName() : extras.getString("targetFindPackage", "");
            int sw = rootView.getWidth() > 0 ? rootView.getWidth() : 1080;
            int sh = rootView.getHeight() > 0 ? rootView.getHeight() : 2400;
            UiNode rootNode = buildUiNodeFromLiveView(rootView, 0);

            JSONObject payload = new JSONObject();
            payload.put("ok", true);
            payload.put("actionOk", actionOk);
            payload.put("pkg", pkg);
            payload.put("sw", sw);
            payload.put("sh", sh);
            if (rootNode != null) {
                payload.put("root", rootNode.toJson());
            }

            Parcel data = Parcel.obtain();
            Parcel reply = Parcel.obtain();
            try {
                data.writeInterfaceToken(APEX_CALLBACK_DESCRIPTOR);
                data.writeString(payload.toString());
                callback.transact(1, data, reply, 0);
            } finally {
                data.recycle();
                reply.recycle();
            }
        } catch (Throwable ignored) {
        }
    }

    private static boolean performInProcessClick(View rootView, int x, int y) {
        View target = findDeepestClickableViewAt(rootView, x, y);
        boolean clicked = false;
        if (target != null) {
            try {
                clicked = target.performClick();
            } catch (Throwable ignored) {
            }
        }
        if (!clicked) {
            clicked = performAdapterViewItemClickAt(rootView, x, y);
        }
        int[] rootLoc = new int[2];
        try {
            rootView.getLocationOnScreen(rootLoc);
        } catch (Throwable ignored) {
        }
        float localX = x - rootLoc[0];
        float localY = y - rootLoc[1];
        long now = SystemClock.uptimeMillis();
        MotionEvent down = MotionEvent.obtain(now, now, MotionEvent.ACTION_DOWN, localX, localY, 0);
        MotionEvent up = MotionEvent.obtain(now, now + 45L, MotionEvent.ACTION_UP, localX, localY, 0);
        try {
            boolean dispatched = rootView.dispatchTouchEvent(down);
            dispatched |= rootView.dispatchTouchEvent(up);
            return clicked || dispatched;
        } catch (Throwable ignored) {
            return clicked;
        } finally {
            down.recycle();
            up.recycle();
        }
    }

    private static boolean performAdapterViewItemClickAt(View view, int x, int y) {
        if (view == null || view.getVisibility() != View.VISIBLE) return false;
        int[] loc = new int[2];
        view.getLocationOnScreen(loc);
        Rect r = new Rect(loc[0], loc[1], loc[0] + view.getWidth(), loc[1] + view.getHeight());
        if (!r.contains(x, y)) return false;

        if (view instanceof android.widget.AdapterView) {
            android.widget.AdapterView<?> av = (android.widget.AdapterView<?>) view;
            for (int i = 0; i < av.getChildCount(); i++) {
                View child = av.getChildAt(i);
                if (child == null || child.getVisibility() != View.VISIBLE) continue;
                int[] cLoc = new int[2];
                child.getLocationOnScreen(cLoc);
                Rect cr = new Rect(cLoc[0], cLoc[1], cLoc[0] + child.getWidth(), cLoc[1] + child.getHeight());
                if (cr.contains(x, y)) {
                    try {
                        int pos = av.getPositionForView(child);
                        long id = av.getAdapter() != null && pos >= 0 ? av.getAdapter().getItemId(pos) : 0L;
                        if (pos >= 0 && av.performItemClick(child, pos, id)) {
                            return true;
                        }
                    } catch (Throwable ignored) {
                    }
                }
            }
        }
        if (view instanceof ViewGroup) {
            ViewGroup vg = (ViewGroup) view;
            for (int i = vg.getChildCount() - 1; i >= 0; i--) {
                if (performAdapterViewItemClickAt(vg.getChildAt(i), x, y)) {
                    return true;
                }
            }
        }
        return false;
    }

    private static boolean performInProcessSetText(View rootView, int x, int y, String text) {
        View target = findEditableViewAtOrFirst(rootView, x, y);
        if (target == null) return false;
        try {
            target.requestFocus();
        } catch (Throwable ignored) {
        }
        if (target instanceof EditText) {
            EditText et = (EditText) target;
            et.setText(text);
            if (text != null) {
                try {
                    et.setSelection(text.length());
                } catch (Throwable ignored) {
                }
            }
            return true;
        }
        try {
            Method setTextMethod = target.getClass().getMethod("setText", CharSequence.class);
            setTextMethod.invoke(target, text);
            return true;
        } catch (Throwable ignored) {
        }
        try {
            Bundle args = new Bundle();
            args.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text);
            return target.performAccessibilityAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args);
        } catch (Throwable ignored) {
            return false;
        }
    }

    private static View findDeepestClickableViewAt(View view, int x, int y) {
        if (view == null || view.getVisibility() != View.VISIBLE) return null;
        int[] loc = new int[2];
        view.getLocationOnScreen(loc);
        Rect r = new Rect(loc[0], loc[1], loc[0] + view.getWidth(), loc[1] + view.getHeight());
        if (!r.contains(x, y)) return null;

        if (view instanceof ViewGroup) {
            ViewGroup vg = (ViewGroup) view;
            for (int i = vg.getChildCount() - 1; i >= 0; i--) {
                View found = findDeepestClickableViewAt(vg.getChildAt(i), x, y);
                if (found != null) return found;
            }
        }
        if (view.isClickable() || view.isLongClickable()) {
            return view;
        }
        return null;
    }

    private static View findEditableViewAtOrFirst(View view, int x, int y) {
        View atPoint = findEditableViewRecursive(view, x, y, true);
        if (atPoint != null) return atPoint;
        return findEditableViewRecursive(view, x, y, false);
    }

    private static View findEditableViewRecursive(View view, int x, int y, boolean requireContains) {
        if (view == null || view.getVisibility() != View.VISIBLE) return null;
        int[] loc = new int[2];
        view.getLocationOnScreen(loc);
        Rect r = new Rect(loc[0], loc[1], loc[0] + view.getWidth(), loc[1] + view.getHeight());
        boolean isEdit = (view instanceof EditText)
            || view.getClass().getName().contains("EditText")
            || view.onCheckIsTextEditor();
        if (isEdit && (!requireContains || r.contains(x, y))) {
            return view;
        }
        if (view instanceof ViewGroup) {
            ViewGroup vg = (ViewGroup) view;
            for (int i = 0; i < vg.getChildCount(); i++) {
                View found = findEditableViewRecursive(vg.getChildAt(i), x, y, requireContains);
                if (found != null) return found;
            }
        }
        return null;
    }

    private static UiNode buildUiNodeFromLiveView(View view, int depth) {
        if (view == null || depth > 45 || view.getVisibility() != View.VISIBLE) return null;
        int w = view.getWidth();
        int h = view.getHeight();
        if (w <= 0 || h <= 0) return null;

        int[] loc = new int[2];
        view.getLocationOnScreen(loc);
        UiBounds bounds = new UiBounds(loc[0], loc[1], loc[0] + w, loc[1] + h);

        String className = view.getClass().getName();
        String viewId = "";
        try {
            if (view.getId() != View.NO_ID && view.getResources() != null) {
                viewId = view.getResources().getResourceEntryName(view.getId());
            }
        } catch (Throwable ignored) {
        }

        String text = "";
        String hint = "";
        String desc = view.getContentDescription() != null ? view.getContentDescription().toString().trim() : "";
        boolean editable = (view instanceof EditText) || className.contains("EditText");
        boolean clickable = view.isClickable();
        boolean scrollable = view.canScrollVertically(1) || view.canScrollVertically(-1);

        if (view instanceof TextView) {
            TextView tv = (TextView) view;
            if (tv.getText() != null) text = tv.getText().toString().trim();
            if (tv.getHint() != null) hint = tv.getHint().toString().trim();
        }
        if (text.isEmpty() || desc.isEmpty()) {
            try {
                AccessibilityNodeInfo info = view.createAccessibilityNodeInfo();
                if (info != null) {
                    if (text.isEmpty() && info.getText() != null) {
                        text = info.getText().toString().trim();
                    }
                    if (desc.isEmpty() && info.getContentDescription() != null) {
                        desc = info.getContentDescription().toString().trim();
                    }
                    editable |= info.isEditable();
                    clickable |= info.isClickable();
                    scrollable |= info.isScrollable();
                    info.recycle();
                }
            } catch (Throwable ignored) {
            }
        }
        if (text.isEmpty()
                && (!(view instanceof ViewGroup) || ((ViewGroup) view).getChildCount() == 0)
                && !(view instanceof android.widget.ImageView)) {
            text = extractCustomViewText(view, className);
        }

        UiNode node = new UiNode(
            text, desc, hint, className, viewId, bounds,
            clickable, editable, scrollable, true, depth, null, ""
        );
        if (view instanceof ViewGroup) {
            ViewGroup vg = (ViewGroup) view;
            for (int i = 0; i < vg.getChildCount(); i++) {
                UiNode child = buildUiNodeFromLiveView(vg.getChildAt(i), depth + 1);
                if (child != null) {
                    node.children.add(child);
                }
            }
        }
        // Capture real user/group avatar bitmap directly from live IM view if it matches avatar geometry
        android.util.DisplayMetrics dm = view.getResources() != null ? view.getResources().getDisplayMetrics() : null;
        float density = (dm != null && dm.density > 0f) ? dm.density : 2.75f;
        int sw = (dm != null && dm.widthPixels > 0) ? dm.widthPixels : 1080;
        int sh = (dm != null && dm.heightPixels > 0) ? dm.heightPixels : 2400;
        int minAvatarPx = Math.max(36, Math.round(28f * density));
        int maxAvatarPx = Math.max(minAvatarPx, Math.round(sw * 0.16f));
        int centerX = loc[0] + w / 2;
        boolean inLeftGutter = loc[0] <= sw * 0.18f && centerX <= sw * 0.22f;
        boolean inRightGutter = (loc[0] + w) >= sw * 0.82f && centerX >= sw * 0.78f;
        if (loc[1] >= 120 && w >= minAvatarPx && w <= maxAvatarPx && h >= minAvatarPx && h <= maxAvatarPx && Math.abs(w - h) <= 24) {
            boolean isAvatarPosition = inLeftGutter || inRightGutter;
            boolean isAvatarView = className.contains("Image")
                || className.contains("Avatar")
                || !ImAppCatalog.extractAvatarSenderName(desc).isEmpty()
                || (text.isEmpty() && (view.getBackground() != null || !(view instanceof ViewGroup) || ((ViewGroup) view).getChildCount() <= 2));
            if (isAvatarPosition && isAvatarView) {
                boolean childAlreadyHasAvatar = false;
                for (UiNode c : node.children) {
                    if (!c.avatarB64.isEmpty()) {
                        childAlreadyHasAvatar = true;
                        break;
                    }
                }
                if (!childAlreadyHasAvatar) {
                    node.avatarB64 = captureLiveAvatarBase64(view);
                }
            }
        }
        if (node.avatarB64.isEmpty()
                && node.mediaB64.isEmpty()
                && loc[1] >= 120
                && !inLeftGutter
                && !inRightGutter
                && w >= 56
                && h >= 56
                && w <= sw * 0.75f
                && h <= sh * 0.55f
                && text.isEmpty()) {
            if ((view instanceof android.widget.ImageView)
                    || className.contains("Image")
                    || className.contains("Emoji")
                    || className.contains("Animate")
                    || className.contains("Sticker")
                    || !(view instanceof ViewGroup)) {
                boolean childAlreadyHasMedia = false;
                for (UiNode c : node.children) {
                    if (!c.mediaB64.isEmpty()) {
                        childAlreadyHasMedia = true;
                        break;
                    }
                }
                if (!childAlreadyHasMedia) {
                    node.mediaB64 = captureLiveMediaViewBase64(view);
                }
            }
        }
        return node;
    }

    private static String captureLiveMediaViewBase64(View view) {
        if (view == null) return "";
        try {
            int w = view.getWidth();
            int h = view.getHeight();
            if (w < 48 || h < 48 || w > 1200 || h > 1600) return "";
            float scale = Math.min(1.0f, 200.0f / (float) Math.max(w, h));
            int targetW = Math.max(36, Math.round(w * scale));
            int targetH = Math.max(36, Math.round(h * scale));
            Bitmap bmp = Bitmap.createBitmap(targetW, targetH, Bitmap.Config.ARGB_8888);
            Canvas canvas = new Canvas(bmp);
            boolean drawn = false;
            if (view instanceof android.widget.ImageView) {
                try {
                    android.graphics.drawable.Drawable d = ((android.widget.ImageView) view).getDrawable();
                    if (d instanceof android.graphics.drawable.BitmapDrawable) {
                        Bitmap srcBmp = ((android.graphics.drawable.BitmapDrawable) d).getBitmap();
                        if (srcBmp != null && !srcBmp.isRecycled() && srcBmp.getWidth() >= 32 && srcBmp.getHeight() >= 32) {
                            Bitmap swBmp = (srcBmp.getConfig() == Bitmap.Config.HARDWARE)
                                ? srcBmp.copy(Bitmap.Config.ARGB_8888, false)
                                : srcBmp;
                            if (swBmp != null) {
                                canvas.drawBitmap(
                                    swBmp,
                                    new Rect(0, 0, swBmp.getWidth(), swBmp.getHeight()),
                                    new RectF(0, 0, targetW, targetH),
                                    new android.graphics.Paint(
                                        android.graphics.Paint.ANTI_ALIAS_FLAG | android.graphics.Paint.FILTER_BITMAP_FLAG
                                    )
                                );
                                if (swBmp != srcBmp) swBmp.recycle();
                                drawn = true;
                            }
                        }
                    }
                } catch (Throwable ignored) {
                }
            }
            if (!drawn) {
                try {
                    canvas.save();
                    canvas.scale((float) targetW / (float) w, (float) targetH / (float) h);
                    view.draw(canvas);
                    canvas.restore();
                } catch (Throwable ignored) {
                }
            }

            int[] sampleCoords = new int[]{
                targetW / 4, targetH / 4,      targetW / 2, targetH / 4,      (targetW * 3) / 4, targetH / 4,
                targetW / 4, targetH / 2,      targetW / 2, targetH / 2,      (targetW * 3) / 4, targetH / 2,
                targetW / 4, (targetH * 3) / 4, targetW / 2, (targetH * 3) / 4, (targetW * 3) / 4, (targetH * 3) / 4
            };
            int nonTransparent = 0;
            int firstOpaqueRgb = 0;
            boolean hasColorVariance = false;
            for (int i = 0; i < sampleCoords.length; i += 2) {
                int sx = Math.max(0, Math.min(targetW - 1, sampleCoords[i]));
                int sy = Math.max(0, Math.min(targetH - 1, sampleCoords[i + 1]));
                int px = bmp.getPixel(sx, sy);
                int alpha = (px >>> 24) & 0xFF;
                if (alpha > 20) {
                    int rgb = px & 0x00FFFFFF;
                    if (nonTransparent == 0) {
                        firstOpaqueRgb = rgb;
                    } else if (rgb != firstOpaqueRgb) {
                        hasColorVariance = true;
                    }
                    nonTransparent++;
                }
            }
            if (nonTransparent < 2 || !hasColorVariance) {
                bmp.recycle();
                return "";
            }
            boolean transparentCorners =
                ((bmp.getPixel(2, 2) >>> 24) & 0xFF) < 200
                || ((bmp.getPixel(targetW - 3, 2) >>> 24) & 0xFF) < 200
                || ((bmp.getPixel(2, targetH - 3) >>> 24) & 0xFF) < 200
                || ((bmp.getPixel(targetW - 3, targetH - 3) >>> 24) & 0xFF) < 200;
            ByteArrayOutputStream baos = new ByteArrayOutputStream(4096);
            String mimePrefix;
            if (transparentCorners) {
                bmp.compress(Bitmap.CompressFormat.PNG, 90, baos);
                mimePrefix = "data:image/png;base64,";
            } else {
                bmp.compress(Bitmap.CompressFormat.JPEG, 80, baos);
                mimePrefix = "data:image/jpeg;base64,";
            }
            bmp.recycle();
            byte[] bytes = baos.toByteArray();
            if (bytes.length < 100) return "";
            return mimePrefix + Base64.encodeToString(bytes, Base64.NO_WRAP);
        } catch (Throwable ignored) {
            return "";
        }
    }

    private static String captureLiveAvatarBase64(View view) {
        if (view == null) return "";
        try {
            int w = view.getWidth();
            int h = view.getHeight();
            if (w < 36 || h < 36 || w > 300 || h > 300) return "";
            int targetSize = 64;
            Bitmap bmp = Bitmap.createBitmap(targetSize, targetSize, Bitmap.Config.ARGB_8888);
            Canvas canvas = new Canvas(bmp);
            android.graphics.Path clipPath = new android.graphics.Path();
            clipPath.addRoundRect(
                new RectF(0, 0, targetSize, targetSize),
                12f,
                12f,
                android.graphics.Path.Direction.CW
            );
            canvas.clipPath(clipPath);
            canvas.scale((float) targetSize / (float) w, (float) targetSize / (float) h);
            view.draw(canvas);

            int[] sampleCoords = new int[]{
                16, 16,  32, 16,  48, 16,
                16, 32,  32, 32,  48, 32,
                16, 48,  32, 48,  48, 48
            };
            int nonTransparent = 0;
            int firstOpaqueRgb = 0;
            boolean hasColorVariance = false;
            for (int i = 0; i < sampleCoords.length; i += 2) {
                int px = bmp.getPixel(sampleCoords[i], sampleCoords[i + 1]);
                int alpha = (px >>> 24) & 0xFF;
                if (alpha > 24) {
                    int rgb = px & 0x00FFFFFF;
                    if (nonTransparent == 0) {
                        firstOpaqueRgb = rgb;
                    } else if (rgb != firstOpaqueRgb) {
                        hasColorVariance = true;
                    }
                    nonTransparent++;
                }
            }
            if (nonTransparent < 3 || !hasColorVariance) {
                bmp.recycle();
                return "";
            }
            ByteArrayOutputStream baos = new ByteArrayOutputStream(2048);
            bmp.compress(Bitmap.CompressFormat.PNG, 100, baos);
            bmp.recycle();
            byte[] bytes = baos.toByteArray();
            if (bytes.length < 80) return "";
            return Base64.encodeToString(bytes, Base64.NO_WRAP);
        } catch (Throwable ignored) {
            return "";
        }
    }

    private static boolean performInProcessScrollDown(View view, int x, int y) {
        if (view == null || view.getVisibility() != View.VISIBLE) return false;
        if (view.canScrollVertically(1)) {
            try {
                if (view.performAccessibilityAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD, null)) {
                    return true;
                }
            } catch (Throwable ignored) {
            }
            try {
                view.scrollBy(0, Math.max(200, view.getHeight() / 2));
                return true;
            } catch (Throwable ignored) {
            }
        }
        if (view instanceof ViewGroup) {
            ViewGroup vg = (ViewGroup) view;
            for (int i = 0; i < vg.getChildCount(); i++) {
                if (performInProcessScrollDown(vg.getChildAt(i), x, y)) {
                    return true;
                }
            }
        }
        return false;
    }

    private static String extractCustomViewText(View view, String className) {
        if (view == null) return "";
        String lowerCls = className == null ? "" : className.toLowerCase(Locale.ROOT);
        boolean isLeafView = !(view instanceof ViewGroup) || ((ViewGroup) view).getChildCount() == 0;
        if (!isLeafView
                && !lowerCls.contains("text")
                && !lowerCls.contains("neat")
                && !lowerCls.contains("edit")
                && !lowerCls.contains("cell")
                && !lowerCls.contains("msg")) {
            return "";
        }
        String[] candidateMethods = new String[]{
            "getText", "getContentText", "getTitle", "getSummary", "getCharSequence", "a"
        };
        for (String methodName : candidateMethods) {
            try {
                Method m = view.getClass().getMethod(methodName);
                if (m.getParameterTypes().length == 0
                        && (CharSequence.class.isAssignableFrom(m.getReturnType())
                            || String.class.equals(m.getReturnType()))) {
                    Object val = m.invoke(view);
                    if (val != null) {
                        String s = val.toString().trim();
                        if (!s.isEmpty() && !isBogusInternalViewConstant(s)) return s;
                    }
                }
            } catch (Throwable ignored) {
            }
        }
        try {
            java.lang.reflect.Field[] fields = view.getClass().getDeclaredFields();
            for (int i = 0; i < fields.length && i < 20; i++) {
                java.lang.reflect.Field f = fields[i];
                if (java.lang.reflect.Modifier.isStatic(f.getModifiers())) continue;
                Class<?> ft = f.getType();
                if (CharSequence.class.equals(ft)
                        || String.class.equals(ft)
                        || android.text.Spannable.class.isAssignableFrom(ft)) {
                    f.setAccessible(true);
                    Object val = f.get(view);
                    if (val != null) {
                        String s = val.toString().trim();
                        if (!s.isEmpty() && !isBogusInternalViewConstant(s)) return s;
                    }
                }
            }
        } catch (Throwable ignored) {
        }
        return "";
    }

    private static boolean isBogusInternalViewConstant(String s) {
        if (s == null || s.isEmpty()) return true;
        if ("ImageView".equals(s)
                || "TextView".equals(s)
                || "View".equals(s)
                || "Button".equals(s)
                || "LinearLayout".equals(s)
                || "RelativeLayout".equals(s)
                || "FrameLayout".equals(s)
                || "credential".equals(s)
                || s.startsWith("[\\u0001-")) {
            return true;
        }
        if (s.startsWith("com.") || s.startsWith("android.") || s.startsWith("androidx.")) {
            return true;
        }
        if (s.matches("^[a-zA-Z0-9_\\-./:]{20,}$") && !s.contains(" ")) {
            return true;
        }
        return false;
    }

    /**
     * Captures visible chat messages from the active IM window and merges them into PushStore.
     */
    public static JSONObject captureActiveChatWindow(Context context) throws Exception {
        return captureActiveChatWindow(context, "", "");
    }

    /**
     * Captures visible chat messages (or conversation list rows) for the given package/title,
     * working even when the phone is locked via the StepOS apexRequest bridge.
     */
    public static JSONObject captureActiveChatWindow(
            Context context,
            String preferredPkg,
            String preferredTitle) throws Exception {
        String targetPkg = preferredPkg == null ? "" : preferredPkg.trim();
        String targetTitle = preferredTitle == null ? "" : preferredTitle.trim();
        String effectivePkg = targetPkg.isEmpty() ? "com.tencent.mm" : targetPkg;

        if (PushNotificationService.instance != null) {
            try {
                PushNotificationService.instance.scanActiveNotifications();
            } catch (Throwable ignored) {
            }
        }

        // Never acquire SCREEN_DIM_WAKE_LOCK | ACQUIRE_CAUSES_WAKEUP during passive IM capture/sync,
        // as waking the display when locked triggers Android Face Unlock and display/camera thermal drain.
        UiWindowSnapshot snap = captureBestWindowSnapshot(context, effectivePkg);
        if (snap == null || snap.root == null) {
            PushStore store = PushStore.get(context);
            PushStore.ThreadRecord fallbackThread = null;
            if (!targetTitle.isEmpty()) {
                fallbackThread = store.getThreadRecord(
                    targetTitle.contains("::") ? targetTitle : ImAppCatalog.buildThreadId(effectivePkg, targetTitle)
                );
            }
            if (fallbackThread == null) {
                fallbackThread = store.getThreadRecord(ImAppCatalog.buildThreadId(effectivePkg, "群聊"));
            }
            if (fallbackThread != null) {
                return new JSONObject()
                    .put("ok", true)
                    .put("packageName", fallbackThread.packageName)
                    .put("appName", fallbackThread.appName)
                    .put("title", fallbackThread.title)
                    .put("threadId", fallbackThread.threadId)
                    .put("capturedCount", 0)
                    .put("upgradedSendersCount", 0)
                    .put("thread", fallbackThread.toJson());
            }
            throw new IllegalStateException("未检测到活动的聊天窗口，请确认目标应用已在运行");
        }

        String pkg = snap.packageName;
        String appName = ImAppCatalog.appDisplayName(context, pkg);
        ParsedChatWindow parsed = parseWindowSnapshot(snap);

        String lookupTitle = targetTitle.isEmpty() ? "群聊" : targetTitle;
        List<String> knownMessages = PushStore.get(context).getRecentMessageTextsForThread(
            pkg, lookupTitle
        );
        List<String> participantHints = PushStore.get(context).getParticipantHintsForThread(
            pkg, lookupTitle
        );

        // If currently inside a chat window that does NOT match targetTitle, save its messages
        // to its own thread (without targetTitle alias!) and navigate back to LauncherUI first.
        if (!parsed.isConversationList && !targetTitle.isEmpty()) {
            boolean alreadyMatching = isMatchingChatWindow(
                snap, parsed, pkg, targetTitle, knownMessages, participantHints
            );
            if (!alreadyMatching) {
                if (!parsed.rawTitle.isEmpty() && !parsed.bubbles.isEmpty()) {
                    PushStore.get(context).mergeCapturedWindowMessages(
                        pkg,
                        appName,
                        parsed.rawTitle,
                        parsed.bubbles,
                        parsed.isGroup,
                        "",
                        parsed.avatarB64,
                        parsed.senderAvatars
                    );
                }
                for (int backStep = 0; backStep < 3 && !parsed.isConversationList; backStep++) {
                    performBackNavigation(context, snap);
                    Thread.sleep(420L);
                    UiWindowSnapshot afterBack = captureBestWindowSnapshot(context, pkg);
                    if (afterBack == null || afterBack.root == null) break;
                    snap = afterBack;
                    parsed = parseWindowSnapshot(snap);
                }
            }
        }

        boolean clickedTargetFromList = false;
        int listUpgradedCount = 0;
        // If the app is currently on its main Conversation List (LauncherUI), merge visible conversation rows first!
        if (parsed.isConversationList && !parsed.conversationRows.isEmpty()) {
            listUpgradedCount = PushStore.get(context).mergeCapturedConversationList(pkg, appName, parsed.conversationRows);

            if (!targetTitle.isEmpty()) {
                // Refresh hints after merging conversation list
                knownMessages = PushStore.get(context).getRecentMessageTextsForThread(
                    pkg, lookupTitle
                );
                participantHints = PushStore.get(context).getParticipantHintsForThread(
                    pkg, lookupTitle
                );
                UiNode rowToOpen = findConversationRowOnList(
                    snap.root,
                    pkg,
                    lookupTitle,
                    knownMessages,
                    participantHints,
                    snap.screenHeight
                );
                if (rowToOpen != null) {
                    performNodeClick(context, snap, rowToOpen);
                    Thread.sleep(550L);
                    UiWindowSnapshot entered = captureBestWindowSnapshot(context, pkg);
                    if (entered != null && entered.root != null) {
                        ParsedChatWindow enteredParsed = parseWindowSnapshot(entered);
                        if (!enteredParsed.isConversationList) {
                            snap = entered;
                            parsed = enteredParsed;
                            clickedTargetFromList = true;
                        }
                    }
                }
            }
        }

        if (parsed.isConversationList) {
            PushStore store = PushStore.get(context);
            JSONObject listRes = new JSONObject()
                .put("ok", true)
                .put("packageName", pkg)
                .put("appName", appName)
                .put("isConversationList", true)
                .put("capturedCount", parsed.conversationRows.size())
                .put("upgradedSendersCount", listUpgradedCount)
                .put("revision", store.getRevision());
            if (!targetTitle.isEmpty()) {
                PushStore.ThreadRecord existing = store.getThreadRecord(
                    targetTitle.contains("::") ? targetTitle : ImAppCatalog.buildThreadId(pkg, targetTitle)
                );
                if (existing != null) {
                    listRes.put("title", existing.title);
                    listRes.put("threadId", existing.threadId);
                    listRes.put("thread", existing.toJson());
                }
            }
            return listRes;
        }

        boolean matchedTarget = targetTitle.isEmpty()
            || clickedTargetFromList
            || isMatchingChatWindow(snap, parsed, pkg, targetTitle, knownMessages, participantHints);

        String rawTitle = !parsed.rawTitle.isEmpty()
            ? parsed.rawTitle
            : (matchedTarget && !targetTitle.isEmpty() ? targetTitle : "");
        if (rawTitle.isEmpty() || rawTitle.equals(appName) || parsed.bubbles == null || parsed.bubbles.isEmpty()) {
            return new JSONObject().put("ok", false).put("reason", "non_chat_window");
        }
        String aliasHint = matchedTarget
            ? (!targetTitle.isEmpty() ? targetTitle : "群聊")
            : "";

        return PushStore.get(context).mergeCapturedWindowMessages(
            pkg,
            appName,
            rawTitle,
            parsed.bubbles,
            parsed.isGroup,
            aliasHint,
            parsed.avatarB64,
            parsed.senderAvatars
        );
    }

    /**
     * Sends a chat message via the multi-tier UI bridge (supporting locked screen via StepOS apexRequest
     * as well as generic "群聊" threads via message-history overlap matching).
     */
    public static synchronized JSONObject sendChatViaUi(
            Context context,
            String targetPkg,
            String targetTitle,
            String replyText,
            boolean force,
            String clientMsgId) throws Exception {
        if (replyText == null || replyText.trim().isEmpty()) {
            throw new IllegalArgumentException("回复内容不能为空");
        }
        String trimmedText = replyText.trim();
        String safePkg = (targetPkg == null || targetPkg.trim().isEmpty())
            ? "com.tencent.mm"
            : targetPkg.trim();
        String safeTargetTitle = targetTitle == null ? "" : targetTitle.trim();

        // Always check cached RemoteInput PendingIntent first unless ui_force was explicitly requested!
        if (!force && context != null && !safeTargetTitle.isEmpty()) {
            try {
                PushStore store = PushStore.get(context);
                String candidateThreadId = safeTargetTitle.contains("::")
                    ? safeTargetTitle
                    : ImAppCatalog.buildThreadId(safePkg, safeTargetTitle);
                if (!store.hasReplyHandle(candidateThreadId) && PushNotificationService.instance != null) {
                    try {
                        PushNotificationService.instance.scanActiveNotifications();
                    } catch (Throwable ignored) {
                    }
                }
                if (store.hasReplyHandle(candidateThreadId)) {
                    return store.executeRemoteReply(context, candidateThreadId, trimmedText, clientMsgId);
                }
            } catch (Throwable ignored) {
            }
        }

        // Respect human touch priority unless forced
        if (!force && GuiAccessibilityService.isHumanTouchingRecently(10_000L)) {
            throw new IllegalStateException("检测到手机屏幕正在被手动操作，已暂停自动填入以免干扰操作");
        }

        if (!isApexBridgeAvailable() && GuiAccessibilityService.instance == null) {
            throw new IllegalStateException("无障碍服务与系统视图桥接均未就绪，无法执行界面发送");
        }

        // Briefly wake display if screen is off so target window ViewRootImpl is active for apexRequest
        PowerManager.WakeLock tempWake = null;
        try {
            if (context != null) {
                PowerManager pm = context.getSystemService(PowerManager.class);
                if (pm != null && !pm.isInteractive()) {
                    tempWake = pm.newWakeLock(
                        PowerManager.SCREEN_DIM_WAKE_LOCK | PowerManager.ACQUIRE_CAUSES_WAKEUP,
                        "StepX:im-reply-wake"
                    );
                    tempWake.setReferenceCounted(false);
                    tempWake.acquire(8_000L);
                    Thread.sleep(250L);
                }
            }

            List<String> knownThreadMessages = PushStore.get(context)
                .getRecentMessageTextsForThread(safePkg, safeTargetTitle);
            List<String> participantHints = PushStore.get(context)
                .getParticipantHintsForThread(safePkg, safeTargetTitle);

            UiWindowSnapshot snap = captureBestWindowSnapshot(context, safePkg);
            if (snap == null || snap.root == null || !safePkg.equals(snap.packageName)) {
                launchOrOpenConversation(context, safePkg, safeTargetTitle);
                for (int attempt = 0; attempt < 6; attempt++) {
                    Thread.sleep(450L);
                    snap = captureBestWindowSnapshot(context, safePkg);
                    if (snap != null && snap.root != null && safePkg.equals(snap.packageName)) {
                        break;
                    }
                    if (attempt == 1) {
                        try {
                            if ("com.tencent.mm".equals(safePkg)) {
                                Runtime.getRuntime().exec(new String[]{
                                    "am", "start", "-n", "com.tencent.mm/.ui.LauncherUI"
                                });
                            } else {
                                Runtime.getRuntime().exec(new String[]{
                                    "am", "start", "-p", safePkg
                                });
                            }
                        } catch (Throwable ignored) {
                        }
                    } else if (attempt == 3) {
                        try {
                            Runtime.getRuntime().exec(new String[]{
                                "monkey", "-p", safePkg, "-c", "android.intent.category.LAUNCHER", "1"
                            });
                        } catch (Throwable ignored) {
                        }
                    }
                }
            }

            if (snap == null || snap.root == null) {
                throw new IllegalStateException("无法获取 " + ImAppCatalog.appDisplayName(context, safePkg) + " 窗口视图，请确认应用已在后台或前台打开");
            }

            ParsedChatWindow parsed = parseWindowSnapshot(snap);
            if (parsed.isConversationList && !parsed.conversationRows.isEmpty()) {
                PushStore.get(context).mergeCapturedConversationList(
                    safePkg,
                    ImAppCatalog.appDisplayName(context, safePkg),
                    parsed.conversationRows
                );
                if (!safeTargetTitle.isEmpty()) {
                    PushStore.ThreadRecord upgraded = PushStore.get(context).getThreadRecord(
                        ImAppCatalog.buildThreadId(safePkg, safeTargetTitle)
                    );
                    if (upgraded != null && upgraded.title != null && !upgraded.title.isEmpty()
                            && !ImAppCatalog.isGenericPlaceholderTitle(upgraded.title)) {
                        safeTargetTitle = upgraded.title;
                    }
                }
            }

            // Check if the current window is already the target chat window
            boolean inMatchingChatWindow = !parsed.isConversationList
                && isMatchingChatWindow(snap, parsed, safePkg, safeTargetTitle, knownThreadMessages, participantHints);

            if (!inMatchingChatWindow && !safeTargetTitle.isEmpty()) {
                // 1) Try jumping directly into the conversation via its cached contentIntent first
                try {
                    JSONObject openRes = PushStore.get(context).openPush(
                        context,
                        ImAppCatalog.buildThreadId(safePkg, ImAppCatalog.cleanConversationTitle(safePkg, safeTargetTitle)),
                        safePkg
                    );
                    if (openRes != null && openRes.optBoolean("openedViaContentIntent", false)) {
                        Thread.sleep(450L);
                        UiWindowSnapshot afterOpen = captureBestWindowSnapshot(context, safePkg);
                        if (afterOpen != null && afterOpen.root != null && safePkg.equals(afterOpen.packageName)) {
                            snap = afterOpen;
                            parsed = parseWindowSnapshot(snap);
                            inMatchingChatWindow = !parsed.isConversationList
                                && isMatchingChatWindow(snap, parsed, safePkg, safeTargetTitle, knownThreadMessages, participantHints);
                        }
                    }
                } catch (Throwable ignored) {
                }

                // 2) If still not in the matching chat window, navigate back up to 4 steps until Conversation List
                if (!inMatchingChatWindow && !parsed.isConversationList) {
                    for (int backStep = 0; backStep < 4; backStep++) {
                        if (!performBackNavigation(context, snap)) {
                            break;
                        }
                        Thread.sleep(450L);
                        UiWindowSnapshot afterBack = captureBestWindowSnapshot(context, safePkg);
                        if (afterBack != null && afterBack.root != null) {
                            snap = afterBack;
                            parsed = parseWindowSnapshot(snap);
                            if (parsed.isConversationList) {
                                if (!parsed.conversationRows.isEmpty()) {
                                    PushStore.get(context).mergeCapturedConversationList(
                                        safePkg,
                                        ImAppCatalog.appDisplayName(context, safePkg),
                                        parsed.conversationRows
                                    );
                                    PushStore.ThreadRecord upgraded = PushStore.get(context).getThreadRecord(
                                        ImAppCatalog.buildThreadId(safePkg, safeTargetTitle)
                                    );
                                    if (upgraded != null && upgraded.title != null && !upgraded.title.isEmpty()
                                            && !ImAppCatalog.isGenericPlaceholderTitle(upgraded.title)) {
                                        safeTargetTitle = upgraded.title;
                                    }
                                }
                                break;
                            }
                        }
                    }
                }

                // 3) On the Conversation List, locate the target row by title, recent message preview, or hints
                if (!inMatchingChatWindow && (parsed.isConversationList || findChatInputNode(snap.root, snap.screenHeight) == null)) {
                    UiNode rowNode = findConversationRowOnList(
                        snap.root,
                        safePkg,
                        safeTargetTitle,
                        knownThreadMessages,
                        participantHints,
                        snap.screenHeight
                    );
                    if (rowNode == null) {
                        UiNode firstTab = findFirstBottomChatTabNode(snap.root, snap.screenWidth, snap.screenHeight);
                        if (firstTab != null && performNodeClick(context, snap, firstTab)) {
                            Thread.sleep(420L);
                            UiWindowSnapshot afterTab = captureBestWindowSnapshot(context, safePkg);
                            if (afterTab != null && afterTab.root != null) {
                                snap = afterTab;
                                parsed = parseWindowSnapshot(snap);
                                if (parsed.isConversationList && !parsed.conversationRows.isEmpty()) {
                                    PushStore.get(context).mergeCapturedConversationList(
                                        safePkg,
                                        ImAppCatalog.appDisplayName(context, safePkg),
                                        parsed.conversationRows
                                    );
                                    PushStore.ThreadRecord upgraded = PushStore.get(context).getThreadRecord(
                                        ImAppCatalog.buildThreadId(safePkg, safeTargetTitle)
                                    );
                                    if (upgraded != null && upgraded.title != null && !upgraded.title.isEmpty()
                                            && !ImAppCatalog.isGenericPlaceholderTitle(upgraded.title)) {
                                        safeTargetTitle = upgraded.title;
                                    }
                                }
                                rowNode = findConversationRowOnList(
                                    snap.root,
                                    safePkg,
                                    safeTargetTitle,
                                    knownThreadMessages,
                                    participantHints,
                                    snap.screenHeight
                                );
                            }
                        }
                    }
                    for (int scrollAttempt = 0; rowNode == null && scrollAttempt < 3; scrollAttempt++) {
                        if (!performScrollDown(context, snap)) {
                            break;
                        }
                        Thread.sleep(450L);
                        UiWindowSnapshot scrolled = captureBestWindowSnapshot(context, safePkg);
                        if (scrolled == null || scrolled.root == null) {
                            break;
                        }
                        snap = scrolled;
                        parsed = parseWindowSnapshot(snap);
                        if (parsed.isConversationList && !parsed.conversationRows.isEmpty()) {
                            PushStore.get(context).mergeCapturedConversationList(
                                safePkg,
                                ImAppCatalog.appDisplayName(context, safePkg),
                                parsed.conversationRows
                            );
                        }
                        rowNode = findConversationRowOnList(
                            snap.root,
                            safePkg,
                            safeTargetTitle,
                            knownThreadMessages,
                            participantHints,
                            snap.screenHeight
                        );
                    }
                    if (rowNode != null) {
                        performNodeClick(context, snap, rowNode);
                        Thread.sleep(600L);
                        UiWindowSnapshot afterClick = captureBestWindowSnapshot(context, safePkg);
                        if (afterClick != null && afterClick.root != null) {
                            snap = afterClick;
                            parsed = parseWindowSnapshot(snap);
                            inMatchingChatWindow = !parsed.isConversationList;
                        }
                    } else if (!safeTargetTitle.isEmpty()) {
                        UiWindowSnapshot afterSearch = tryOpenConversationViaSearch(
                            context,
                            snap,
                            safePkg,
                            ImAppCatalog.cleanConversationTitle(safePkg, safeTargetTitle)
                        );
                        if (afterSearch != null && afterSearch.root != null) {
                            snap = afterSearch;
                            parsed = parseWindowSnapshot(snap);
                            inMatchingChatWindow = !parsed.isConversationList;
                        }
                    }
                }
            }

            // If WeChat input bar is in voice mode ("按住 说话"), switch to keyboard mode first
            UiNode inputNode = findChatInputNode(snap.root, snap.screenHeight);
            if (inputNode == null) {
                UiNode switchToKb = findVoiceToKeyboardSwitchNode(snap.root, snap.screenHeight);
                if (switchToKb != null) {
                    performNodeClick(context, snap, switchToKb);
                    Thread.sleep(380L);
                    UiWindowSnapshot afterSwitch = captureBestWindowSnapshot(context, safePkg);
                    if (afterSwitch != null && afterSwitch.root != null) {
                        snap = afterSwitch;
                        parsed = parseWindowSnapshot(snap);
                        inputNode = findChatInputNode(snap.root, snap.screenHeight);
                    }
                }
            }

            if (inputNode == null) {
                throw new IllegalStateException("当前界面未找到聊天输入框，请确保手机已打开目标聊天窗口");
            }

            // Re-verify recipient safety before typing and clicking Send
            if (!safeTargetTitle.isEmpty()
                    && !inMatchingChatWindow
                    && !isMatchingChatWindow(snap, parsed, safePkg, safeTargetTitle, knownThreadMessages, participantHints)) {
                throw new IllegalStateException(
                    "会话校验未通过: 期望 [" + safeTargetTitle + "]，当前窗口 [" + parsed.rawTitle + "]"
                );
            }

            // Fill text into the chat input box
            boolean textSet = performNodeSetText(context, snap, inputNode, trimmedText);
            if (!textSet) {
                performNodeClick(context, snap, inputNode);
                Thread.sleep(220L);
                UiWindowSnapshot refocused = captureBestWindowSnapshot(context, safePkg);
                if (refocused != null && refocused.root != null) {
                    snap = refocused;
                    UiNode retryInput = findChatInputNode(snap.root, snap.screenHeight);
                    if (retryInput != null) {
                        inputNode = retryInput;
                    }
                }
                textSet = performNodeSetText(context, snap, inputNode, trimmedText);
            }
            if (!textSet) {
                throw new IllegalStateException("无法向聊天输入框填入文本");
            }

            Thread.sleep(260L);

            // Re-snapshot to locate the newly visible "发送" / "Send" button
            UiWindowSnapshot preSendSnap = captureBestWindowSnapshot(context, safePkg);
            if (preSendSnap != null && preSendSnap.root != null) {
                snap = preSendSnap;
            }
            UiNode sendBtn = findSendButtonNode(snap.root, snap.screenHeight);
            if (sendBtn == null) {
                throw new IllegalStateException("文本已填入输入框，但未找到【发送】按钮");
            }

            boolean clickedSend = performNodeClick(context, snap, sendBtn);
            if (!clickedSend) {
                throw new IllegalStateException("点击【发送】按钮失败");
            }

            Thread.sleep(320L);

            // Capture post-send window to sync real group title, real sender nicknames, and latest bubbles
            String effectiveRawTitle = !parsed.rawTitle.isEmpty() ? parsed.rawTitle : safeTargetTitle;
            boolean effectiveIsGroup = parsed.isGroup;
            UiWindowSnapshot postSendSnap = captureBestWindowSnapshot(context, safePkg);
            if (postSendSnap != null && postSendSnap.root != null) {
                ParsedChatWindow postParsed = parseWindowSnapshot(postSendSnap);
                if (!postParsed.isConversationList) {
                    if (!postParsed.rawTitle.isEmpty()) {
                        effectiveRawTitle = postParsed.rawTitle;
                    }
                    effectiveIsGroup |= postParsed.isGroup;
                    if (!postParsed.bubbles.isEmpty()) {
                        PushStore.get(context).mergeCapturedWindowMessages(
                            safePkg,
                            ImAppCatalog.appDisplayName(context, safePkg),
                            effectiveRawTitle,
                            postParsed.bubbles,
                            effectiveIsGroup,
                            safeTargetTitle,
                            postParsed.avatarB64,
                            postParsed.senderAvatars
                        );
                    }
                }
            } else if (!parsed.bubbles.isEmpty()) {
                PushStore.get(context).mergeCapturedWindowMessages(
                    safePkg,
                    ImAppCatalog.appDisplayName(context, safePkg),
                    effectiveRawTitle,
                    parsed.bubbles,
                    effectiveIsGroup,
                    safeTargetTitle,
                    parsed.avatarB64,
                    parsed.senderAvatars
                );
            }

            return PushStore.get(context).recordOutgoingUiMessage(
                context,
                safePkg,
                effectiveRawTitle.isEmpty() ? safeTargetTitle : effectiveRawTitle,
                trimmedText,
                clientMsgId
            );
        } finally {
            if (tempWake != null && tempWake.isHeld()) {
                try {
                    tempWake.release();
                } catch (Throwable ignored) {
                }
            }
        }
    }

    private static void launchOrOpenConversation(Context context, String pkg, String targetTitle) {
        if (context == null || pkg == null || pkg.isEmpty()) return;
        boolean triedOpenPush = false;
        try {
            PushStore store = PushStore.get(context);
            String tid = ImAppCatalog.buildThreadId(pkg, ImAppCatalog.cleanConversationTitle(pkg, targetTitle));
            PushStore.ThreadRecord tr = store.getThreadRecord(tid);
            if (tr == null && targetTitle != null && !targetTitle.trim().isEmpty()) {
                tr = store.getThreadRecord(pkg + "::" + targetTitle.trim());
            }
            if (tr == null && ImAppCatalog.isGenericPlaceholderTitle(targetTitle)) {
                tr = store.getThreadRecord(ImAppCatalog.buildThreadId(pkg, "群聊"));
            }
            String openKey = (tr != null && tr.lastNotificationKey != null && !tr.lastNotificationKey.isEmpty())
                ? tr.lastNotificationKey
                : (tr != null && tr.threadId != null && !tr.threadId.isEmpty() ? tr.threadId : tid);
            if (openKey != null && !openKey.isEmpty()) {
                JSONObject res = store.openPush(context, openKey, pkg);
                triedOpenPush = res != null && res.optBoolean("openedViaContentIntent", false);
            }
        } catch (Throwable ignored) {
        }
        if (triedOpenPush) {
            try {
                Thread.sleep(600L);
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
            }
            UiWindowSnapshot snap = captureBestWindowSnapshot(context, pkg);
            if (snap != null && snap.root != null && pkg.equals(snap.packageName)) {
                return;
            }
        }
        try {
            Intent launch = context.getPackageManager().getLaunchIntentForPackage(pkg);
            if (launch != null) {
                launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_RESET_TASK_IF_NEEDED);
                android.os.Bundle opts = PushStore.buildBackgroundActivityLaunchOptions();
                if (opts != null) {
                    context.startActivity(launch, opts);
                } else {
                    context.startActivity(launch);
                }
            }
        } catch (Throwable ignored) {
        }
        try {
            if ("com.tencent.mm".equals(pkg)) {
                Runtime.getRuntime().exec(new String[]{"am", "start", "-n", "com.tencent.mm/.ui.LauncherUI"});
            } else {
                Runtime.getRuntime().exec(new String[]{"am", "start", "-p", pkg});
            }
        } catch (Throwable ignored) {
        }
    }

    private static UiWindowSnapshot captureBestWindowSnapshot(Context context, String preferredPkg) {
        String safePkg = (preferredPkg == null || preferredPkg.trim().isEmpty())
            ? "com.tencent.mm"
            : preferredPkg.trim();

        UiWindowSnapshot apexSnap = null;
        if (isApexBridgeAvailable()) {
            if (!sSelfApexBlocked && context != null) {
                apexSnap = invokeSelfApex(context, safePkg, "snapshot", 0, 0, "");
            }
            if (apexSnap == null || apexSnap.root == null) {
                apexSnap = invokeAbilityApex(context, safePkg, "snapshot", 0, 0, "");
            }
        }

        UiWindowSnapshot a11ySnap = captureFromAccessibility(safePkg);
        if (apexSnap != null && apexSnap.root != null && a11ySnap != null && a11ySnap.root != null) {
            mergeAvatarsFromApexTree(apexSnap.root, a11ySnap.root);
            int apexTexts = countTextBearingNodes(apexSnap.root);
            int a11yTexts = countTextBearingNodes(a11ySnap.root);
            return a11yTexts > apexTexts ? a11ySnap : apexSnap;
        }
        if (apexSnap != null && apexSnap.root != null) return apexSnap;
        return a11ySnap;
    }

    private static void mergeAvatarsFromApexTree(UiNode srcRoot, UiNode dstRoot) {
        if (srcRoot == null || dstRoot == null) return;
        List<UiNode> srcNodes = new ArrayList<UiNode>();
        flattenNodes(srcRoot, srcNodes);
        for (UiNode sn : srcNodes) {
            if ((sn.avatarB64 != null && !sn.avatarB64.isEmpty())
                    || (sn.mediaB64 != null && !sn.mediaB64.isEmpty())) {
                dstRoot.children.add(sn);
            }
        }
    }

    private static String cropAvatarBase64FromBitmap(Bitmap screenBmp, UiBounds b) {
        if (screenBmp == null || screenBmp.isRecycled() || b == null) return "";
        try {
            int bw = screenBmp.getWidth();
            int bh = screenBmp.getHeight();
            int l = Math.max(0, Math.min(bw - 1, b.left));
            int t = Math.max(0, Math.min(bh - 1, b.top));
            int r = Math.max(l + 1, Math.min(bw, b.right));
            int bot = Math.max(t + 1, Math.min(bh, b.bottom));
            int cw = r - l;
            int ch = bot - t;
            if (cw < 28 || ch < 28) return "";

            int targetSize = 64;
            Bitmap bmp = Bitmap.createBitmap(targetSize, targetSize, Bitmap.Config.ARGB_8888);
            Canvas canvas = new Canvas(bmp);
            android.graphics.Path clipPath = new android.graphics.Path();
            clipPath.addRoundRect(
                new RectF(0, 0, targetSize, targetSize),
                12f,
                12f,
                android.graphics.Path.Direction.CW
            );
            canvas.clipPath(clipPath);
            Rect srcRect = new Rect(l, t, r, bot);
            RectF dstRect = new RectF(0, 0, targetSize, targetSize);
            android.graphics.Paint paint = new android.graphics.Paint(
                android.graphics.Paint.ANTI_ALIAS_FLAG | android.graphics.Paint.FILTER_BITMAP_FLAG
            );
            canvas.drawBitmap(screenBmp, srcRect, dstRect, paint);

            int[] sampleCoords = new int[]{
                16, 16,  32, 16,  48, 16,
                16, 32,  32, 32,  48, 32,
                16, 48,  32, 48,  48, 48
            };
            int nonTransparent = 0;
            int firstOpaqueRgb = 0;
            boolean hasColorVariance = false;
            for (int i = 0; i < sampleCoords.length; i += 2) {
                int px = bmp.getPixel(sampleCoords[i], sampleCoords[i + 1]);
                int alpha = (px >>> 24) & 0xFF;
                if (alpha > 24) {
                    int rgb = px & 0x00FFFFFF;
                    if (nonTransparent == 0) {
                        firstOpaqueRgb = rgb;
                    } else if (rgb != firstOpaqueRgb) {
                        hasColorVariance = true;
                    }
                    nonTransparent++;
                }
            }
            if (nonTransparent < 3 || !hasColorVariance) {
                bmp.recycle();
                return "";
            }
            ByteArrayOutputStream baos = new ByteArrayOutputStream(2048);
            bmp.compress(Bitmap.CompressFormat.PNG, 100, baos);
            bmp.recycle();
            byte[] bytes = baos.toByteArray();
            if (bytes.length < 80) return "";
            return Base64.encodeToString(bytes, Base64.NO_WRAP);
        } catch (Throwable ignored) {
            return "";
        }
    }

    private static String cropMediaBubbleBase64FromBitmap(Bitmap screenBmp, UiBounds b) {
        if (screenBmp == null || screenBmp.isRecycled() || b == null) return "";
        try {
            int bw = screenBmp.getWidth();
            int bh = screenBmp.getHeight();
            int l = Math.max(0, Math.min(bw - 1, b.left));
            int t = Math.max(0, Math.min(bh - 1, b.top));
            int r = Math.max(l + 1, Math.min(bw, b.right));
            int bot = Math.max(t + 1, Math.min(bh, b.bottom));
            int cw = r - l;
            int ch = bot - t;
            if (cw < 40 || ch < 40) return "";

            float scale = Math.min(1.0f, 200.0f / (float) Math.max(cw, ch));
            int targetW = Math.max(36, Math.round(cw * scale));
            int targetH = Math.max(36, Math.round(ch * scale));
            Bitmap bmp = Bitmap.createBitmap(targetW, targetH, Bitmap.Config.ARGB_8888);
            Canvas canvas = new Canvas(bmp);
            Rect srcRect = new Rect(l, t, r, bot);
            RectF dstRect = new RectF(0, 0, targetW, targetH);
            android.graphics.Paint paint = new android.graphics.Paint(
                android.graphics.Paint.ANTI_ALIAS_FLAG | android.graphics.Paint.FILTER_BITMAP_FLAG
            );
            canvas.drawBitmap(screenBmp, srcRect, dstRect, paint);

            int[] sampleCoords = new int[]{
                targetW / 4, targetH / 4,      targetW / 2, targetH / 4,      (targetW * 3) / 4, targetH / 4,
                targetW / 4, targetH / 2,      targetW / 2, targetH / 2,      (targetW * 3) / 4, targetH / 2,
                targetW / 4, (targetH * 3) / 4, targetW / 2, (targetH * 3) / 4, (targetW * 3) / 4, (targetH * 3) / 4
            };
            int nonTransparent = 0;
            int firstOpaqueRgb = 0;
            boolean hasColorVariance = false;
            for (int i = 0; i < sampleCoords.length; i += 2) {
                int sx = Math.max(0, Math.min(targetW - 1, sampleCoords[i]));
                int sy = Math.max(0, Math.min(targetH - 1, sampleCoords[i + 1]));
                int px = bmp.getPixel(sx, sy);
                int alpha = (px >>> 24) & 0xFF;
                if (alpha > 20) {
                    int rgb = px & 0x00FFFFFF;
                    if (nonTransparent == 0) {
                        firstOpaqueRgb = rgb;
                    } else if (rgb != firstOpaqueRgb) {
                        hasColorVariance = true;
                    }
                    nonTransparent++;
                }
            }
            if (nonTransparent < 2 || !hasColorVariance) {
                bmp.recycle();
                return "";
            }
            ByteArrayOutputStream baos = new ByteArrayOutputStream(4096);
            bmp.compress(Bitmap.CompressFormat.JPEG, 80, baos);
            bmp.recycle();
            byte[] bytes = baos.toByteArray();
            if (bytes.length < 100) return "";
            return "data:image/jpeg;base64," + Base64.encodeToString(bytes, Base64.NO_WRAP);
        } catch (Throwable ignored) {
            return "";
        }
    }

    private static int countTextBearingNodes(UiNode node) {
        if (node == null) return 0;
        int c = (!node.text.isEmpty() || !node.desc.isEmpty()) ? 1 : 0;
        for (UiNode child : node.children) {
            c += countTextBearingNodes(child);
        }
        return c;
    }

    private static UiWindowSnapshot invokeSelfApex(
            Context context,
            String targetPkg,
            String op,
            int x,
            int y,
            String text) {
        try {
            File roBridge = new File(READONLY_BRIDGE_DEX_PATH);
            String dexPath = (roBridge.isFile() && roBridge.canRead())
                ? READONLY_BRIDGE_DEX_PATH
                : context.getApplicationInfo().sourceDir;

            final CountDownLatch latch = new CountDownLatch(1);
            final AtomicReference<String> jsonRef = new AtomicReference<String>();
            Binder callback = new Binder() {
                @Override
                protected boolean onTransact(int code, Parcel data, Parcel reply, int flags) throws RemoteException {
                    if (code == 1) {
                        data.enforceInterface(APEX_CALLBACK_DESCRIPTOR);
                        jsonRef.set(data.readString());
                        if (reply != null) reply.writeNoException();
                        latch.countDown();
                        return true;
                    }
                    return super.onTransact(code, data, reply, flags);
                }
            };

            Intent intent = new Intent();
            Bundle bundle = new Bundle();
            // Pass empty targetDexPackage so StepOSApexViewCaptureManager.findDexPath returns immediately
            // at TextUtils.isEmpty(targetDexPackage) without overwriting dexPath.
            bundle.putString("targetDexPackage", "");
            bundle.putString("targetDexClass", ChatUiBridge.class.getName());
            bundle.putString("targetDexMethod", "handleApexRequest");
            bundle.putString("dexPath", dexPath);
            bundle.putString("dexLibrarySearchPath", "/system/lib64");
            bundle.putBinder("targetDexBinder", callback);
            bundle.putString("targetWindowTitle", "");
            bundle.putString("targetFindPackage", targetPkg);
            bundle.putString("stepxOp", op);
            bundle.putInt("stepxX", x);
            bundle.putInt("stepxY", y);
            bundle.putString("stepxText", text == null ? "" : text);
            intent.putExtras(bundle);

            Class<?> utilsCls = Class.forName("android.view.StepOSViewContentUtils");
            utilsCls.getMethod("apexRequest", Intent.class).invoke(null, intent);

            // Never permanently set sSelfApexBlocked on a timeout: a timeout simply means the target
            // package has no visible window mounted yet at that instant.
            if (!latch.await(500L, TimeUnit.MILLISECONDS)) {
                return null;
            }
            String rawJson = jsonRef.get();
            if (rawJson == null || rawJson.isEmpty()) return null;
            JSONObject obj = new JSONObject(rawJson);
            UiNode root = UiNode.fromJson(obj.optJSONObject("root"));
            if (root == null) return null;
            return new UiWindowSnapshot(
                obj.optString("pkg", targetPkg),
                "",
                obj.optInt("sw", 1080),
                obj.optInt("sh", 2400),
                root,
                "self_apex",
                obj.optBoolean("actionOk", true)
            );
        } catch (Throwable e) {
            android.util.Log.w("ChatUiBridge", "invokeSelfApex failed: " + e);
            return null;
        }
    }

    private static ClassLoader getAbilityClassLoader(Context context) throws Exception {
        if (sAbilityClassLoader != null) return sAbilityClassLoader;
        synchronized (ChatUiBridge.class) {
            if (sAbilityClassLoader != null) return sAbilityClassLoader;
            if (context != null) {
                Context abilityCtx = context.createPackageContext(
                    "com.stepos.ability",
                    Context.CONTEXT_INCLUDE_CODE | Context.CONTEXT_IGNORE_SECURITY
                );
                sAbilityClassLoader = abilityCtx.getClassLoader();
                return sAbilityClassLoader;
            }
        }
        return null;
    }

    private static UiWindowSnapshot invokeAbilityApex(
            Context context,
            String targetPkg,
            String function,
            int x,
            int y,
            String text) {
        // system_server's BootClassLoader cannot unmarshal com.mdisdk.capability.model.RequestInfo from a
        // non-system caller, so skip ability_apex to avoid a 1200ms CountDownLatch timeout per fallback.
        if (context != null) {
            return null;
        }
        try {
            final ClassLoader cl = getAbilityClassLoader(context);
            if (cl == null) return null;

            // In ability.apk's RequestHandler.process, window capture is "snapshot" (NOT "getWindowState")
            String mappedFunction = "getWindowState".equals(function) ? "snapshot" : function;

            Class<?> reqBuilderCls = cl.loadClass("com.mdisdk.capability.model.RequestInfo$Builder");
            Object builder = reqBuilderCls.getDeclaredConstructor().newInstance();
            long reqId = Math.abs(UUID.randomUUID().getLeastSignificantBits());
            if (reqId == 0L) reqId = System.currentTimeMillis();
            reqBuilderCls.getMethod("requestId", long.class).invoke(builder, Long.valueOf(reqId));
            try {
                reqBuilderCls.getMethod("displayId", Integer.class).invoke(builder, Integer.valueOf(0));
            } catch (Throwable ignored) {
            }
            reqBuilderCls.getMethod("packageName", String.class).invoke(builder, targetPkg);
            reqBuilderCls.getMethod("windowTitle", String.class).invoke(builder, "");
            reqBuilderCls.getMethod("function", String.class).invoke(builder, mappedFunction);

            if ("click".equals(mappedFunction) || "scrollDown".equals(mappedFunction)) {
                Class<?> extraCls = cl.loadClass("com.mdisdk.capability.model.ActionExtra");
                Object extra = extraCls.getMethod("click", int.class, int.class)
                    .invoke(null, Integer.valueOf(x), Integer.valueOf(y));
                reqBuilderCls.getMethod("actionExtra", extraCls).invoke(builder, extra);
            } else if ("setText".equals(mappedFunction)) {
                Class<?> extraCls = cl.loadClass("com.mdisdk.capability.model.ActionExtra");
                Object extra = extraCls.getMethod("setText", int.class, int.class, String.class)
                    .invoke(null, Integer.valueOf(x), Integer.valueOf(y), text == null ? "" : text);
                reqBuilderCls.getMethod("actionExtra", extraCls).invoke(builder, extra);
            } else if ("replaceText".equals(mappedFunction)) {
                Class<?> extraCls = cl.loadClass("com.mdisdk.capability.model.ActionExtra");
                Object extra = extraCls.getMethod("replaceText", int.class, int.class, int.class, int.class, String.class, long.class)
                    .invoke(null, Integer.valueOf(x), Integer.valueOf(y), Integer.valueOf(0), Integer.valueOf(-1), text == null ? "" : text, Long.valueOf(0L));
                reqBuilderCls.getMethod("actionExtra", extraCls).invoke(builder, extra);
            }

            Object requestInfo = reqBuilderCls.getMethod("build").invoke(builder);

            final CountDownLatch latch = new CountDownLatch(1);
            final AtomicReference<Object> resultInfoRef = new AtomicReference<Object>();

            Binder callbackBinder = new Binder() {
                @Override
                protected boolean onTransact(int code, Parcel data, Parcel reply, int flags) throws RemoteException {
                    if (code == 1) {
                        try {
                            data.enforceInterface("com.mdisdk.capability.aidl.IGrabNodeCallback");
                            int count = data.readInt();
                            Intent resIntent = data.readInt() != 0 ? Intent.CREATOR.createFromParcel(data) : null;
                            if (reply != null) reply.writeNoException();
                            if (resIntent != null) {
                                resIntent.setExtrasClassLoader(cl);
                                Bundle b = resIntent.getExtras();
                                if (b != null) {
                                    resultInfoRef.set(b.getParcelable("result"));
                                }
                            }
                        } catch (Throwable ignored) {
                        } finally {
                            latch.countDown();
                        }
                        return true;
                    } else if (code == 2) {
                        try {
                            data.enforceInterface("com.mdisdk.capability.aidl.IGrabNodeCallback");
                            if (reply != null) reply.writeNoException();
                        } catch (Throwable ignored) {
                        } finally {
                            latch.countDown();
                        }
                        return true;
                    }
                    return super.onTransact(code, data, reply, flags);
                }
            };

            Intent intent = new Intent();
            Bundle bundle = new Bundle();
            bundle.putString("targetDexPackage", "com.stepos.ability");
            bundle.putString("targetDexClass", "com.mdisdk.capability.thirdpart.RequestMain");
            bundle.putString("targetDexMethod", "handleApexRequest");
            bundle.putString("dexPath", "/system/priv-app/ability/ability.apk");
            bundle.putString("dexLibrarySearchPath", "/system/priv-app/ability/lib/arm64");
            bundle.putBinder("targetDexBinder", callbackBinder);
            bundle.putParcelable("requestInfo", (android.os.Parcelable) requestInfo);
            bundle.putString("targetWindowTitle", "");
            bundle.putString("targetFindPackage", targetPkg);
            intent.putExtras(bundle);

            Class<?> utilsCls = Class.forName("android.view.StepOSViewContentUtils");
            utilsCls.getMethod("apexRequest", Intent.class).invoke(null, intent);

            if (!latch.await(1200L, TimeUnit.MILLISECONDS)) {
                return null;
            }
            Object resultInfo = resultInfoRef.get();
            if (resultInfo == null) return null;

            Class<?> resCls = resultInfo.getClass();
            int resultCode = ((Integer) resCls.getMethod("getResultCode").invoke(resultInfo)).intValue();
            boolean actionOk = (resultCode == 0);
            Object winInfo = resCls.getMethod("getWindowStateInfo").invoke(resultInfo);
            if (winInfo == null) {
                return new UiWindowSnapshot(targetPkg, "", 1080, 2400, null, "ability_apex", actionOk);
            }
            Class<?> winCls = winInfo.getClass();
            String pkg = safeStr(winCls.getMethod("getPackageName").invoke(winInfo));
            String winTitle = safeStr(winCls.getMethod("getWindowTitle").invoke(winInfo));
            Object rootVn = winCls.getMethod("getRootNode").invoke(winInfo);
            UiNode root = convertAbilityViewNode(rootVn, 0);
            int sw = (root != null && root.bounds.right > 0) ? root.bounds.right : 1080;
            int sh = (root != null && root.bounds.bottom > 0) ? root.bounds.bottom : 2400;
            if (root == null) {
                try {
                    Object touchRegion = winCls.getMethod("getTouchRegion").invoke(winInfo);
                    if (touchRegion instanceof android.graphics.Region) {
                        Rect rb = ((android.graphics.Region) touchRegion).getBounds();
                        if (rb.right > 0) sw = rb.right;
                        if (rb.bottom > 0) sh = rb.bottom;
                    }
                } catch (Throwable ignored) {
                }
            }
            return new UiWindowSnapshot(
                pkg.isEmpty() ? targetPkg : pkg,
                winTitle,
                sw,
                sh,
                root,
                "ability_apex",
                actionOk
            );
        } catch (Throwable ignored) {
            return null;
        }
    }

    public static UiNode convertAbilityViewNode(Object vn, int depth) {
        if (vn == null || depth > 45) return null;
        try {
            Class<?> cls = vn.getClass();
            Object childrenObj = invokeOptional(cls, vn, "getChildren");
            boolean hasChildren = (childrenObj instanceof List<?>) && !((List<?>) childrenObj).isEmpty();
            Object visObj = invokeOptional(cls, vn, "isVisible");
            if (depth > 0 && !hasChildren && visObj != null && !Boolean.TRUE.equals(visObj)) {
                return null;
            }
            String text = safeStr(invokeOptional(cls, vn, "getText"));
            String desc = safeStr(invokeOptional(cls, vn, "getContentDescription"));
            String hint = safeStr(invokeOptional(cls, vn, "getHint"));
            String className = safeStr(invokeOptional(cls, vn, "getClassName"));
            String viewId = safeStr(invokeOptional(cls, vn, "getViewId"));
            UiBounds bounds = extractAbilityBounds(invokeOptional(cls, vn, "getBounds"));
            boolean clickable = Boolean.TRUE.equals(invokeOptional(cls, vn, "isClickable"));
            boolean editable = Boolean.TRUE.equals(invokeOptional(cls, vn, "isTextEditor"))
                || className.contains("EditText");
            boolean scrollable = Boolean.TRUE.equals(invokeOptional(cls, vn, "isVerticalScrollable"));

            UiNode node = new UiNode(
                text, desc, hint, className, viewId, bounds,
                clickable, editable, scrollable, true, depth, null
            );
            if (childrenObj instanceof List<?>) {
                for (Object child : (List<?>) childrenObj) {
                    UiNode c = convertAbilityViewNode(child, depth + 1);
                    if (c != null) node.children.add(c);
                }
            }
            return node;
        } catch (Throwable e) {
            return null;
        }
    }

    private static Object invokeOptional(Class<?> cls, Object target, String methodName) {
        try {
            return cls.getMethod(methodName).invoke(target);
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static UiBounds extractAbilityBounds(Object boundsObj) {
        if (boundsObj == null) {
            return new UiBounds(0, 0, 0, 0);
        }
        if (boundsObj instanceof Rect) {
            return UiBounds.fromRect((Rect) boundsObj);
        }
        try {
            Class<?> bCls = boundsObj.getClass();
            int left = ((Number) bCls.getMethod("getLeft").invoke(boundsObj)).intValue();
            int top = ((Number) bCls.getMethod("getTop").invoke(boundsObj)).intValue();
            int right = ((Number) bCls.getMethod("getRight").invoke(boundsObj)).intValue();
            int bottom = ((Number) bCls.getMethod("getBottom").invoke(boundsObj)).intValue();
            return new UiBounds(left, top, right, bottom);
        } catch (Throwable ignored) {
            return new UiBounds(0, 0, 0, 0);
        }
    }

    private static UiWindowSnapshot captureFromAccessibility(String preferredPkg) {
        GuiAccessibilityService service = GuiAccessibilityService.instance;
        if (service == null) return null;
        try {
            AccessibilityNodeInfo targetRoot = null;
            String pkg = "";
            AccessibilityNodeInfo active = service.getRootInActiveWindow();
            if (active != null && active.getPackageName() != null) {
                String activePkg = active.getPackageName().toString();
                if (preferredPkg == null || preferredPkg.isEmpty() || preferredPkg.equals(activePkg)) {
                    targetRoot = active;
                    pkg = activePkg;
                }
            }
            if (targetRoot == null) {
                List<AccessibilityWindowInfo> windows = service.getWindows();
                if (windows != null) {
                    for (AccessibilityWindowInfo win : windows) {
                        AccessibilityNodeInfo r = win.getRoot();
                        if (r == null || r.getPackageName() == null) continue;
                        String p = r.getPackageName().toString();
                        if (preferredPkg != null && !preferredPkg.isEmpty()) {
                            if (preferredPkg.equals(p)) {
                                targetRoot = r;
                                pkg = p;
                                break;
                            }
                        } else if (ImAppCatalog.isImPackage(p)) {
                            targetRoot = r;
                            pkg = p;
                            break;
                        }
                    }
                }
            }
            if (targetRoot == null) return null;
            Rect rootBounds = new Rect();
            targetRoot.getBoundsInScreen(rootBounds);
            int sw = rootBounds.width() > 0 ? rootBounds.width() : 1080;
            int sh = rootBounds.height() > 0 ? rootBounds.height() : 2400;
            UiNode root = convertAccessibilityNode(targetRoot, 0);
            if (root == null || (root.children.isEmpty() && root.text.isEmpty() && root.desc.isEmpty())) {
                return null;
            }
            return new UiWindowSnapshot(pkg, "", sw, sh, root, "accessibility", true);
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static UiNode convertAccessibilityNode(AccessibilityNodeInfo info, int depth) {
        if (info == null || depth > 45) return null;
        if (depth > 0 && info.getChildCount() == 0 && !info.isVisibleToUser()) return null;
        Rect bounds = new Rect();
        info.getBoundsInScreen(bounds);
        String text = info.getText() != null ? info.getText().toString().trim() : "";
        String desc = info.getContentDescription() != null ? info.getContentDescription().toString().trim() : "";
        String hint = "";
        try {
            if (info.getHintText() != null) hint = info.getHintText().toString().trim();
        } catch (Throwable ignored) {
        }
        String className = info.getClassName() != null ? info.getClassName().toString() : "";
        String viewId = info.getViewIdResourceName() != null ? info.getViewIdResourceName() : "";
        boolean clickable = info.isClickable();
        boolean editable = info.isEditable() || className.contains("EditText");
        boolean scrollable = info.isScrollable();

        UiNode node = new UiNode(
            text, desc, hint, className, viewId, UiBounds.fromRect(bounds),
            clickable, editable, scrollable, true, depth, info
        );
        int count = info.getChildCount();
        for (int i = 0; i < count; i++) {
            UiNode child = convertAccessibilityNode(info.getChild(i), depth + 1);
            if (child != null) node.children.add(child);
        }
        return node;
    }

    private static boolean performNodeClick(Context context, UiWindowSnapshot snap, UiNode node) {
        if (node == null) return false;
        int cx = node.bounds.centerX();
        int cy = node.bounds.centerY();

        if (isApexBridgeAvailable() && snap != null && !snap.packageName.isEmpty()) {
            if (!sSelfApexBlocked && context != null) {
                UiWindowSnapshot res = invokeSelfApex(context, snap.packageName, "click", cx, cy, "");
                if (res != null && res.actionResult) return true;
            }
            UiWindowSnapshot abRes = invokeAbilityApex(context, snap.packageName, "click", cx, cy, "");
            if (abRes != null && abRes.actionResult) return true;
        }

        if (node.a11yNode != null) {
            if (clickAccessibilityNodeOrParent(node.a11yNode)) return true;
        }
        GuiAccessibilityService service = GuiAccessibilityService.instance;
        if (service != null && cx > 0 && cy > 0) {
            return dispatchAccessibilityTap(service, cx, cy);
        }
        return false;
    }

    private static boolean performNodeSetText(Context context, UiWindowSnapshot snap, UiNode node, String text) {
        if (node == null) return false;
        int cx = node.bounds.centerX();
        int cy = node.bounds.centerY();

        if (isApexBridgeAvailable() && snap != null && !snap.packageName.isEmpty()) {
            if (!sSelfApexBlocked && context != null) {
                UiWindowSnapshot res = invokeSelfApex(context, snap.packageName, "setText", cx, cy, text);
                if (res != null && res.actionResult) return true;
            }
            UiWindowSnapshot abRes = invokeAbilityApex(context, snap.packageName, "setText", cx, cy, text);
            if (abRes != null && abRes.actionResult) return true;
            UiWindowSnapshot repRes = invokeAbilityApex(context, snap.packageName, "replaceText", cx, cy, text);
            if (repRes != null && repRes.actionResult) return true;
        }

        if (node.a11yNode != null) {
            try {
                node.a11yNode.performAction(AccessibilityNodeInfo.ACTION_FOCUS);
                Bundle args = new Bundle();
                args.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text);
                if (node.a11yNode.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)) {
                    return true;
                }
            } catch (Throwable ignored) {
            }
        }
        return false;
    }

    private static boolean performBackNavigation(Context context, UiWindowSnapshot snap) {
        if (snap != null && snap.root != null) {
            UiNode backBtn = findTopLeftBackNode(snap.root, snap.screenWidth, snap.screenHeight);
            if (backBtn != null && performNodeClick(context, snap, backBtn)) {
                return true;
            }
        }
        GuiAccessibilityService service = GuiAccessibilityService.instance;
        if (service != null) {
            try {
                if (service.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)) {
                    return true;
                }
            } catch (Throwable ignored) {
            }
        }
        try {
            Runtime.getRuntime().exec(new String[]{"input", "keyevent", "4"});
            return true;
        } catch (Throwable ignored) {
        }
        return false;
    }

    private static boolean performScrollDown(Context context, UiWindowSnapshot snap) {
        String pkg = (snap != null && snap.packageName != null) ? snap.packageName : "";
        int sw = (snap != null && snap.screenWidth > 0) ? snap.screenWidth : 1080;
        int sh = (snap != null && snap.screenHeight > 0) ? snap.screenHeight : 2400;
        int cx = sw / 2;
        int cy = sh / 2;
        UiWindowSnapshot scrolled = invokeSelfApex(context, pkg, "scrollDown", cx, cy, "");
        if (scrolled != null && scrolled.actionResult) {
            return true;
        }
        scrolled = invokeAbilityApex(context, pkg, "scrollDown", cx, cy, "");
        if (scrolled != null && scrolled.actionResult) {
            return true;
        }
        GuiAccessibilityService service = GuiAccessibilityService.instance;
        if (service != null) {
            try {
                AccessibilityNodeInfo root = service.getRootInActiveWindow();
                if (root != null) {
                    boolean ok = performAccessibilityScrollRecursive(root);
                    try {
                        root.recycle();
                    } catch (Throwable ignored) {
                    }
                    if (ok) {
                        return true;
                    }
                }
            } catch (Throwable ignored) {
            }
        }
        return false;
    }

    private static boolean performAccessibilityScrollRecursive(AccessibilityNodeInfo node) {
        if (node == null) return false;
        try {
            if (node.isScrollable()) {
                if (node.performAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD)) {
                    return true;
                }
            }
            int count = node.getChildCount();
            for (int i = 0; i < count; i++) {
                AccessibilityNodeInfo child = node.getChild(i);
                if (child == null) continue;
                boolean ok = performAccessibilityScrollRecursive(child);
                try {
                    child.recycle();
                } catch (Throwable ignored) {
                }
                if (ok) return true;
            }
        } catch (Throwable ignored) {
        }
        return false;
    }

    private static UiNode findTopLeftBackNode(UiNode root, int screenWidth, int screenHeight) {
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);
        for (UiNode n : all) {
            if (n.bounds.top >= 0 && n.bounds.bottom <= screenHeight * 0.16
                    && n.bounds.left >= 0 && n.bounds.right <= screenWidth * 0.24) {
                if ("返回".equals(n.desc) || "返回".equals(n.text)
                        || n.desc.contains("返回") || n.desc.toLowerCase(Locale.ROOT).contains("back")
                        || n.clickable) {
                    return n;
                }
            }
        }
        return null;
    }

    private static UiWindowSnapshot tryOpenConversationViaSearch(
            Context context,
            UiWindowSnapshot snap,
            String safePkg,
            String cleanTargetTitle) {
        if (snap == null || snap.root == null || cleanTargetTitle == null || cleanTargetTitle.isEmpty()) {
            return null;
        }
        try {
            UiNode searchBtn = findTopBarSearchNode(snap.root, snap.screenWidth, snap.screenHeight);
            if (searchBtn == null) return null;
            if (!performNodeClick(context, snap, searchBtn)) return null;
            Thread.sleep(500L);
            UiWindowSnapshot searchPage = captureBestWindowSnapshot(context, safePkg);
            if (searchPage == null || searchPage.root == null) return null;
            UiNode searchInput = findTopSearchInputNode(searchPage.root, searchPage.screenHeight);
            if (searchInput == null) return null;
            if (!performNodeSetText(context, searchPage, searchInput, cleanTargetTitle)) {
                performNodeClick(context, searchPage, searchInput);
                Thread.sleep(200L);
                UiWindowSnapshot refocused = captureBestWindowSnapshot(context, safePkg);
                if (refocused != null && refocused.root != null) {
                    searchPage = refocused;
                    UiNode retryInput = findTopSearchInputNode(searchPage.root, searchPage.screenHeight);
                    if (retryInput != null) searchInput = retryInput;
                }
                if (!performNodeSetText(context, searchPage, searchInput, cleanTargetTitle)) {
                    return null;
                }
            }
            Thread.sleep(600L);
            UiWindowSnapshot resultsSnap = captureBestWindowSnapshot(context, safePkg);
            if (resultsSnap == null || resultsSnap.root == null) return null;
            UiNode resultRow = findSearchResultRowNode(resultsSnap.root, cleanTargetTitle, resultsSnap.screenHeight);
            if (resultRow == null) return null;
            performNodeClick(context, resultsSnap, resultRow);
            Thread.sleep(650L);
            return captureBestWindowSnapshot(context, safePkg);
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static UiNode findTopBarSearchNode(UiNode root, int sw, int sh) {
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);
        for (UiNode n : all) {
            if (n.bounds.top >= 0 && n.bounds.bottom <= sh * 0.16 && n.bounds.left >= sw * 0.60) {
                if ("搜索".equals(n.desc) || "搜索".equals(n.text)
                        || n.desc.contains("搜索") || n.desc.toLowerCase(Locale.ROOT).contains("search")) {
                    return n;
                }
            }
        }
        return null;
    }

    private static UiNode findTopSearchInputNode(UiNode root, int sh) {
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);
        for (UiNode n : all) {
            if (n.editable && n.bounds.top >= 0 && n.bounds.bottom <= sh * 0.25
                    && n.bounds.width() > 80 && n.bounds.height() > 24) {
                return n;
            }
        }
        return null;
    }

    private static UiNode findSearchResultRowNode(UiNode root, String targetTitle, int sh) {
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);
        for (UiNode n : all) {
            if (n.editable) continue;
            if (n.bounds.top < sh * 0.12 || n.bounds.bottom > sh * 0.85) continue;
            String val = !n.text.isEmpty() ? n.text : n.desc;
            if (targetTitle.equals(val) || val.startsWith(targetTitle)) {
                return n;
            }
        }
        return null;
    }

    /**
     * Parses a window snapshot into either a Conversation List (with preview rows)
     * or a Chat Window (with top-bar title, group indicator, and sender-attributed message bubbles).
     */
    public static ParsedChatWindow parseWindowSnapshot(UiWindowSnapshot snap) {
        if (snap == null || snap.root == null) {
            return new ParsedChatWindow("", "", false, false, null, null);
        }
        int sw = snap.screenWidth > 0 ? snap.screenWidth : 1080;
        int sh = snap.screenHeight > 0 ? snap.screenHeight : 2400;

        boolean hasChatInput = findChatInputNode(snap.root, sh) != null
            || findVoiceToKeyboardSwitchNode(snap.root, sh) != null;
        if (!hasChatInput) {
            List<PushStore.CapturedConversationRow> rows = extractConversationRows(snap.root, snap.packageName, sw, sh);
            if (isMainTabListScreen(snap.root, sh) || rows.size() >= 2) {
                return new ParsedChatWindow("", "", false, true, null, rows);
            }
            return new ParsedChatWindow("", "", false, false, null, null);
        }

        List<UiNode> allNodes = new ArrayList<UiNode>();
        flattenNodes(snap.root, allNodes);

        // 1. Collect avatar nodes (with extracted sender nickname from contentDescription and live avatarB64)
        List<AvatarMarker> avatars = new ArrayList<AvatarMarker>();
        java.util.Set<UiNode> avatarNodeSet = new java.util.HashSet<UiNode>();
        int minAvatarPx = Math.min(76, Math.max(36, Math.round(28f * (sw / 411f))));
        int maxAvatarPx = Math.max(minAvatarPx, (int) (sw * 0.16f));
        Bitmap chatScreenBmp = null;
        boolean chatScreenAttempted = false;
        try {
            for (UiNode n : allNodes) {
                if (n.bounds.top < sh * 0.08 || n.bounds.bottom > sh * 0.90) continue;
                if (n.bounds.width() < minAvatarPx || n.bounds.width() > maxAvatarPx) continue;
                if (n.bounds.height() < minAvatarPx || n.bounds.height() > maxAvatarPx) continue;
                if (Math.abs(n.bounds.width() - n.bounds.height()) > 24) continue;
                String senderName = ImAppCatalog.extractAvatarSenderName(n.desc);
                int cx = n.bounds.centerX();
                boolean isLeft = n.bounds.left <= sw * 0.18f && cx <= sw * 0.22f;
                boolean isRight = n.bounds.right >= sw * 0.82f && cx >= sw * 0.78f;
                if (!isLeft && !isRight) continue;
                boolean isSquareEdgeAvatar = n.text.isEmpty();
                if (!senderName.isEmpty()
                        || !n.avatarB64.isEmpty()
                        || n.className.contains("Image")
                        || n.className.contains("Avatar")
                        || isSquareEdgeAvatar) {
                    String avB64 = n.avatarB64;
                    if ((avB64 == null || avB64.isEmpty()) && Math.abs(n.bounds.width() - n.bounds.height()) <= 32) {
                        if (!chatScreenAttempted) {
                            chatScreenAttempted = true;
                            chatScreenBmp = GuiAccessibilityService.captureScreenBitmapSync(900L);
                        }
                        if (chatScreenBmp != null) {
                            avB64 = cropAvatarBase64FromBitmap(chatScreenBmp, n.bounds);
                        }
                    }
                    avatars.add(new AvatarMarker(senderName, n.bounds, isLeft, isRight, avB64));
                    avatarNodeSet.add(n);
                }
            }

            // 2. Detect top-bar conversation title
            String rawTitle = detectTopBarTitle(allNodes, sw, sh);
            String cleanTitle = ImAppCatalog.cleanConversationTitle(snap.packageName, rawTitle);
            boolean isGroup = ImAppCatalog.hasGroupMemberCountSuffix(rawTitle)
                || "群聊".equals(cleanTitle);

            // 3. Determine vertical cutoff of the bottom input bar
            int topBarBottom = (int) (sh * 0.11);
            int inputBarTop = (int) (sh * 0.88);
            UiNode inputNode = findChatInputNode(snap.root, sh);
            if (inputNode != null && inputNode.bounds.top > sh * 0.50) {
                inputBarTop = Math.min(inputBarTop, inputNode.bounds.top - 8);
            }

            // 4. Collect candidate text and media/sticker nodes in the chat message area
            List<UiNode> textNodes = new ArrayList<UiNode>();
            for (UiNode n : allNodes) {
                if (n.editable || avatarNodeSet.contains(n)) continue;
                if (n.bounds.top < topBarBottom || n.bounds.bottom > inputBarTop) continue;
                if (n.bounds.width() <= 10 || n.bounds.height() <= 10) continue;

                // Skip avatar ImageViews so their contentDescription ("张三头像") is not treated as a bubble
                if (!ImAppCatalog.extractAvatarSenderName(n.desc).isEmpty() && n.text.isEmpty()) {
                    continue;
                }
                String val = !n.text.isEmpty() ? n.text.trim() : n.desc.trim();
                boolean isNonAvatarImageBubble = val.isEmpty()
                    && n.className != null
                    && n.className.contains("ImageView")
                    && n.children.isEmpty()
                    && n.bounds.width() >= 60
                    && n.bounds.height() >= 60
                    && n.bounds.width() <= sw * 0.72f
                    && n.bounds.height() <= sh * 0.50f;
                if (isNonAvatarImageBubble) {
                    boolean overlapsAvatar = false;
                    for (AvatarMarker a : avatars) {
                        if (Math.abs(a.bounds.centerX() - n.bounds.centerX()) <= 28
                                && Math.abs(a.bounds.centerY() - n.bounds.centerY()) <= 28) {
                            overlapsAvatar = true;
                            break;
                        }
                    }
                    if (!overlapsAvatar) {
                        val = "[图片]";
                        String mediaB64 = n.mediaB64;
                        if (mediaB64 == null || mediaB64.isEmpty()) {
                            if (!chatScreenAttempted) {
                                chatScreenAttempted = true;
                                chatScreenBmp = GuiAccessibilityService.captureScreenBitmapSync(900L);
                            }
                            if (chatScreenBmp != null) {
                                mediaB64 = cropMediaBubbleBase64FromBitmap(chatScreenBmp, n.bounds);
                            }
                        }
                        n = new UiNode(
                            val, val, n.hint, n.className, n.viewId, n.bounds,
                            n.clickable, n.editable, n.scrollable, n.visible, n.depth, null, n.avatarB64
                        );
                        n.mediaB64 = mediaB64 == null ? "" : mediaB64;
                    }
                }
                if (val.isEmpty()) continue;
                if (!n.children.isEmpty() && hasChildWithSameText(n, val)) {
                    continue;
                }
                if (val.equals(rawTitle) || val.equals(cleanTitle)) continue;
                if (isSystemStatusBarNoise(val) || isChatInputChromeLabel(val) || isTimestampOrSystemTip(val)) {
                    continue;
                }
                String normVal = normalizeVoiceBubbleText(val);
                if (("[图片]".equals(normVal) || "[动画表情]".equals(normVal))
                        && (n.mediaB64 == null || n.mediaB64.isEmpty())
                        && n.bounds.width() >= 48
                        && n.bounds.height() >= 48) {
                    if (!chatScreenAttempted) {
                        chatScreenAttempted = true;
                        chatScreenBmp = GuiAccessibilityService.captureScreenBitmapSync(900L);
                    }
                    if (chatScreenBmp != null) {
                        n.mediaB64 = cropMediaBubbleBase64FromBitmap(chatScreenBmp, n.bounds);
                    }
                }
                textNodes.add(n);
            }

            // Synthesize sticker/image bubble for any chat row avatar that has no text node aligned with it
            for (AvatarMarker a : avatars) {
                if (!a.isLeft && !a.isRight) continue;
                if (a.bounds.top < topBarBottom || a.bounds.bottom > inputBarTop) continue;
                if ("文件传输助手".equals(cleanTitle) && a.isLeft) continue;
                boolean hasAlignedText = false;
                for (UiNode tn : textNodes) {
                    boolean sameSideOrCenter = a.isRight
                        ? (tn.bounds.right >= sw * 0.45f || tn.bounds.centerX() >= sw * 0.40f)
                        : (tn.bounds.left <= sw * 0.55f || tn.bounds.centerX() <= sw * 0.60f);
                    if (sameSideOrCenter
                            && tn.bounds.bottom >= a.bounds.top - 45
                            && tn.bounds.top <= a.bounds.bottom + 60) {
                        hasAlignedText = true;
                        break;
                    }
                }
                if (!hasAlignedText) {
                    UiNode candidateNode = null;
                    for (UiNode cand : allNodes) {
                        if (cand.editable || avatarNodeSet.contains(cand)) continue;
                        if (cand.bounds.top < topBarBottom || cand.bounds.bottom > inputBarTop) continue;
                        int cw = cand.bounds.width();
                        int ch = cand.bounds.height();
                        if (cw < 48 || ch < 48 || cw > sw * 0.68f || ch > sh * 0.42f) continue;
                        boolean verticallyAligned = cand.bounds.bottom >= a.bounds.top - 30
                            && cand.bounds.top <= a.bounds.bottom + 80;
                        boolean sameSide = a.isRight
                            ? cand.bounds.right >= sw * 0.45f
                            : cand.bounds.left <= sw * 0.55f;
                        if (verticallyAligned && sameSide) {
                            if (candidateNode == null
                                    || (!cand.mediaB64.isEmpty() && candidateNode.mediaB64.isEmpty())
                                    || (cand.children.isEmpty() && !candidateNode.children.isEmpty())) {
                                candidateNode = cand;
                            }
                        }
                    }
                    int synthH = Math.max(a.bounds.height(), Math.min(180, (int) (sw * 0.26f)));
                    UiBounds syntheticBounds = a.isRight
                        ? new UiBounds(Math.max(0, a.bounds.left - 180), a.bounds.top, a.bounds.left - 8, a.bounds.top + synthH)
                        : new UiBounds(a.bounds.right + 8, a.bounds.top, Math.min(sw, a.bounds.right + 180), a.bounds.top + synthH);
                    UiBounds targetBounds = candidateNode != null ? candidateNode.bounds : syntheticBounds;
                    String mediaB64 = (candidateNode != null && candidateNode.mediaB64 != null)
                        ? candidateNode.mediaB64
                        : "";
                    if (mediaB64.isEmpty()) {
                        if (!chatScreenAttempted) {
                            chatScreenAttempted = true;
                            chatScreenBmp = GuiAccessibilityService.captureScreenBitmapSync(900L);
                        }
                        if (chatScreenBmp != null) {
                            mediaB64 = cropMediaBubbleBase64FromBitmap(chatScreenBmp, targetBounds);
                        }
                    }
                    UiNode stickerNode = new UiNode(
                        "[动画表情]", "[动画表情]", "", "android.widget.ImageView", "",
                        targetBounds, false, false, false, true, 1, null, a.avatarB64
                    );
                    stickerNode.mediaB64 = mediaB64 == null ? "" : mediaB64;
                    textNodes.add(stickerNode);
                }
            }

            Collections.sort(textNodes, new Comparator<UiNode>() {
                @Override
                public int compare(UiNode a, UiNode b) {
                    int dy = Integer.compare(a.bounds.top, b.bounds.top);
                    return dy != 0 ? dy : Integer.compare(a.bounds.left, b.bounds.left);
                }
            });

            List<PushStore.CapturedBubble> bubbles = new ArrayList<PushStore.CapturedBubble>();
            Map<String, String> senderAvatars = new HashMap<String, String>();
            String windowAvatarB64 = "";

            boolean isServiceNotificationWindow = PushStore.isServiceNotificationContainerTitle(cleanTitle)
                || PushStore.isServiceNotificationContainerTitle(rawTitle);

            if (isServiceNotificationWindow && !textNodes.isEmpty()) {
                // Group vertically contiguous card lines in "服务通知" / "公众号" into structured multi-line card bubbles
                int maxCardGap = Math.max(64, (int) (sh * 0.045));
                List<List<UiNode>> cardGroups = new ArrayList<List<UiNode>>();
                List<UiNode> currentCard = new ArrayList<UiNode>();
                int currentCardBottom = -1;

                for (UiNode tn : textNodes) {
                    if (currentCard.isEmpty()) {
                        currentCard.add(tn);
                        currentCardBottom = tn.bounds.bottom;
                    } else if (tn.bounds.top - currentCardBottom <= maxCardGap) {
                        currentCard.add(tn);
                        currentCardBottom = Math.max(currentCardBottom, tn.bounds.bottom);
                    } else {
                        cardGroups.add(currentCard);
                        currentCard = new ArrayList<UiNode>();
                        currentCard.add(tn);
                        currentCardBottom = tn.bounds.bottom;
                    }
                }
                if (!currentCard.isEmpty()) {
                    cardGroups.add(currentCard);
                }

                for (List<UiNode> card : cardGroups) {
                    if (card.isEmpty()) continue;
                    int cardTop = card.get(0).bounds.top;
                    int cardBottom = card.get(card.size() - 1).bounds.bottom;
                    String merchantSender = "";
                    int bodyStartIndex = 0;

                    UiNode firstNode = card.get(0);
                    String firstVal = (!firstNode.text.isEmpty() ? firstNode.text : firstNode.desc).trim();
                    String cleanedMerchant = firstVal.replaceAll("[+＞>]+$", "").trim();
                    if (card.size() >= 2
                            && !cleanedMerchant.isEmpty()
                            && cleanedMerchant.length() <= 28
                            && !cleanedMerchant.contains(":")
                            && !cleanedMerchant.contains("：")) {
                        merchantSender = cleanedMerchant;
                        bodyStartIndex = 1;
                    }

                    StringBuilder bodySb = new StringBuilder();
                    UiNode prevNode = null;
                    for (int idx = bodyStartIndex; idx < card.size(); idx++) {
                        UiNode cn = card.get(idx);
                        String lineVal = (!cn.text.isEmpty() ? cn.text : cn.desc).trim();
                        if (lineVal.isEmpty() || "小程序".equals(lineVal) || "拒收此消息".equals(lineVal)) {
                            continue;
                        }
                        if (bodySb.length() == 0) {
                            bodySb.append(lineVal);
                        } else if (prevNode != null
                                && Math.abs(cn.bounds.top - prevNode.bounds.top) <= 20
                                && cn.bounds.left > prevNode.bounds.right - 10) {
                            bodySb.append(": ").append(lineVal);
                        } else {
                            bodySb.append("\n").append(lineVal);
                        }
                        prevNode = cn;
                    }

                    String cardBody = bodySb.toString().trim();
                    if (cardBody.isEmpty() && !firstVal.isEmpty()) {
                        cardBody = firstVal;
                    }
                    if (cardBody.isEmpty()) continue;

                    AvatarMarker cardAvatar = findAlignedLeftAvatar(avatars, cardTop - 20, cardBottom + 20);
                    String cardAvatarB64 = (cardAvatar != null) ? cardAvatar.avatarB64 : "";
                    String effectiveSender = !merchantSender.isEmpty() ? merchantSender : cleanTitle;
                    if (!cardAvatarB64.isEmpty() && !effectiveSender.isEmpty()) {
                        senderAvatars.put(effectiveSender, cardAvatarB64);
                    }
                    bubbles.add(new PushStore.CapturedBubble(
                        effectiveSender,
                        cardBody,
                        false,
                        0L,
                        cardAvatarB64
                    ));
                }

                return new ParsedChatWindow(
                    rawTitle,
                    cleanTitle,
                    false,
                    false,
                    bubbles,
                    null,
                    windowAvatarB64,
                    senderAvatars
                );
            }

            // 5. Identify nickname header labels right above incoming group message bubbles
            boolean[] consumedAsHeader = new boolean[textNodes.size()];
            String[] headerSenderForBubble = new String[textNodes.size()];

            for (int i = 0; i < textNodes.size(); i++) {
                if (consumedAsHeader[i]) continue;
                UiNode upper = textNodes.get(i);
                String upperText = !upper.text.isEmpty() ? upper.text : upper.desc;
                if (upper.bounds.left >= sw * 0.45
                        || upper.bounds.height() > 76
                        || upperText.length() > 24
                        || upperText.contains("\n")
                        || isVoiceDurationText(upperText)) {
                    continue;
                }
                for (int j = i + 1; j < textNodes.size(); j++) {
                    UiNode lower = textNodes.get(j);
                    int vGap = lower.bounds.top - upper.bounds.bottom;
                    if (vGap > 48) break;
                    if (vGap >= -12 && Math.abs(upper.bounds.left - lower.bounds.left) <= 48
                            && lower.bounds.left < sw * 0.45) {
                        AvatarMarker matchingAvatar = findAlignedLeftAvatar(avatars, upper.bounds.top, lower.bounds.bottom);
                        boolean matchesAvatarName = matchingAvatar != null
                            && !matchingAvatar.senderName.isEmpty()
                            && matchingAvatar.senderName.equals(upperText);
                        if (isGroup || matchesAvatarName || (matchingAvatar != null && Math.abs(upper.bounds.top - matchingAvatar.bounds.top) <= 35)) {
                            consumedAsHeader[i] = true;
                            headerSenderForBubble[j] = upperText;
                            isGroup = true;
                            break;
                        }
                    }
                }
            }

            // 6. Build CapturedBubble list with accurate sender attribution and live avatar bitmaps
            for (int i = 0; i < textNodes.size(); i++) {
                if (consumedAsHeader[i]) continue;
                UiNode n = textNodes.get(i);
                String body = normalizeVoiceBubbleText((!n.text.isEmpty() ? n.text : n.desc).trim());
                if (body.isEmpty()) continue;

                AvatarMarker leftAvatar = findAlignedLeftAvatar(avatars, n.bounds.top, n.bounds.bottom);
                AvatarMarker rightAvatar = findAlignedRightAvatar(avatars, n.bounds.top, n.bounds.bottom);

                boolean outgoing;
                if (rightAvatar != null && leftAvatar == null) {
                    outgoing = true;
                } else if (leftAvatar != null && rightAvatar == null) {
                    outgoing = false;
                } else if (rightAvatar != null && leftAvatar != null) {
                    int rightDist = Math.abs(rightAvatar.bounds.top - n.bounds.top);
                    int leftDist = Math.abs(leftAvatar.bounds.top - n.bounds.top);
                    if (rightDist + 8 < leftDist) {
                        outgoing = true;
                    } else if (leftDist + 8 < rightDist) {
                        outgoing = false;
                    } else {
                        outgoing = n.bounds.right >= sw * 0.72f || n.bounds.centerX() > sw * 0.52f;
                    }
                } else {
                    outgoing = n.bounds.centerX() > sw * 0.52f
                        || (n.bounds.right >= sw * 0.72f && leftAvatar == null)
                        || (n.bounds.left > sw * 0.28f && (sw - n.bounds.right) < n.bounds.left);
                }

                String sender;
                String bubbleAvatarB64 = "";
                if (outgoing) {
                    sender = "我";
                    if (rightAvatar != null && !rightAvatar.avatarB64.isEmpty()) {
                        bubbleAvatarB64 = rightAvatar.avatarB64;
                        senderAvatars.put("我", bubbleAvatarB64);
                    }
                } else {
                    String fromHeader = headerSenderForBubble[i];
                    String fromAvatar = (leftAvatar != null) ? leftAvatar.senderName : "";
                    if (leftAvatar != null && !leftAvatar.avatarB64.isEmpty()) {
                        bubbleAvatarB64 = leftAvatar.avatarB64;
                    }
                    if (fromHeader != null && !fromHeader.isEmpty() && !ImAppCatalog.isGenericSenderPlaceholder(fromHeader)) {
                        sender = fromHeader;
                        isGroup = true;
                    } else if (fromAvatar != null && !fromAvatar.isEmpty() && !ImAppCatalog.isGenericSenderPlaceholder(fromAvatar)) {
                        sender = fromAvatar;
                        if (!sender.equals(cleanTitle)) {
                            isGroup = true;
                        }
                    } else if (!isGroup && !ImAppCatalog.isGenericPlaceholderTitle(cleanTitle)) {
                        sender = cleanTitle;
                    } else {
                        sender = "";
                    }
                    if (!bubbleAvatarB64.isEmpty()) {
                        if (!sender.isEmpty()) {
                            senderAvatars.put(sender, bubbleAvatarB64);
                        }
                        if (!isGroup && windowAvatarB64.isEmpty()) {
                            windowAvatarB64 = bubbleAvatarB64;
                        }
                    }
                }
                bubbles.add(new PushStore.CapturedBubble(sender, body, outgoing, 0L, bubbleAvatarB64, n.mediaB64));
            }

            if (!isGroup && windowAvatarB64.isEmpty()) {
                for (AvatarMarker a : avatars) {
                    if (a.isLeft && !a.avatarB64.isEmpty()) {
                        windowAvatarB64 = a.avatarB64;
                        if (!cleanTitle.isEmpty()) {
                            senderAvatars.put(cleanTitle, windowAvatarB64);
                        }
                        break;
                    }
                }
            }

            return new ParsedChatWindow(
                rawTitle,
                cleanTitle,
                isGroup,
                false,
                bubbles,
                null,
                windowAvatarB64,
                senderAvatars
            );
        } finally {
            if (chatScreenBmp != null && !chatScreenBmp.isRecycled()) {
                chatScreenBmp.recycle();
            }
        }
    }

    static boolean isVoiceDurationText(String raw) {
        if (raw == null) return false;
        String s = raw.trim();
        return s.matches("^\\d{1,3}\\s*[\"″''”]+$")
            || s.matches("^\\[\\s*语音\\s*\\].*")
            || normalizeVoiceBubbleText(s).startsWith("[语音]");
    }

    static String normalizeVoiceBubbleText(String raw) {
        if (raw == null) return "";
        String s = raw.trim();
        if ("图片".equals(s)) return "[图片]";
        if ("动画表情".equals(s) || "微信动画表情".equals(s)) return "[动画表情]";
        if ("视频".equals(s)) return "[视频]";
        if ("小程序卡片".equals(s) || "微信小程序".equals(s)) return "[小程序]";
        if (s.matches("^\\d{1,3}\\s*[\"″''”]+$")) {
            return "[语音] " + s.replaceAll("\\s+", "");
        }
        if (s.matches("^\\[\\s*语音\\s*\\].*")) {
            String rest = s.replaceFirst("^\\[\\s*语音\\s*\\]\\s*", "").trim();
            return rest.isEmpty() ? "[语音]" : ("[语音] " + rest);
        }
        if (s.contains("语音")) {
            String withoutUnplayed = s.replace("未播放", "").trim();
            if (withoutUnplayed.startsWith("语音") || withoutUnplayed.endsWith("语音")) {
                String numPart = withoutUnplayed.replace("语音", "").trim();
                boolean hasUnit = numPart.contains("秒")
                    || numPart.contains("\"")
                    || numPart.contains("″")
                    || numPart.contains("'")
                    || numPart.contains("”");
                String digits = numPart.replaceAll("[\\s秒\"″'”]+", "");
                if (hasUnit && digits.matches("^\\d{1,3}$")) {
                    return "[语音] " + digits + "\"";
                }
            }
        }
        return s;
    }

    private static final class AvatarMarker {
        final String senderName;
        final UiBounds bounds;
        final boolean isLeft;
        final boolean isRight;
        final String avatarB64;

        AvatarMarker(String senderName, UiBounds bounds, boolean isLeft, boolean isRight) {
            this(senderName, bounds, isLeft, isRight, "");
        }

        AvatarMarker(String senderName, UiBounds bounds, boolean isLeft, boolean isRight, String avatarB64) {
            this.senderName = senderName == null ? "" : senderName;
            this.bounds = bounds;
            this.isLeft = isLeft;
            this.isRight = isRight;
            this.avatarB64 = avatarB64 == null ? "" : avatarB64;
        }
    }

    private static AvatarMarker findAlignedLeftAvatar(List<AvatarMarker> avatars, int top, int bottom) {
        AvatarMarker best = null;
        int bestDist = Integer.MAX_VALUE;
        for (AvatarMarker a : avatars) {
            if (!a.isLeft) continue;
            if (a.bounds.bottom >= top - 45 && a.bounds.top <= bottom + 45) {
                int dist = Math.abs(a.bounds.top - top);
                if (dist < bestDist) {
                    bestDist = dist;
                    best = a;
                }
            }
        }
        return best;
    }

    private static AvatarMarker findAlignedRightAvatar(List<AvatarMarker> avatars, int top, int bottom) {
        AvatarMarker best = null;
        int bestDist = Integer.MAX_VALUE;
        for (AvatarMarker a : avatars) {
            if (!a.isRight) continue;
            if (a.bounds.bottom >= top - 45 && a.bounds.top <= bottom + 45) {
                int dist = Math.abs(a.bounds.top - top);
                if (dist < bestDist) {
                    bestDist = dist;
                    best = a;
                }
            }
        }
        return best;
    }

    private static String detectTopBarTitle(List<UiNode> allNodes, int sw, int sh) {
        String best = "";
        int bestScore = -1;
        for (UiNode n : allNodes) {
            if (n.editable) continue;
            if (n.bounds.top < 0 || n.bounds.bottom > sh * 0.145 || n.bounds.top > sh * 0.125) continue;
            if (n.bounds.left > sw * 0.75) continue;
            String val = (!n.text.isEmpty() ? n.text : n.desc).trim();
            if (val.isEmpty() || val.length() > 42) continue;
            if (!ImAppCatalog.extractAvatarSenderName(val).isEmpty()) continue;
            if (isSystemStatusBarNoise(val) || isTopBarChromeLabel(val)) continue;

            int score = 10;
            if (ImAppCatalog.hasGroupMemberCountSuffix(val)) {
                score += 50;
            }
            if (n.bounds.left >= sw * 0.12 && n.bounds.left <= sw * 0.55) {
                score += 20;
            }
            if (!ImAppCatalog.isGenericPlaceholderTitle(val)) {
                score += 15;
            }
            if (score > bestScore) {
                bestScore = score;
                best = val;
            }
        }
        return best;
    }

    private static List<PushStore.CapturedConversationRow> extractConversationRows(
            UiNode root,
            String pkg,
            int sw,
            int sh) {
        List<PushStore.CapturedConversationRow> rows = new ArrayList<PushStore.CapturedConversationRow>();
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);

        List<UiNode> rowContainers = new ArrayList<UiNode>();
        for (UiNode n : all) {
            if (n.bounds.top < sh * 0.08 || n.bounds.bottom > sh * 0.91) continue;
            if (n.bounds.width() < sw * 0.75) continue;
            int h = n.bounds.height();
            if (h < 70 || h > 320) continue;
            // Skip outer container if one of its children is also a full-width row container in the same band
            boolean hasFullWidthRowChild = false;
            for (UiNode c : n.children) {
                if (c.bounds.width() >= sw * 0.75 && c.bounds.height() >= 70 && c.bounds.height() <= 320) {
                    hasFullWidthRowChild = true;
                    break;
                }
            }
            if (hasFullWidthRowChild) continue;
            // Avoid duplicate overlapping row containers at the same vertical center
            int midY = n.bounds.centerY();
            boolean duplicateBand = false;
            for (UiNode existingRow : rowContainers) {
                if (Math.abs(existingRow.bounds.centerY() - midY) < 40) {
                    duplicateBand = true;
                    break;
                }
            }
            if (!duplicateBand) {
                rowContainers.add(n);
            }
        }

        Bitmap listScreenBmp = null;
        boolean listScreenAttempted = false;
        try {
            for (UiNode row : rowContainers) {
                List<UiNode> rowTexts = new ArrayList<UiNode>();
                flattenNodes(row, rowTexts);
                String title = "";
                String preview = "";
                int unread = 0;
                String rowAvatarB64 = "";
                UiBounds avatarBounds = null;
                int rowMidY = row.bounds.centerY();

                for (UiNode tn : rowTexts) {
                    if (rowAvatarB64.isEmpty() && !tn.avatarB64.isEmpty() && tn.bounds.left < sw * 0.35) {
                        rowAvatarB64 = tn.avatarB64;
                    }
                    if (avatarBounds == null
                            && tn.bounds.left < sw * 0.28
                            && tn.bounds.width() >= 48
                            && tn.bounds.width() <= 220
                            && tn.bounds.height() >= 48
                            && tn.bounds.height() <= 220
                            && Math.abs(tn.bounds.width() - tn.bounds.height()) <= 32) {
                        avatarBounds = tn.bounds;
                    }
                    String val = (!tn.text.isEmpty() ? tn.text : tn.desc).trim();
                    if (val.isEmpty()) continue;
                    if (!ImAppCatalog.extractAvatarSenderName(val).isEmpty()) continue;
                    if (val.matches("^\\d{1,3}$") && tn.bounds.width() < 90) {
                        try {
                            unread = Integer.parseInt(val);
                        } catch (NumberFormatException ignored) {
                        }
                        continue;
                    }
                    if (isTimestampOrSystemTip(val) && tn.bounds.left > sw * 0.65) {
                        continue;
                    }
                    if (title.isEmpty() && tn.bounds.centerY() <= rowMidY + 15 && tn.bounds.left < sw * 0.65) {
                        title = val;
                    } else if (!val.equals(title) && tn.bounds.centerY() >= rowMidY - 15) {
                        preview = val;
                    }
                }
                if (rowAvatarB64.isEmpty()) {
                    for (UiNode an : all) {
                        if (!an.avatarB64.isEmpty()
                                && an.bounds.left < sw * 0.35
                                && an.bounds.centerY() >= row.bounds.top - 20
                                && an.bounds.centerY() <= row.bounds.bottom + 20) {
                            rowAvatarB64 = an.avatarB64;
                            break;
                        }
                    }
                }
                if (!title.isEmpty() && !preview.isEmpty()) {
                    if (rowAvatarB64.isEmpty()) {
                        if (avatarBounds == null) {
                            int side = Math.min(132, Math.max(72, (int) (row.bounds.height() * 0.65f)));
                            int left = Math.max(24, (int) (sw * 0.042f));
                            int top = Math.max(0, rowMidY - side / 2);
                            avatarBounds = new UiBounds(left, top, left + side, top + side);
                        }
                        if (!listScreenAttempted) {
                            listScreenAttempted = true;
                            listScreenBmp = GuiAccessibilityService.captureScreenBitmapSync(900L);
                        }
                        if (listScreenBmp != null) {
                            rowAvatarB64 = cropAvatarBase64FromBitmap(listScreenBmp, avatarBounds);
                        }
                    }
                    ImAppCatalog.SenderBody sb = ImAppCatalog.extractSenderAndBody(pkg, title, preview, true);
                    rows.add(new PushStore.CapturedConversationRow(title, preview, sb.sender, unread, rowAvatarB64));
                }
            }
        } finally {
            if (listScreenBmp != null && !listScreenBmp.isRecycled()) {
                listScreenBmp.recycle();
            }
        }
        return rows;
    }

    private static boolean isMatchingChatWindow(
            UiWindowSnapshot snap,
            ParsedChatWindow parsed,
            String pkg,
            String targetTitle,
            List<String> knownThreadMessages,
            List<String> participantHints) {
        if (parsed == null || parsed.isConversationList) return false;
        String cleanTarget = ImAppCatalog.cleanConversationTitle(pkg, targetTitle);

        List<String> effectiveHints = new ArrayList<String>();
        for (String h : ImAppCatalog.extractGenericGroupParticipantHints(targetTitle)) {
            if (!effectiveHints.contains(h)) effectiveHints.add(h);
        }
        if (participantHints != null) {
            for (String h : participantHints) {
                if (h != null && !h.trim().isEmpty() && !effectiveHints.contains(h.trim())) {
                    effectiveHints.add(h.trim());
                }
            }
        }

        // 1. Direct or cleaned title match
        if (!cleanTarget.isEmpty() && !ImAppCatalog.isGenericPlaceholderTitle(cleanTarget)) {
            if (matchesRecipientNodeText(pkg, cleanTarget, parsed.rawTitle)
                    || matchesRecipientNodeText(pkg, cleanTarget, parsed.cleanTitle)) {
                return true;
            }
            List<UiNode> all = new ArrayList<UiNode>();
            flattenNodes(snap.root, all);
            for (UiNode n : all) {
                if (n.bounds.bottom <= snap.screenHeight * 0.22) {
                    if (matchesRecipientNodeText(pkg, cleanTarget, n.text)
                            || matchesRecipientNodeText(pkg, cleanTarget, n.desc)) {
                        return true;
                    }
                }
            }
        }

        // 2. Generic placeholder title ("群聊" / "群聊(4)" / "群聊 · 敏" / "群聊 (窝-StepX)"):
        // Match if recent messages or participant hints appear in this chat window; only fall back to bare
        // parsed.isGroup when no disambiguating hints or recent messages exist!
        if (ImAppCatalog.isGenericPlaceholderTitle(targetTitle)
                || ImAppCatalog.isGenericPlaceholderTitle(cleanTarget)) {
            String rawHeader = parsed.rawTitle == null ? "" : parsed.rawTitle.trim();
            String cleanHeader = parsed.cleanTitle == null ? "" : parsed.cleanTitle.trim();
            for (String hint : effectiveHints) {
                if (rawHeader.contains(hint) || cleanHeader.contains(hint)) {
                    return true;
                }
            }
            if (parsed.bubbles != null && !parsed.bubbles.isEmpty()) {
                for (PushStore.CapturedBubble b : parsed.bubbles) {
                    String bText = b.text == null ? "" : b.text.trim();
                    String bSender = b.sender == null ? "" : b.sender.trim();
                    for (String hint : effectiveHints) {
                        if (bSender.equals(hint) || bSender.contains(hint) || bText.contains(hint)) {
                            return true;
                        }
                    }
                    if (!bText.isEmpty() && knownThreadMessages != null) {
                        for (String known : knownThreadMessages) {
                            if (known == null || known.isEmpty()) continue;
                            if (bText.equals(known)
                                    || (bText.startsWith("[语音]") && known.startsWith("[语音]"))) {
                                return true;
                            }
                        }
                    }
                }
            }
            if (snap != null && snap.root != null && !effectiveHints.isEmpty()) {
                List<UiNode> all = new ArrayList<UiNode>();
                flattenNodes(snap.root, all);
                for (UiNode n : all) {
                    if (n.bounds.bottom <= snap.screenHeight * 0.22) {
                        String val = (!n.text.isEmpty() ? n.text : n.desc).trim();
                        for (String hint : effectiveHints) {
                            if (val.contains(hint)) {
                                return true;
                            }
                        }
                    }
                }
            }
            boolean hasDisambiguatingEvidence = !effectiveHints.isEmpty()
                || (knownThreadMessages != null && !knownThreadMessages.isEmpty());
            if (!hasDisambiguatingEvidence) {
                if (parsed.isGroup
                        || ImAppCatalog.hasGroupMemberCountSuffix(rawHeader)
                        || "群聊".equals(cleanHeader)
                        || rawHeader.startsWith("群聊")
                        || rawHeader.startsWith("(")
                        || rawHeader.startsWith("（")) {
                    return true;
                }
                if (snap != null && snap.root != null) {
                    List<UiNode> all = new ArrayList<UiNode>();
                    flattenNodes(snap.root, all);
                    for (UiNode n : all) {
                        if (n.bounds.bottom <= snap.screenHeight * 0.22) {
                            String val = (!n.text.isEmpty() ? n.text : n.desc).trim();
                            if (val.startsWith("群聊") || val.startsWith("(") || val.startsWith("（")) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        return false;
    }

    private static UiNode findFirstBottomChatTabNode(UiNode root, int screenWidth, int screenHeight) {
        if (root == null) return null;
        int sw = screenWidth > 0 ? screenWidth : 1080;
        int sh = screenHeight > 0 ? screenHeight : 2400;
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);
        for (UiNode n : all) {
            if (n.bounds.top < sh * 0.80 || n.bounds.left > sw * 0.28) continue;
            String t = (!n.text.isEmpty() ? n.text : n.desc).trim();
            if ("微信".equals(t) || t.startsWith("微信(") || "消息".equals(t) || t.startsWith("消息(")) {
                return n;
            }
        }
        for (UiNode n : all) {
            if (n.clickable && n.bounds.top >= sh * 0.85 && n.bounds.left <= sw * 0.28 && n.bounds.right <= sw * 0.35) {
                return n;
            }
        }
        return null;
    }

    private static UiNode findConversationRowOnList(
            UiNode root,
            String pkg,
            String targetTitle,
            List<String> knownThreadMessages,
            List<String> participantHints,
            int screenHeight) {
        if (root == null) return null;
        String cleanTarget = ImAppCatalog.cleanConversationTitle(pkg, targetTitle);
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);

        List<String> effectiveHints = new ArrayList<String>();
        for (String h : ImAppCatalog.extractGenericGroupParticipantHints(targetTitle)) {
            if (!effectiveHints.contains(h)) effectiveHints.add(h);
        }
        if (participantHints != null) {
            for (String h : participantHints) {
                if (h != null && !h.trim().isEmpty() && !effectiveHints.contains(h.trim())) {
                    effectiveHints.add(h.trim());
                }
            }
        }

        // 1. If targetTitle is a specific non-generic name, match by title first
        if (!cleanTarget.isEmpty() && !ImAppCatalog.isGenericPlaceholderTitle(cleanTarget)) {
            for (UiNode n : all) {
                if (n.editable) continue;
                if (n.bounds.top < screenHeight * 0.10 || n.bounds.bottom > screenHeight * 0.90) continue;
                if (matchesRecipientNodeText(pkg, cleanTarget, n.text)
                        || matchesRecipientNodeText(pkg, cleanTarget, n.desc)) {
                    return n;
                }
            }
        }

        // 2. Match by overlapping recent message body in the conversation list row's preview snippet!
        if (knownThreadMessages != null && !knownThreadMessages.isEmpty()) {
            for (String msgText : knownThreadMessages) {
                if (msgText == null || msgText.isEmpty()) continue;
                for (UiNode n : all) {
                    if (n.editable) continue;
                    if (n.bounds.top < screenHeight * 0.10 || n.bounds.bottom > screenHeight * 0.90) continue;
                    String val = (!n.text.isEmpty() ? n.text : n.desc).trim();
                    if (val.isEmpty()) continue;
                    if (val.equals(msgText)
                            || val.endsWith(": " + msgText)
                            || val.endsWith("：" + msgText)
                            || val.contains(": " + msgText)
                            || val.contains("：" + msgText)
                            || (msgText.startsWith("[语音]") && val.contains("[语音]"))
                            || (msgText.length() >= 4 && val.contains(msgText))) {
                        return n;
                    }
                }
            }
        }

        // 3. Match by participant hints or generic "群聊" row when targetTitle is generic
        if (ImAppCatalog.isGenericPlaceholderTitle(targetTitle)
                || ImAppCatalog.isGenericPlaceholderTitle(cleanTarget)) {
            if (!effectiveHints.isEmpty()) {
                // First prefer rows that have group indicators (e.g. "hint: ..." or "群聊..." or member count)
                for (String hint : effectiveHints) {
                    for (UiNode n : all) {
                        if (n.editable) continue;
                        if (n.bounds.top < screenHeight * 0.10 || n.bounds.bottom > screenHeight * 0.90) continue;
                        String val = (!n.text.isEmpty() ? n.text : n.desc).trim();
                        if (val.isEmpty()) continue;
                        if (val.startsWith(hint + ":")
                                || val.startsWith(hint + "：")
                                || val.contains(hint + ":")
                                || val.contains(hint + "：")
                                || ((val.startsWith("群聊") || ImAppCatalog.hasGroupMemberCountSuffix(val))
                                    && val.contains(hint))) {
                            return n;
                        }
                    }
                }
                // Then fall back to plain hint containment
                for (String hint : effectiveHints) {
                    for (UiNode n : all) {
                        if (n.editable) continue;
                        if (n.bounds.top < screenHeight * 0.10 || n.bounds.bottom > screenHeight * 0.90) continue;
                        String val = (!n.text.isEmpty() ? n.text : n.desc).trim();
                        if (val.isEmpty()) continue;
                        if (val.equals(hint) || val.contains(hint)) {
                            return n;
                        }
                    }
                }
            }
            for (UiNode n : all) {
                if (n.editable) continue;
                if (n.bounds.top < screenHeight * 0.10 || n.bounds.bottom > screenHeight * 0.90) continue;
                String val = (!n.text.isEmpty() ? n.text : n.desc).trim();
                if (val.isEmpty()) continue;
                if ("群聊".equals(val)
                        || val.startsWith("群聊")
                        || ImAppCatalog.hasGroupMemberCountSuffix(val)) {
                    return n;
                }
            }
        }

        // 4. Fallback: match literal cleanTarget
        if (!cleanTarget.isEmpty()) {
            for (UiNode n : all) {
                if (n.editable) continue;
                if (n.bounds.top < screenHeight * 0.10 || n.bounds.bottom > screenHeight * 0.90) continue;
                if (matchesRecipientNodeText(pkg, cleanTarget, n.text)
                        || matchesRecipientNodeText(pkg, cleanTarget, n.desc)) {
                    return n;
                }
            }
        }
        return null;
    }

    static boolean matchesRecipientNodeText(String pkg, String cleanTargetTitle, String candidateText) {
        if (cleanTargetTitle == null || cleanTargetTitle.isEmpty() || candidateText == null) {
            return false;
        }
        String raw = candidateText.trim();
        if (raw.isEmpty()) return false;
        if (raw.equalsIgnoreCase(cleanTargetTitle)) return true;

        String cleanedCandidate = ImAppCatalog.cleanConversationTitle(pkg, raw);
        if (!cleanedCandidate.isEmpty() && cleanedCandidate.equalsIgnoreCase(cleanTargetTitle)) {
            return true;
        }
        if ((cleanTargetTitle.indexOf('@') >= 0 || cleanTargetTitle.indexOf('＠') >= 0
                || raw.indexOf('@') >= 0 || raw.indexOf('＠') >= 0
                || cleanTargetTitle.endsWith("…") || raw.endsWith("…"))
                && PushStore.titlesReferToSameConversation(pkg, cleanTargetTitle, raw)) {
            return true;
        }
        if (raw.startsWith(cleanTargetTitle + "(") || raw.startsWith(cleanTargetTitle + "（")
                || raw.startsWith(cleanTargetTitle + " (")) {
            return true;
        }
        return false;
    }

    private static UiNode findChatInputNode(UiNode root, int screenHeight) {
        if (root == null) return null;
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);
        for (UiNode n : all) {
            if (n.editable || n.className.contains("EditText")) {
                if (n.bounds.centerY() > screenHeight * 0.45) {
                    return n;
                }
            }
        }
        return null;
    }

    private static UiNode findVoiceToKeyboardSwitchNode(UiNode root, int screenHeight) {
        if (root == null) return null;
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);
        boolean hasHoldToTalk = false;
        for (UiNode n : all) {
            if (n.bounds.top > screenHeight * 0.70) {
                String t = (!n.text.isEmpty() ? n.text : n.desc).trim();
                if ("切换到键盘".equals(t) || t.contains("切换到键盘")
                        || t.toLowerCase(Locale.ROOT).contains("switch to keyboard")) {
                    return n;
                }
                if (t.contains("按住 说话") || t.contains("按住说话")
                        || t.toLowerCase(Locale.ROOT).contains("hold to talk")) {
                    hasHoldToTalk = true;
                }
            }
        }
        if (hasHoldToTalk) {
            for (UiNode n : all) {
                if (n.clickable && n.bounds.top > screenHeight * 0.85 && n.bounds.right < 220) {
                    return n;
                }
            }
        }
        return null;
    }

    private static UiNode findSendButtonNode(UiNode root, int screenHeight) {
        if (root == null) return null;
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);
        for (UiNode n : all) {
            if (n.editable) continue;
            if (n.bounds.top < screenHeight * 0.35) continue;
            String t = n.text.trim();
            String d = n.desc.trim();
            if ("发送".equals(t) || "Send".equalsIgnoreCase(t)
                    || "发送".equals(d) || "Send".equalsIgnoreCase(d)
                    || t.startsWith("发送(")) {
                return n;
            }
        }
        return null;
    }

    private static boolean isMainTabListScreen(UiNode root, int screenHeight) {
        if (root == null) return false;
        List<UiNode> all = new ArrayList<UiNode>();
        flattenNodes(root, all);
        int bottomTabs = 0;
        for (UiNode n : all) {
            if (n.bounds.top < screenHeight * 0.80) continue;
            String t = (!n.text.isEmpty() ? n.text : n.desc).trim();
            if ("微信".equals(t) || "通讯录".equals(t) || "发现".equals(t) || "我".equals(t)
                    || "消息".equals(t) || "联系人".equals(t) || "动态".equals(t)) {
                bottomTabs++;
            }
        }
        return bottomTabs >= 2;
    }

    private static void flattenNodes(UiNode node, List<UiNode> out) {
        if (node == null) return;
        out.add(node);
        for (UiNode child : node.children) {
            flattenNodes(child, out);
        }
    }

    private static boolean hasChildWithSameText(UiNode node, String val) {
        for (UiNode c : node.children) {
            if (val.equals(c.text) || val.equals(c.desc) || hasChildWithSameText(c, val)) {
                return true;
            }
        }
        return false;
    }

    private static boolean isSystemStatusBarNoise(String s) {
        if (s == null || s.isEmpty()) return true;
        if (s.matches("^\\d{1,2}:\\d{2}$")) return true;
        if (s.matches("^\\d{1,3}%$")) return true;
        if (s.matches("^[0-9.]+\\s*(KB/s|MB/s|B/s|K/s|M/s)$")) return true;
        return "5G".equalsIgnoreCase(s) || "4G".equalsIgnoreCase(s)
            || "LTE".equalsIgnoreCase(s) || "HD".equalsIgnoreCase(s)
            || "Wi-Fi".equalsIgnoreCase(s) || "WLAN".equalsIgnoreCase(s);
    }

    private static boolean isTopBarChromeLabel(String s) {
        return "返回".equals(s) || "Back".equalsIgnoreCase(s)
            || "更多".equals(s) || "更多信息".equals(s) || "聊天信息".equals(s)
            || "...".equals(s) || "…".equals(s)
            || "关闭".equals(s) || "搜索".equals(s) || "添加".equals(s);
    }

    private static boolean isChatInputChromeLabel(String s) {
        return "发送".equals(s) || "Send".equalsIgnoreCase(s)
            || "按住 说话".equals(s) || "按住说话".equals(s)
            || "切换到键盘".equals(s) || "切换到语音".equals(s)
            || "表情".equals(s) || "更多功能按钮，已折叠".equals(s)
            || "更多功能按钮".equals(s) || "切换到按住说话".equals(s)
            || "转文字".equals(s) || "重发".equals(s)
            || "撤回".equals(s) || "引用".equals(s)
            || "拍一拍".equals(s) || "多选".equals(s)
            || "转发截图".equals(s);
    }

    private static boolean isTimestampOrSystemTip(String s) {
        if (s == null) return false;
        String t = s.trim();
        if (t.matches("^\\d{1,2}:\\d{2}$")) return true;
        if (t.matches("^(昨天|前天|今天|星期[一二三四五六日天]|周[一二三四五六日])(\\s+\\d{1,2}:\\d{2})?$")) return true;
        if (t.matches("^(上午|下午|晚上|凌晨|早上)\\s*\\d{1,2}:\\d{2}$")) return true;
        if (t.matches("^\\d{1,2}月\\d{1,2}日(\\s+\\d{1,2}:\\d{2})?$")) return true;
        return "以下为新消息".equals(t) || "以下是新消息".equals(t);
    }

    private static boolean clickAccessibilityNodeOrParent(AccessibilityNodeInfo node) {
        AccessibilityNodeInfo curr = node;
        for (int depth = 0; depth < 6 && curr != null; depth++) {
            if (curr.isClickable()) {
                return curr.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            }
            curr = curr.getParent();
        }
        return false;
    }

    private static boolean dispatchAccessibilityTap(GuiAccessibilityService service, int x, int y) {
        if (x < 0 || y < 0) return false;
        try {
            Path path = new Path();
            path.moveTo(x, y);
            GestureDescription.StrokeDescription stroke =
                new GestureDescription.StrokeDescription(path, 0L, 60L);
            GestureDescription gesture = new GestureDescription.Builder().addStroke(stroke).build();
            final CountDownLatch latch = new CountDownLatch(1);
            final boolean[] ok = new boolean[]{false};
            boolean dispatched = service.dispatchGesture(gesture, new AccessibilityService.GestureResultCallback() {
                @Override
                public void onCompleted(GestureDescription gestureDescription) {
                    ok[0] = true;
                    latch.countDown();
                }

                @Override
                public void onCancelled(GestureDescription gestureDescription) {
                    latch.countDown();
                }
            }, null);
            if (!dispatched) return false;
            latch.await(1500L, TimeUnit.MILLISECONDS);
            return ok[0];
        } catch (Throwable ignored) {
            return false;
        }
    }

    private static String safeStr(Object o) {
        return o == null ? "" : o.toString().trim();
    }
}
