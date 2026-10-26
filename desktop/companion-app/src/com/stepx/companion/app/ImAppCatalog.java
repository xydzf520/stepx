package com.stepx.companion.app;

import android.content.Context;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Mainstream IM & Push Notification catalog, title normalizer, sender/body splitter,
 * 6-category classifier, OTP/pickup-code extractor, and URL extractor.
 */
public final class ImAppCatalog {
    public static final String CAT_IM = "im";
    public static final String CAT_SOCIAL = "social";
    public static final String CAT_FINANCE = "finance";
    public static final String CAT_DELIVERY_TRAVEL = "delivery_travel";
    public static final String CAT_SYSTEM = "system";
    public static final String CAT_OTHER = "other";

    private static final Map<String, String> DISPLAY_NAMES;
    private static final ConcurrentHashMap<String, String> RESOLVED_APP_LABELS = new ConcurrentHashMap<String, String>();
    private static final Set<String> CORE_IM_PACKAGES;
    private static final Set<String> SOCIAL_PACKAGES;
    private static final Set<String> FINANCE_PACKAGES;
    private static final Set<String> DELIVERY_TRAVEL_PACKAGES;
    private static final Set<String> SYSTEM_PACKAGES;

    // Leading unread prefixes: "[3条] ", "[12条通知] ", "[4条消息] ", "【5条】", "(3条) ", "[99+条] "
    private static final Pattern LEADING_UNREAD_PATTERN = Pattern.compile(
        "^\\s*(?:\\[[0-9]+\\+?\\s*(?:条(?:新?消息|通知)?|个?(?:未读|联系人)|messages?|new)?\\]|"
        + "【[0-9]+\\+?\\s*条(?:新?消息|通知)?】|"
        + "[（(][0-9]+\\+?\\s*条(?:新?消息|通知)?[)）])\\s*"
    );

    // Trailing unread suffixes: " (3)", " (3条)", " (5条新消息)", "（3条）", " [4]"
    private static final Pattern TRAILING_UNREAD_PATTERN = Pattern.compile(
        "\\s*(?:\\([0-9]+\\+?\\s*(?:条(?:新?消息|未读)?|未读|messages?|new(?:\\s+messages?)?)?\\)|"
        + "（[0-9]+\\+?\\s*(?:条(?:新?消息|未读)?|未读)?）|"
        + "\\[[0-9]+\\+?\\s*(?:条(?:新?消息|未读)?)?\\])\\s*$"
    );

    // Group sender prefix: "王敏: 今晚8点发版" or "李四：方案发我一下"
    private static final Pattern SENDER_SPLIT_PATTERN = Pattern.compile(
        "^([^:：\\r\\n]{1,28}?)[\\s]*[:：][\\s]*(.+)$",
        Pattern.DOTALL
    );

    // OTP trigger keywords
    private static final Pattern OTP_TRIGGER_PATTERN = Pattern.compile(
        "(?:验证码|校验码|动态码|确认码|取件码|提货码|认证码|安全码|口令|"
        + "verification|verify|passcode|otp|\\bcode\\b|\\bpin\\b)",
        Pattern.CASE_INSENSITIVE
    );

    // Pickup code trigger keywords
    private static final Pattern PICKUP_TRIGGER_PATTERN = Pattern.compile(
        "(?:取件码|提货码|取货码|丰巢|菜鸟驿站|快递柜)"
    );

    // Cabinet pickup code e.g. "4-2-1089", "A-12-3048", "12-3-8821"
    private static final Pattern PICKUP_HYPHEN_CODE_PATTERN = Pattern.compile(
        "(?<![A-Za-z0-9-])([A-Za-z0-9]{1,4}-[0-9]{1,4}-[0-9]{2,6})(?![A-Za-z0-9-])"
    );

    // Direct keyword-to-digits pattern: "验证码 839201" / "验证码为：482910" / "code is 392014"
    private static final Pattern OTP_DIRECT_AFTER_KEYWORD = Pattern.compile(
        "(?:验证码|校验码|动态码|确认码|取件码|提货码|认证码|安全码|"
        + "verification\\s*code|verify\\s*code|passcode|otp|\\bcode\\b)"
        + "(?:\\s*(?:为|是|is)?\\s*[:：]?\\s*[【\\[\"「『（(]?)([0-9]{4,8})(?![0-9])",
        Pattern.CASE_INSENSITIVE
    );

    // Direct digits-before-keyword pattern: "839201（登录验证码）" / "492018 is your verification code"
    private static final Pattern OTP_DIRECT_BEFORE_KEYWORD = Pattern.compile(
        "(?<![0-9.])([0-9]{4,8})[】\\]\"」』）)]?\\s*(?:是您的|为您的|is\\s+your|（|[(]|\\[)?\\s*"
        + "(?:登录|动态|短信|手机|安全)?(?:验证码|校验码|动态码|确认码|verification\\s*code|otp)",
        Pattern.CASE_INSENSITIVE
    );

    // Generic 4-8 digit standalone token
    private static final Pattern STANDALONE_DIGITS_PATTERN = Pattern.compile(
        "(?<![0-9A-Za-z._-])([0-9]{4,8})(?![0-9A-Za-z._-])"
    );

    // First URL extractor
    private static final Pattern URL_PATTERN = Pattern.compile(
        "(https?://[^\\s\"<>\\u3000\\uFF01-\\uFF5E\\u3001-\\u303F]+)"
    );

    static {
        Map<String, String> names = new HashMap<String, String>();
        names.put("com.tencent.mm", "微信");
        names.put("com.tencent.wework", "企业微信");
        names.put("com.alibaba.android.rimet", "钉钉");
        names.put("com.ss.android.lark", "飞书");
        names.put("com.larksuite.suite", "飞书");
        names.put("com.tencent.mobileqq", "QQ");
        names.put("com.tencent.tim", "TIM");
        names.put("org.telegram.messenger", "Telegram");
        names.put("org.telegram.messenger.web", "Telegram");
        names.put("org.thunderdog.challegram", "Telegram X");
        names.put("tw.nekomimi.nekogram", "Nekogram");
        names.put("com.whatsapp", "WhatsApp");
        names.put("com.whatsapp.w4b", "WhatsApp Business");
        names.put("com.Slack", "Slack");
        names.put("com.discord", "Discord");
        names.put("com.microsoft.teams", "Teams");
        names.put("org.thoughtcrime.securesms", "Signal");
        names.put("com.facebook.orca", "Messenger");
        names.put("com.xingin.xhs", "小红书");
        names.put("com.ss.android.ugc.aweme", "抖音");
        names.put("com.taobao.idlefish", "闲鱼");
        names.put("com.taobao.taobao", "淘宝");
        names.put("com.jingdong.app.mall", "京东");
        names.put("com.xunmeng.pinduoduo", "拼多多");
        names.put("com.android.mms", "短信");
        names.put("com.google.android.apps.messaging", "短信");
        names.put("com.eg.android.AlipayGphone", "支付宝");
        names.put("com.cainiao.wireless", "菜鸟");
        names.put("com.Cainiao.wireless", "菜鸟");
        names.put("com.sf.activity", "顺丰速运");
        names.put("com.sankuai.meituan", "美团");
        names.put("me.ele", "饿了么");
        names.put("com.sdu.didi.psnger", "滴滴出行");
        names.put("com.autonavi.minimap", "高德地图");
        names.put("com.MobileTicket", "铁路12306");
        names.put("com.umetrip.android.msky.app", "航旅纵横");
        names.put("com.sina.weibo", "微博");
        names.put("tv.danmaku.bili", "哔哩哔哩");
        names.put("com.zhihu.android", "知乎");
        names.put("com.github.android", "GitHub");
        names.put("cmb.pb", "招商银行");
        names.put("com.unionpay", "云闪付");
        DISPLAY_NAMES = Collections.unmodifiableMap(names);

        Set<String> im = new HashSet<String>();
        im.add("com.tencent.mm");
        im.add("com.tencent.wework");
        im.add("com.alibaba.android.rimet");
        im.add("com.ss.android.lark");
        im.add("com.larksuite.suite");
        im.add("com.tencent.mobileqq");
        im.add("com.tencent.tim");
        im.add("org.telegram.messenger");
        im.add("org.telegram.messenger.web");
        im.add("org.thunderdog.challegram");
        im.add("tw.nekomimi.nekogram");
        im.add("com.whatsapp");
        im.add("com.whatsapp.w4b");
        im.add("com.Slack");
        im.add("com.discord");
        im.add("com.microsoft.teams");
        im.add("org.thoughtcrime.securesms");
        im.add("com.facebook.orca");
        im.add("com.android.mms");
        im.add("com.google.android.apps.messaging");
        CORE_IM_PACKAGES = Collections.unmodifiableSet(im);

        Set<String> social = new HashSet<String>();
        social.add("com.xingin.xhs");
        social.add("com.ss.android.ugc.aweme");
        social.add("com.taobao.idlefish");
        social.add("com.sina.weibo");
        social.add("tv.danmaku.bili");
        social.add("com.zhihu.android");
        social.add("com.github.android");
        SOCIAL_PACKAGES = Collections.unmodifiableSet(social);

        Set<String> finance = new HashSet<String>();
        finance.add("com.eg.android.AlipayGphone");
        finance.add("cmb.pb");
        finance.add("com.unionpay");
        finance.add("com.icbc");
        finance.add("com.chinamworld.main");
        finance.add("com.android.bankabc");
        finance.add("com.chinamworld.bocmbci");
        FINANCE_PACKAGES = Collections.unmodifiableSet(finance);

        Set<String> delivery = new HashSet<String>();
        delivery.add("com.cainiao.wireless");
        delivery.add("com.Cainiao.wireless");
        delivery.add("com.sf.activity");
        delivery.add("com.sankuai.meituan");
        delivery.add("me.ele");
        delivery.add("com.sdu.didi.psnger");
        delivery.add("com.autonavi.minimap");
        delivery.add("com.MobileTicket");
        delivery.add("com.umetrip.android.msky.app");
        delivery.add("com.taobao.taobao");
        delivery.add("com.jingdong.app.mall");
        delivery.add("com.xunmeng.pinduoduo");
        DELIVERY_TRAVEL_PACKAGES = Collections.unmodifiableSet(delivery);

        Set<String> sys = new HashSet<String>();
        sys.add("android");
        sys.add("com.android.systemui");
        sys.add("com.android.settings");
        sys.add("com.stepx.companion.app");
        sys.add("com.miui.securitycenter");
        sys.add("com.coloros.phonemanager");
        SYSTEM_PACKAGES = Collections.unmodifiableSet(sys);
    }

    private ImAppCatalog() {}

    public static final class SenderBody {
        public final String sender;
        public final String body;

        public SenderBody(String sender, String body) {
            this.sender = sender == null ? "" : sender;
            this.body = body == null ? "" : body;
        }

        public String get(int index) {
            return index == 0 ? sender : body;
        }

        public String[] toArray() {
            return new String[] { sender, body };
        }
    }

    public static boolean isImPackage(String pkg) {
        return pkg != null && CORE_IM_PACKAGES.contains(pkg);
    }

    public static boolean isSocialOrCommerceImPackage(String pkg) {
        if (pkg == null) return false;
        return CORE_IM_PACKAGES.contains(pkg)
            || "com.xingin.xhs".equals(pkg)
            || "com.ss.android.ugc.aweme".equals(pkg)
            || "com.taobao.idlefish".equals(pkg)
            || "com.taobao.taobao".equals(pkg);
    }

    public static String appDisplayName(Context context, String pkg) {
        if (pkg == null || pkg.isEmpty()) return "未知应用";
        String builtIn = DISPLAY_NAMES.get(pkg);
        if (builtIn != null) return builtIn;
        String cached = RESOLVED_APP_LABELS.get(pkg);
        if (cached != null) return cached;
        int dot = pkg.lastIndexOf('.');
        String fallback = dot >= 0 && dot + 1 < pkg.length() ? pkg.substring(dot + 1) : pkg;
        if (context != null) {
            try {
                PackageManager pm = context.getPackageManager();
                if (pm != null) {
                    ApplicationInfo info = pm.getApplicationInfo(pkg, 0);
                    CharSequence label = pm.getApplicationLabel(info);
                    if (label != null && label.length() > 0) {
                        String resolved = label.toString().trim();
                        if (!resolved.isEmpty()) {
                            RESOLVED_APP_LABELS.put(pkg, resolved);
                            return resolved;
                        }
                    }
                }
            } catch (Exception ignored) {
            }
            RESOLVED_APP_LABELS.put(pkg, fallback);
        }
        return fallback;
    }

    public static boolean isSystemNoiseNotification(String pkg, String title, String text, boolean ongoing) {
        String t = (title == null ? "" : title.trim()) + " " + (text == null ? "" : text.trim());
        if (t.trim().isEmpty()) return true;
        if ("com.stepx.companion.app".equals(pkg)) return true;
        if (t.contains("正在后台运行") || t.contains("微信正在运行")
                || t.contains("网页版微信已登录") || t.contains("Windows 微信已登录")
                || t.contains("Mac 微信已登录") || t.contains("桌面版微信已登录")
                || t.contains("正在通过 USB") || t.contains("无线调试已连接")) {
            return true;
        }
        if (ongoing && isImPackage(pkg) && (t.contains("正在运行") || t.contains("后台服务") || t.contains("保持连接"))) {
            return true;
        }
        return false;
    }

    /**
     * Classifies a notification into one of:
     * "im", "social", "finance", "delivery_travel", "system", "other".
     */
    public static String classifyNotification(
            String pkg,
            String categoryHint,
            String title,
            String text,
            boolean hasRemoteInput) {
        String safePkg = pkg == null ? "" : pkg.trim();
        String safeHint = categoryHint == null ? "" : categoryHint.trim().toLowerCase(Locale.ROOT);
        String safeTitle = title == null ? "" : title.trim();
        String safeText = text == null ? "" : text.trim();
        String combined = safeTitle + " " + safeText;

        if (SYSTEM_PACKAGES.contains(safePkg)
                || safePkg.startsWith("com.android.systemui")
                || safePkg.startsWith("com.android.providers.")
                || isSystemNoiseNotification(safePkg, safeTitle, safeText, false)) {
            return CAT_SYSTEM;
        }

        // WeChat Pay / Alipay / Bank transaction alerts
        if (FINANCE_PACKAGES.contains(safePkg)
                || safeTitle.contains("微信支付")
                || safeTitle.contains("支付宝")
                || containsAny(combined, "付款成功", "支付成功", "收款到账", "动账提醒", "信用卡", "消费提醒", "转账到账", "账户余额", "账单提醒", "扣款成功")) {
            return CAT_FINANCE;
        }

        // Express delivery, food delivery, ride-hailing, flights/trains
        if (containsAny(combined, "取件码", "提货码", "丰巢", "菜鸟驿站", "派送中", "已签收", "快件", "包裹已到", "骑手", "外卖", "航班", "登机口", "检票口")) {
            return CAT_DELIVERY_TRAVEL;
        }

        // SMS special handling: if SMS is purely OTP or system security alert without personal chat, check finance/delivery/system
        if ("com.android.mms".equals(safePkg) || "com.google.android.apps.messaging".equals(safePkg)) {
            if (containsAny(combined, "验证码", "校验码", "动态码", "安全码", "verification code", "OTP")) {
                return CAT_IM;
            }
            return CAT_IM;
        }

        if (CORE_IM_PACKAGES.contains(safePkg)) {
            return CAT_IM;
        }

        if (DELIVERY_TRAVEL_PACKAGES.contains(safePkg)) {
            if (hasRemoteInput || "msg".equals(safeHint)) {
                return CAT_IM;
            }
            return CAT_DELIVERY_TRAVEL;
        }

        if (SOCIAL_PACKAGES.contains(safePkg)) {
            return CAT_SOCIAL;
        }

        if (hasRemoteInput || "msg".equals(safeHint)) {
            return CAT_IM;
        }
        if ("social".equals(safeHint)) {
            return CAT_SOCIAL;
        }
        if ("transport".equals(safeHint) || "navigation".equals(safeHint)) {
            return CAT_DELIVERY_TRAVEL;
        }
        if ("sys".equals(safeHint) || "service".equals(safeHint) || "security".equals(safeHint)
                || containsAny(combined, "验证码", "校验码", "动态码", "系统更新", "权限提醒", "电量不足")) {
            return CAT_SYSTEM;
        }
        return CAT_OTHER;
    }

    private static boolean containsAny(String haystack, String... needles) {
        if (haystack == null || haystack.isEmpty()) return false;
        for (String n : needles) {
            if (haystack.contains(n)) return true;
        }
        return false;
    }

    /**
     * Strips unread count prefixes/suffixes so notifications from the same contact or group
     * normalize to a stable conversation title.
     */
    public static String cleanConversationTitle(String pkg, String rawTitle) {
        if (rawTitle == null) return "";
        String cleaned = rawTitle.replace('\u00A0', ' ').trim();
        if (cleaned.isEmpty()) return "";

        // Strip leading unread prefixes (possibly stacked)
        for (int i = 0; i < 2; i++) {
            Matcher m = LEADING_UNREAD_PATTERN.matcher(cleaned);
            if (m.find()) {
                cleaned = cleaned.substring(m.end()).trim();
            } else {
                break;
            }
        }

        // Strip trailing unread count suffixes (e.g. "产品研发群 (5)" or "研发群(5条新消息)")
        for (int i = 0; i < 2; i++) {
            Matcher m = TRAILING_UNREAD_PATTERN.matcher(cleaned);
            if (m.find()) {
                String prefix = cleaned.substring(0, m.start()).trim();
                if (!prefix.isEmpty()) {
                    cleaned = prefix;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        return cleaned.trim();
    }

    public static String buildThreadId(String pkg, String rawTitle) {
        String safePkg = pkg == null || pkg.isEmpty() ? "unknown" : pkg.trim();
        String cleanTitle = cleanConversationTitle(safePkg, rawTitle);
        if (cleanTitle.isEmpty()) cleanTitle = "未知会话";
        return safePkg + "::" + cleanTitle;
    }

    /**
     * Splits group or single-chat notification text into sender and message body.
     * For example:
     *   title="产品研发群", text="[4条] 王敏: 今晚8点发版", isGroup=true
     *   -> sender="王敏", body="今晚8点发版"
     */
    public static SenderBody extractSenderAndBody(String pkg, String title, String text, boolean isGroup) {
        String cleanTitle = cleanConversationTitle(pkg, title);
        String body = text == null ? "" : text.trim();

        // Strip leading unread badge from body text (e.g. "[4条] 王敏: 今晚8点发版")
        for (int i = 0; i < 2; i++) {
            Matcher m = LEADING_UNREAD_PATTERN.matcher(body);
            if (m.find()) {
                body = body.substring(m.end()).trim();
            } else {
                break;
            }
        }

        // If body repeats the conversation title at the start ("产品群: 李四: 方案发我一下"), strip it first
        if (!cleanTitle.isEmpty()) {
            if (body.startsWith(cleanTitle + ":") || body.startsWith(cleanTitle + "：")) {
                body = body.substring(cleanTitle.length() + 1).trim();
            }
        }

        Matcher split = SENDER_SPLIT_PATTERN.matcher(body);
        if (split.matches()) {
            String candidateSender = split.group(1).trim();
            String candidateBody = split.group(2).trim();
            if (isValidSenderName(candidateSender, candidateBody, cleanTitle, isGroup)) {
                return new SenderBody(candidateSender, candidateBody);
            }
        }

        return new SenderBody(cleanTitle, body);
    }

    private static boolean isValidSenderName(String sender, String remainder, String cleanTitle, boolean isGroup) {
        if (sender.isEmpty() || remainder.isEmpty()) return false;
        // Do not split URLs like "https://..." or "http://..."
        if (remainder.startsWith("//")) return false;
        String lower = sender.toLowerCase(Locale.ROOT);
        if ("http".equals(lower) || "https".equals(lower) || "mailto".equals(lower) || "tel".equals(lower)) {
            return false;
        }
        // Do not split pure timestamps like "14:30"
        if (sender.matches("^[0-9]{1,2}$") && remainder.matches("^[0-9]{2}.*")) {
            return false;
        }
        // Do not split common field labels like "验证码: 123456" unless sender equals cleanTitle
        if (containsAny(sender, "验证码", "取件码", "订单", "位置", "链接", "注意", "提示", "时间", "地点")
                && !sender.equals(cleanTitle)) {
            return false;
        }
        if (isGroup) return true;
        if (sender.equals(cleanTitle)) return true;
        // Even if isGroup flag wasn't set by the app (e.g. standard WeChat text notification for a group),
        // if title looks like a group or sender is a short human name distinct from labels, allow splitting!
        return sender.length() <= 16;
    }

    /**
     * Extracts 4-8 digit verification codes or express cabinet pickup codes when OTP/pickup
     * keywords appear, avoiding false positives on years, prices, and phone numbers.
     */
    public static String extractOtpCode(String title, String text) {
        String safeTitle = title == null ? "" : title;
        String safeText = text == null ? "" : text;
        String combined = (safeTitle + " " + safeText).trim();
        if (combined.isEmpty()) return null;

        if (!OTP_TRIGGER_PATTERN.matcher(combined).find()) {
            return null;
        }

        // 1. Check express cabinet hyphenated pickup code first (e.g. "4-2-1089")
        if (PICKUP_TRIGGER_PATTERN.matcher(combined).find()) {
            Matcher pickup = PICKUP_HYPHEN_CODE_PATTERN.matcher(combined);
            if (pickup.find()) {
                return pickup.group(1);
            }
        }

        // 2. Check high-confidence pattern directly following keyword
        Matcher directAfter = OTP_DIRECT_AFTER_KEYWORD.matcher(combined);
        while (directAfter.find()) {
            String candidate = directAfter.group(1);
            if (isPlausibleOtpCandidate(combined, directAfter.start(1), directAfter.end(1), candidate)) {
                return candidate;
            }
        }

        // 3. Check high-confidence pattern directly preceding keyword
        Matcher directBefore = OTP_DIRECT_BEFORE_KEYWORD.matcher(combined);
        while (directBefore.find()) {
            String candidate = directBefore.group(1);
            if (isPlausibleOtpCandidate(combined, directBefore.start(1), directBefore.end(1), candidate)) {
                return candidate;
            }
        }

        // 4. Fallback: scan standalone 4-8 digit tokens with false-positive guards
        Matcher generic = STANDALONE_DIGITS_PATTERN.matcher(combined);
        while (generic.find()) {
            String candidate = generic.group(1);
            if (isPlausibleOtpCandidate(combined, generic.start(1), generic.end(1), candidate)) {
                return candidate;
            }
        }
        return null;
    }

    private static boolean isPlausibleOtpCandidate(String text, int start, int end, String digits) {
        if (digits == null || digits.length() < 4 || digits.length() > 8) return false;

        String before = text.substring(Math.max(0, start - 6), start);
        String after = text.substring(end, Math.min(text.length(), end + 6));

        // Reject prices/amounts (e.g. "￥1680", "1680元", "$2000")
        if (before.endsWith("￥") || before.endsWith("¥") || before.endsWith("$")
                || before.contains("金额") || before.contains("消费") || before.contains("付款")) {
            return false;
        }
        if (after.startsWith("元") || after.startsWith("块") || after.startsWith("角")
                || after.startsWith("分钟") || after.startsWith("小时") || after.startsWith("秒")
                || after.startsWith("年") || after.startsWith("月") || after.startsWith("日")
                || after.startsWith("人") || after.startsWith("条") || after.startsWith("个")
                || after.startsWith("%")) {
            return false;
        }

        // Reject card tail numbers if explicitly labelled "尾号1234"
        if (before.endsWith("尾号") || before.endsWith("末四位") || before.endsWith("后四位")
                || before.endsWith("卡号")) {
            return false;
        }

        // Reject 4-digit calendar years (e.g. 2024, 2025, 2026) when followed by date separators or in date context
        if (digits.length() == 4) {
            int val = Integer.parseInt(digits);
            if (val >= 1990 && val <= 2040 && (after.startsWith("-") || after.startsWith("/") || after.startsWith("."))) {
                return false;
            }
        }
        return true;
    }

    /**
     * Extracts the first http/https URL from notification text, stripping trailing punctuation.
     */
    public static String extractFirstUrl(String text) {
        if (text == null || text.isEmpty()) return null;
        Matcher m = URL_PATTERN.matcher(text);
        if (!m.find()) return null;
        String raw = m.group(1);
        while (!raw.isEmpty()) {
            char last = raw.charAt(raw.length() - 1);
            if (last == '.' || last == ',' || last == ';' || last == ')' || last == ']'
                    || last == '}' || last == '!' || last == '?' || last == '\'' || last == '"') {
                raw = raw.substring(0, raw.length() - 1);
            } else {
                break;
            }
        }
        return raw.length() > 8 ? raw : null;
    }
}
