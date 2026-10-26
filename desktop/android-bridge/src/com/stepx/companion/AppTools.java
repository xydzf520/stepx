package com.stepx.companion;

import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.ResolveInfo;
import android.content.res.Resources;
import android.content.res.XmlResourceParser;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.ColorFilter;
import android.graphics.Path;
import android.graphics.PixelFormat;
import android.graphics.Rect;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.LayerDrawable;
import android.util.Base64;
import android.util.TypedValue;
import android.util.Xml;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.InputStream;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/** Package metadata comes from the current Android user's PackageManager. */
public final class AppTools {
    private static final int ICON_SIZE = 96;
    private static final int ICON_LIMIT = 48 * 1024;
    private static final int ICON_TOTAL_LIMIT = 8 * 1024 * 1024;

    static int currentUser() throws Exception {
        return (Integer) Class.forName("android.app.ActivityManager")
            .getMethod("getCurrentUser").invoke(null);
    }

    @SuppressWarnings("unchecked")
    static Set<String> launcherPackages(PackageManager pm, int user) throws Exception {
        Intent intent = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER);
        List<ResolveInfo> resolved = (List<ResolveInfo>) PackageManager.class
            .getMethod("queryIntentActivitiesAsUser", Intent.class, int.class, int.class)
            .invoke(pm, intent, PackageManager.MATCH_DISABLED_COMPONENTS, user);
        Set<String> packages = new HashSet<>();
        for (ResolveInfo item : resolved) {
            if (item.activityInfo != null) packages.add(item.activityInfo.packageName);
        }
        return packages;
    }

    @SuppressWarnings("unchecked")
    static boolean isLauncherPackage(PackageManager pm, int user, String packageName) throws Exception {
        Intent intent = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER).setPackage(packageName);
        List<ResolveInfo> resolved = (List<ResolveInfo>) PackageManager.class
            .getMethod("queryIntentActivitiesAsUser", Intent.class, int.class, int.class)
            .invoke(pm, intent, PackageManager.MATCH_DISABLED_COMPONENTS, user);
        if (resolved != null) {
            for (ResolveInfo item : resolved) {
                if (item.activityInfo != null && packageName.equals(item.activityInfo.packageName)) return true;
            }
        }
        return false;
    }

    @SuppressWarnings("unchecked")
    public static JSONObject list(JSONObject request) throws Exception {
        PackageManager pm = Bridge.context.getPackageManager();
        int user = currentUser();
        List<PackageInfo> packages = (List<PackageInfo>) PackageManager.class
            .getMethod("getInstalledPackagesAsUser", int.class, int.class)
            .invoke(pm, PackageManager.MATCH_DISABLED_COMPONENTS, user);
        if (packages.size() > 4096) throw new IllegalStateException("应用数量超出读取上限");
        Set<String> launchers = launcherPackages(pm, user);
        JSONArray result = new JSONArray();
        int iconBytes = 0;
        for (PackageInfo item : packages) {
            if (item.applicationInfo == null) continue;
            JSONObject row = metadata(pm, item, user, launchers.contains(item.packageName));
            if (request.optBoolean("icons", true)) {
                try {
                    byte[] png = icon(pm, item.applicationInfo);
                    if (png.length > ICON_LIMIT || iconBytes + png.length > ICON_TOTAL_LIMIT) {
                        row.put("icon_error", "图标超出读取上限");
                    } else {
                        iconBytes += png.length;
                        row.put("icon_png", Base64.encodeToString(png, Base64.NO_WRAP));
                    }
                } catch (Exception error) {
                    row.put("icon_error", "手机未能读取此应用图标：" + error.getClass().getSimpleName()
                        + (error.getMessage() == null ? "" : " · " + error.getMessage()));
                }
            }
            result.put(row);
        }
        return new JSONObject().put("user_id", user).put("apps", result);
    }

    public static JSONObject info(JSONObject request) throws Exception {
        String name = request.getString("package");
        if (name.length() > 255 || !name.matches("[A-Za-z0-9_]+(?:\\.[A-Za-z0-9_]+)*")) {
            throw new IllegalArgumentException("应用包名无效");
        }
        PackageManager pm = Bridge.context.getPackageManager();
        int user = currentUser();
        PackageInfo item = (PackageInfo) PackageManager.class
            .getMethod("getPackageInfoAsUser", String.class, int.class, int.class)
            .invoke(pm, name, PackageManager.MATCH_DISABLED_COMPONENTS, user);
        return new JSONObject().put("app", metadata(pm, item, user, isLauncherPackage(pm, user, name)));
    }

    static JSONObject metadata(PackageManager pm, PackageInfo item, int user, boolean launcher) throws Exception {
        ApplicationInfo app = item.applicationInfo;
        String label = item.packageName;
        try {
            CharSequence loaded = app.loadLabel(pm);
            if (loaded != null && loaded.length() > 0) label = loaded.toString();
        } catch (Exception ignored) {}
        JSONArray apks = new JSONArray();
        addApk(apks, app.sourceDir);
        if (app.splitSourceDirs != null) {
            for (String path : app.splitSourceDirs) addApk(apks, path);
        }
        return new JSONObject()
            .put("package", item.packageName).put("label", label)
            .put("version_name", item.versionName == null ? "" : item.versionName)
            .put("version_code", item.getLongVersionCode())
            .put("last_update_ms", item.lastUpdateTime).put("user_id", user)
            .put("system", (app.flags & ApplicationInfo.FLAG_SYSTEM) != 0)
            .put("updated_system", (app.flags & ApplicationInfo.FLAG_UPDATED_SYSTEM_APP) != 0)
            .put("enabled", app.enabled).put("launchable", launcher)
            .put("apk_files", apks);
    }

    static void addApk(JSONArray apks, String path) throws Exception {
        if (path == null || path.isEmpty()) return;
        File file = new File(path);
        apks.put(new JSONObject().put("path", path).put("bytes", file.length()));
    }

    static byte[] icon(PackageManager pm, ApplicationInfo app) throws Exception {
        // StepOS wraps PackageManager.loadIcon in its launcher theme service,
        // which assumes a bound ApplicationThread. app_process has no AppBindData;
        // load the installed APK's own resource directly instead of that wrapper.
        Resources resources = pm.getResourcesForApplication(app);
        int resourceId = app.icon != 0 ? app.icon
            : Resources.getSystem().getIdentifier("sym_def_app_icon", "drawable", "android");
        Drawable drawable = rawDrawable(resources, resourceId, 0);
        if (drawable == null) throw new IllegalStateException("图标不可用");
        Bitmap bitmap = Bitmap.createBitmap(ICON_SIZE, ICON_SIZE, Bitmap.Config.ARGB_8888);
        try {
            drawable.setBounds(0, 0, ICON_SIZE, ICON_SIZE);
            drawable.draw(new Canvas(bitmap));
            boolean visible = false;
            boolean nonWhite = false;
            boolean needNonWhite = "com.android.calendar".equals(app.packageName);
            int[] pixels = new int[ICON_SIZE * ICON_SIZE];
            bitmap.getPixels(pixels, 0, ICON_SIZE, 0, 0, ICON_SIZE, ICON_SIZE);
            for (int pixel : pixels) {
                if ((pixel >>> 24) != 0) visible = true;
                if ((pixel >>> 24) > 64 && ((pixel >> 16 & 255) < 230
                    || (pixel >> 8 & 255) < 230 || (pixel & 255) < 230)) nonWhite = true;
                if (visible && (nonWhite || !needNonWhite)) break;
            }
            if (!visible) throw new IllegalStateException("应用图标内容为空");
            if (needNonWhite && !nonWhite) {
                throw new IllegalStateException("手机日历使用主题动态图标，安装包只提供空白底图");
            }
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, bytes)) {
                throw new IllegalStateException("图标转换失败");
            }
            return bytes.toByteArray();
        } finally {
            bitmap.recycle();
        }
    }

    static Drawable rawDrawable(Resources resources, int id, int depth) throws Exception {
        if (depth > 8) throw new IllegalStateException("图标资源嵌套过深");
        TypedValue value = new TypedValue();
        resources.getValue(id, value, true);
        if (value.type >= TypedValue.TYPE_FIRST_COLOR_INT && value.type <= TypedValue.TYPE_LAST_COLOR_INT) {
            return new ColorDrawable(value.data);
        }
        if (value.string != null && value.string.toString().endsWith(".xml")) {
            try (XmlResourceParser parser = resources.getXml(id)) {
                int event;
                do { event = parser.next(); } while (event != XmlResourceParser.START_TAG && event != XmlResourceParser.END_DOCUMENT);
                if (!"adaptive-icon".equals(parser.getName())) {
                    return Drawable.createFromXmlInner(resources, parser, Xml.asAttributeSet(parser));
                }
                // OEM AdaptiveIconDrawable asks a theme ContentProvider for its mask.
                // Render the APK's actual foreground/background with a desktop mask
                // instead, avoiding any registered app or foreground-phone requirement.
                Drawable background = new ColorDrawable(android.graphics.Color.TRANSPARENT);
                Drawable foreground = new ColorDrawable(android.graphics.Color.TRANSPARENT);
                int outerDepth = parser.getDepth();
                while ((event = parser.next()) != XmlResourceParser.END_DOCUMENT) {
                    if (event == XmlResourceParser.END_TAG && parser.getDepth() <= outerDepth) break;
                    if (event != XmlResourceParser.START_TAG || parser.getDepth() != outerDepth + 1) continue;
                    String layer = parser.getName();
                    if (!"background".equals(layer) && !"foreground".equals(layer)) continue;
                    int resource = 0;
                    Integer color = null;
                    // Compiled/optimized XML may omit namespace/name strings.
                    // Android itself identifies styleable attributes by resource ID.
                    for (int index = 0; index < parser.getAttributeCount(); index++) {
                        if (parser.getAttributeNameResource(index) != android.R.attr.drawable) continue;
                        resource = parser.getAttributeResourceValue(index, 0);
                        String literal = parser.getAttributeValue(index);
                        if (resource == 0 && literal != null && literal.startsWith("#")) {
                            color = android.graphics.Color.parseColor(literal);
                        }
                        break;
                    }
                    Drawable item;
                    if (resource != 0) {
                        item = rawDrawable(resources, resource, depth + 1);
                    } else if (color != null) {
                        item = new ColorDrawable(color);
                    } else {
                        do { event = parser.next(); } while (event == XmlResourceParser.TEXT);
                        if (event != XmlResourceParser.START_TAG) continue;
                        item = Drawable.createFromXmlInner(resources, parser, Xml.asAttributeSet(parser));
                    }
                    if ("background".equals(layer)) background = item; else foreground = item;
                }
                return new DesktopAdaptiveIcon(background, foreground);
            }
        }
        try (InputStream stream = resources.openRawResource(id)) {
            return Drawable.createFromResourceStream(resources, value, stream, null);
        }
    }

    static final class DesktopAdaptiveIcon extends Drawable {
        private final LayerDrawable layers;
        DesktopAdaptiveIcon(Drawable background, Drawable foreground) {
            layers = new LayerDrawable(new Drawable[]{background, foreground});
        }
        @Override protected void onBoundsChange(Rect bounds) {
            int extraX = bounds.width() / 4;
            int extraY = bounds.height() / 4;
            layers.setBounds(bounds.left - extraX, bounds.top - extraY,
                bounds.right + extraX, bounds.bottom + extraY);
        }
        @Override public void draw(Canvas canvas) {
            Rect bounds = getBounds();
            Path mask = new Path();
            float radius = Math.min(bounds.width(), bounds.height()) * .22f;
            mask.addRoundRect(bounds.left, bounds.top, bounds.right, bounds.bottom, radius, radius, Path.Direction.CW);
            int checkpoint = canvas.save();
            canvas.clipPath(mask);
            layers.draw(canvas);
            canvas.restoreToCount(checkpoint);
        }
        @Override public void setAlpha(int alpha) { layers.setAlpha(alpha); }
        @Override public void setColorFilter(ColorFilter filter) { layers.setColorFilter(filter); }
        @Override public int getOpacity() { return PixelFormat.TRANSLUCENT; }
    }
}
