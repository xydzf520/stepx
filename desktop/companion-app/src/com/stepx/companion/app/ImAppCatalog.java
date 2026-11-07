package com.stepx.companion.app;

import android.content.Context;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
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

    // Trailing count suffixes with explicit unread/member units: " (48人)", "（48人）", " (3条)", " (5条新消息)", " (48 members)"
    private static final Pattern TRAILING_EXPLICIT_COUNT_PATTERN = Pattern.compile(
        "\\s*(?:[（(\\[][0-9]+\\+?\\s*(?:条(?:新?消息|未读)?|未读|新消息|人|个?成员|members?|messages?|new(?:\\s+messages?)?)[)）\\]])\\s*$",
        Pattern.CASE_INSENSITIVE
    );

    // Trailing bare count suffix without unit: " (3)", " (48)", "（48）", " [4]" (capped at 5 digits to avoid phone numbers)
    private static final Pattern TRAILING_BARE_COUNT_PATTERN = Pattern.compile(
        "\\s*(?:\\([0-9]{1,5}\\+?\\)|（[0-9]{1,5}\\+?）|\\[[0-9]{1,5}\\+?\\])\\s*$"
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
        // 1. IM & Social DM
        names.put("com.tencent.mm", "微信");
        names.put("com.tencent.wework", "企业微信");
        names.put("com.alibaba.android.rimet", "钉钉");
        names.put("com.ss.android.lark", "飞书");
        names.put("com.ss.android.lark.kami", "飞书");
        names.put("com.larksuite.suite", "飞书");
        names.put("com.tencent.mobileqq", "QQ");
        names.put("com.tencent.tim", "TIM");
        names.put("com.tencent.qqlite", "QQ轻聊");
        names.put("com.xingin.xhs", "小红书");
        names.put("com.ss.android.ugc.aweme", "抖音");
        names.put("com.ss.android.ugc.aweme.lite", "抖音");
        names.put("com.smile.gifmaker", "快手");
        names.put("com.kuaishou.nebula", "快手");
        names.put("com.sina.weibo", "微博");
        names.put("com.sina.weibog3", "微博");
        names.put("com.weico.international", "微博");
        names.put("tv.danmaku.bili", "哔哩哔哩");
        names.put("com.bilibili.app.in", "哔哩哔哩");
        names.put("com.zhihu.android", "知乎");
        names.put("com.douban.frodo", "豆瓣");
        names.put("com.baidu.tieba", "百度贴吧");
        names.put("cn.soulapp.android", "Soul");
        names.put("com.immomo.momo", "陌陌");
        names.put("com.p1.mobile.putong", "探探");
        names.put("com.mihoyo.hyperion", "米游社");
        names.put("com.max.xiaoheihe", "小黑盒");
        names.put("com.taptap", "TapTap");
        names.put("com.hupu.games", "虎扑");
        names.put("com.coolapk.market", "酷安");
        names.put("org.telegram.messenger", "Telegram");
        names.put("org.telegram.messenger.web", "Telegram");
        names.put("org.thunderdog.challegram", "Telegram X");
        names.put("org.telegram.plus", "Telegram");
        names.put("tw.nekomimi.nekogram", "Nekogram");
        names.put("com.whatsapp", "WhatsApp");
        names.put("com.whatsapp.w4b", "WhatsApp Business");
        names.put("com.Slack", "Slack");
        names.put("com.slack", "Slack");
        names.put("com.discord", "Discord");
        names.put("com.microsoft.teams", "Teams");
        names.put("org.thoughtcrime.securesms", "Signal");
        names.put("jp.naver.line.android", "LINE");
        names.put("com.facebook.orca", "Messenger");
        names.put("com.kakao.talk", "KakaoTalk");
        names.put("com.viber.voip", "Viber");
        names.put("com.instagram.android", "Instagram");
        names.put("com.instagram.barcelona", "Instagram");
        names.put("com.twitter.android", "X");
        names.put("com.facebook.katana", "Facebook");
        names.put("com.reddit.frontpage", "Reddit");
        names.put("com.android.mms", "短信");
        names.put("com.google.android.apps.messaging", "短信");
        names.put("com.samsung.android.messaging", "短信");
        names.put("com.android.messaging", "短信");
        names.put("com.miui.mms", "短信");

        // 2. Career & Recruitment
        names.put("com.hpbr.bosszhipin", "BOSS直聘");
        names.put("com.taou.maimai", "脉脉");
        names.put("com.zhaopin.social", "智联招聘");
        names.put("com.job.android", "前程无忧");
        names.put("com.lietou.mishu", "猎聘");
        names.put("com.lietpin.android", "猎聘");
        names.put("com.lietpin.android.LPActivity", "猎聘");
        names.put("com.linkedin.android", "领英");
        names.put("com.wuba", "58同城");
        names.put("com.yupao.android", "鱼泡直聘");
        names.put("com.shixiseng.activity", "实习僧");

        // 3. E-commerce & Customer Service
        names.put("com.taobao.taobao", "淘宝");
        names.put("com.tmall.wireless", "天猫");
        names.put("com.taobao.idlefish", "闲鱼");
        names.put("com.jingdong.app.mall", "京东");
        names.put("com.xunmeng.pinduoduo", "拼多多");
        names.put("com.shizhuang.duapp", "得物");
        names.put("com.achievo.vipshop", "唯品会");
        names.put("com.alibaba.wireless", "1688");
        names.put("com.wuba.zhuanzhuan", "转转");
        names.put("com.ss.android.merchant", "抖店");
        names.put("com.xiaomi.shop", "小米商城");
        names.put("com.vmall.client", "华为商城");

        // 4. Food Delivery, Grocery, Express & Travel
        names.put("com.sankuai.meituan", "美团");
        names.put("com.sankuai.meituan.takeoutnew", "美团外卖");
        names.put("com.dianping.v1", "大众点评");
        names.put("me.ele", "饿了么");
        names.put("com.yaya.zone", "叮咚买菜");
        names.put("com.wudaokou.hippo", "盒马");
        names.put("cn.samsclub.app", "山姆会员商店");
        names.put("com.pupumall.customer", "朴朴超市");
        names.put("com.sf.activity", "顺丰速运");
        names.put("com.cainiao.wireless", "菜鸟");
        names.put("com.Cainiao.wireless", "菜鸟");
        names.put("com.jd.express", "京东快递");
        names.put("com.fcbox.hiveconsumer", "丰巢");
        names.put("com.zto.consumer", "中通快递");
        names.put("com.yto.eyto", "圆通速递");
        names.put("com.yunda.express", "韵达快递");
        names.put("com.jtexpress.client", "极兔速递");
        names.put("com.sdu.didi.psnger", "滴滴出行");
        names.put("com.autonavi.minimap", "高德地图");
        names.put("com.baidu.BaiduMap", "百度地图");
        names.put("com.jingyao.easybike", "哈啰");
        names.put("cn.caocaokeji.user", "曹操出行");
        names.put("com.t3go.passenger", "T3出行");
        names.put("com.MobileTicket", "铁路12306");
        names.put("com.umetrip.android.msky.app", "航旅纵横");
        names.put("ctrip.android.view", "携程");
        names.put("com.Qunar", "去哪儿");
        names.put("com.taobao.trip", "飞猪");
        names.put("com.tongcheng.android", "同程旅行");
        names.put("com.htinns", "华住会");
        names.put("com.jd.jddj", "京东到家");
        names.put("com.dmall.mfandroid", "多点");
        names.put("com.ishansong", "闪送");
        names.put("com.lucky.luckyclient", "瑞幸咖啡");
        names.put("com.heytea.android", "喜茶");
        names.put("com.chagee.app", "霸王茶姬");

        // 5. Finance, Payment & Banking
        names.put("com.eg.android.AlipayGphone", "支付宝");
        names.put("com.unionpay", "云闪付");
        names.put("cmb.pb", "招商银行");
        names.put("com.cmbchina.ccd.pluto.cmbActivity", "掌上生活");
        names.put("com.icbc", "工商银行");
        names.put("com.chinamworld.main", "建设银行");
        names.put("com.android.bankabc", "农业银行");
        names.put("com.chinamworld.bocmbci", "中国银行");
        names.put("com.bankcomm.Bankcomm", "交通银行");
        names.put("com.pingan.paces.ccms", "平安口袋银行");
        names.put("com.citiccard.mobilebank", "中信银行");
        names.put("com.cmbc.cc.mbank", "民生银行");
        names.put("com.spdbccc.app", "浦发银行");
        names.put("com.cebbank.mobile.cemb", "光大银行");
        names.put("com.cib.cibmb", "兴业银行");

        // 6. Workspace, Mail & Dev
        names.put("com.tencent.wemeet.app", "腾讯会议");
        names.put("us.zoom.videomeetings", "Zoom");
        names.put("com.tencent.docs", "腾讯文档");
        names.put("cn.wps.moffice_eng", "WPS");
        names.put("chuxin.shimo.shimowendang", "石墨文档");
        names.put("com.yuque.mobile.android.app", "语雀");
        names.put("notion.id", "Notion");
        names.put("com.alicloud.databox", "阿里云盘");
        names.put("com.baidu.netdisk", "百度网盘");
        names.put("com.quark.browser", "夸克");
        names.put("com.tencent.androidqqmail", "QQ邮箱");
        names.put("com.netease.mobimail", "网易邮箱大师");
        names.put("cn.cj.pe", "139邮箱");
        names.put("com.alibaba.cloudmail", "阿里邮箱");
        names.put("com.google.android.gm", "Gmail");
        names.put("com.microsoft.office.outlook", "Outlook");
        names.put("com.readdle.spark", "Spark");
        names.put("com.github.android", "GitHub");
        names.put("com.android.calendar", "日历");
        names.put("com.google.android.calendar", "日历");
        names.put("com.coloros.calendar", "日历");
        names.put("com.miui.calendar", "日历");
        names.put("com.huawei.calendar", "日历");
        names.put("com.samsung.android.calendar", "日历");
        names.put("com.android.dialer", "电话");
        names.put("com.google.android.dialer", "电话");
        names.put("com.android.server.telecom", "电话");
        names.put("com.android.incallui", "电话");
        names.put("com.android.deskclock", "闹钟");
        names.put("com.google.android.deskclock", "闹钟");
        names.put("com.android.alarmclock", "闹钟");
        names.put("com.coloros.alarmclock", "闹钟");
        names.put("com.miui.deskclock", "闹钟");
        DISPLAY_NAMES = Collections.unmodifiableMap(names);

        Set<String> im = new HashSet<String>();
        im.add("com.tencent.mm");
        im.add("com.tencent.wework");
        im.add("com.alibaba.android.rimet");
        im.add("com.ss.android.lark");
        im.add("com.ss.android.lark.kami");
        im.add("com.larksuite.suite");
        im.add("com.tencent.mobileqq");
        im.add("com.tencent.tim");
        im.add("com.tencent.qqlite");
        im.add("org.telegram.messenger");
        im.add("org.telegram.messenger.web");
        im.add("org.thunderdog.challegram");
        im.add("org.telegram.plus");
        im.add("tw.nekomimi.nekogram");
        im.add("com.whatsapp");
        im.add("com.whatsapp.w4b");
        im.add("com.Slack");
        im.add("com.slack");
        im.add("com.discord");
        im.add("com.microsoft.teams");
        im.add("org.thoughtcrime.securesms");
        im.add("jp.naver.line.android");
        im.add("com.facebook.orca");
        im.add("com.kakao.talk");
        im.add("com.viber.voip");
        im.add("com.android.mms");
        im.add("com.google.android.apps.messaging");
        im.add("com.samsung.android.messaging");
        im.add("com.android.messaging");
        im.add("com.miui.mms");
        im.add("com.hpbr.bosszhipin");
        im.add("com.taou.maimai");
        im.add("com.lietou.mishu");
        im.add("com.lietpin.android");
        im.add("com.lietpin.android.LPActivity");
        im.add("com.zhaopin.social");
        im.add("com.job.android");
        im.add("com.linkedin.android");
        im.add("com.wuba");
        im.add("com.yupao.android");
        im.add("com.shixiseng.activity");
        im.add("com.tencent.wemeet.app");
        im.add("us.zoom.videomeetings");
        im.add("com.tencent.docs");
        im.add("cn.wps.moffice_eng");
        im.add("chuxin.shimo.shimowendang");
        im.add("com.yuque.mobile.android.app");
        im.add("notion.id");
        im.add("com.alicloud.databox");
        im.add("com.baidu.netdisk");
        im.add("com.quark.browser");
        im.add("com.tencent.androidqqmail");
        im.add("com.netease.mobimail");
        im.add("cn.cj.pe");
        im.add("com.alibaba.cloudmail");
        im.add("com.google.android.gm");
        im.add("com.microsoft.office.outlook");
        im.add("com.readdle.spark");
        im.add("com.github.android");
        CORE_IM_PACKAGES = Collections.unmodifiableSet(im);

        Set<String> social = new HashSet<String>();
        social.add("com.xingin.xhs");
        social.add("com.ss.android.ugc.aweme");
        social.add("com.ss.android.ugc.aweme.lite");
        social.add("com.smile.gifmaker");
        social.add("com.kuaishou.nebula");
        social.add("com.sina.weibo");
        social.add("com.sina.weibog3");
        social.add("com.weico.international");
        social.add("tv.danmaku.bili");
        social.add("com.bilibili.app.in");
        social.add("com.zhihu.android");
        social.add("com.douban.frodo");
        social.add("com.baidu.tieba");
        social.add("cn.soulapp.android");
        social.add("com.immomo.momo");
        social.add("com.p1.mobile.putong");
        social.add("com.mihoyo.hyperion");
        social.add("com.max.xiaoheihe");
        social.add("com.taptap");
        social.add("com.hupu.games");
        social.add("com.coolapk.market");
        social.add("com.instagram.android");
        social.add("com.instagram.barcelona");
        social.add("com.twitter.android");
        social.add("com.facebook.katana");
        social.add("com.reddit.frontpage");
        social.add("com.taobao.taobao");
        social.add("com.tmall.wireless");
        social.add("com.taobao.idlefish");
        social.add("com.jingdong.app.mall");
        social.add("com.xunmeng.pinduoduo");
        social.add("com.shizhuang.duapp");
        social.add("com.achievo.vipshop");
        social.add("com.alibaba.wireless");
        social.add("com.wuba.zhuanzhuan");
        social.add("com.ss.android.merchant");
        social.add("com.xiaomi.shop");
        social.add("com.vmall.client");
        social.add("com.github.android");
        SOCIAL_PACKAGES = Collections.unmodifiableSet(social);

        Set<String> finance = new HashSet<String>();
        finance.add("com.eg.android.AlipayGphone");
        finance.add("cmb.pb");
        finance.add("com.cmbchina.ccd.pluto.cmbActivity");
        finance.add("com.unionpay");
        finance.add("com.icbc");
        finance.add("com.chinamworld.main");
        finance.add("com.android.bankabc");
        finance.add("com.chinamworld.bocmbci");
        finance.add("com.bankcomm.Bankcomm");
        finance.add("com.pingan.paces.ccms");
        finance.add("com.citiccard.mobilebank");
        finance.add("com.cmbc.cc.mbank");
        finance.add("com.spdbccc.app");
        finance.add("com.cebbank.mobile.cemb");
        finance.add("com.cib.cibmb");
        FINANCE_PACKAGES = Collections.unmodifiableSet(finance);

        Set<String> delivery = new HashSet<String>();
        delivery.add("com.cainiao.wireless");
        delivery.add("com.Cainiao.wireless");
        delivery.add("com.sf.activity");
        delivery.add("com.jd.express");
        delivery.add("com.fcbox.hiveconsumer");
        delivery.add("com.zto.consumer");
        delivery.add("com.yto.eyto");
        delivery.add("com.yunda.express");
        delivery.add("com.jtexpress.client");
        delivery.add("com.sankuai.meituan");
        delivery.add("com.sankuai.meituan.takeoutnew");
        delivery.add("me.ele");
        delivery.add("com.sdu.didi.psnger");
        delivery.add("com.autonavi.minimap");
        delivery.add("com.baidu.BaiduMap");
        delivery.add("com.jingyao.easybike");
        delivery.add("cn.caocaokeji.user");
        delivery.add("com.t3go.passenger");
        delivery.add("com.MobileTicket");
        delivery.add("com.umetrip.android.msky.app");
        delivery.add("ctrip.android.view");
        delivery.add("com.Qunar");
        delivery.add("com.taobao.trip");
        delivery.add("com.tongcheng.android");
        delivery.add("com.htinns");
        delivery.add("com.yaya.zone");
        delivery.add("com.wudaokou.hippo");
        delivery.add("com.pupumall.customer");
        delivery.add("cn.samsclub.app");
        delivery.add("com.jd.jddj");
        delivery.add("com.dmall.mfandroid");
        delivery.add("com.dianping.v1");
        delivery.add("com.ishansong");
        delivery.add("com.lucky.luckyclient");
        delivery.add("com.heytea.android");
        delivery.add("com.chagee.app");
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

    public static boolean isMainstreamMessagePackage(String pkg) {
        if (pkg == null) return false;
        String safe = pkg.trim();
        return DISPLAY_NAMES.containsKey(safe)
            || CORE_IM_PACKAGES.contains(safe)
            || SOCIAL_PACKAGES.contains(safe)
            || FINANCE_PACKAGES.contains(safe)
            || DELIVERY_TRAVEL_PACKAGES.contains(safe);
    }

    public static boolean isNativeSmsPackage(String packageName) {
        if (packageName == null) return false;
        String pkg = packageName.trim();
        return "com.android.mms".equals(pkg)
            || "com.google.android.apps.messaging".equals(pkg)
            || "com.samsung.android.messaging".equals(pkg)
            || "com.android.messaging".equals(pkg)
            || "com.miui.mms".equals(pkg)
            || "com.coloros.mms".equals(pkg)
            || "com.oplus.mms".equals(pkg)
            || "com.huawei.message".equals(pkg)
            || "com.hihonor.mms".equals(pkg)
            || "com.vivo.message".equals(pkg);
    }

    public static boolean isSocialOrCommerceImPackage(String pkg) {
        if (pkg == null) return false;
        String safe = pkg.trim();
        return CORE_IM_PACKAGES.contains(safe)
            || SOCIAL_PACKAGES.contains(safe)
            || DELIVERY_TRAVEL_PACKAGES.contains(safe)
            || FINANCE_PACKAGES.contains(safe);
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

        // Core IM packages checked BEFORE keyword overrides so chat messages mentioning delivery/finance stay CAT_IM
        if (CORE_IM_PACKAGES.contains(safePkg)) {
            if ("com.tencent.mm".equals(safePkg) && safeTitle.contains("微信支付")) {
                return CAT_FINANCE;
            }
            return CAT_IM;
        }

        if (FINANCE_PACKAGES.contains(safePkg)) {
            return CAT_FINANCE;
        }

        if (DELIVERY_TRAVEL_PACKAGES.contains(safePkg)) {
            if (hasRemoteInput || "msg".equals(safeHint)) {
                return CAT_IM;
            }
            return CAT_DELIVERY_TRAVEL;
        }

        if (SOCIAL_PACKAGES.contains(safePkg)) {
            if (containsAny(combined, "已发货", "派送中", "已签收", "待取件", "取件码", "快件")) {
                return CAT_DELIVERY_TRAVEL;
            }
            if (hasRemoteInput || "msg".equals(safeHint)) {
                return CAT_IM;
            }
            return CAT_SOCIAL;
        }

        // WeChat Pay / Alipay / Bank transaction alerts
        if (safeTitle.contains("微信支付")
                || safeTitle.contains("支付宝")
                || containsAny(combined, "付款成功", "支付成功", "收款到账", "动账提醒", "信用卡", "消费提醒", "转账到账", "账户余额", "账单提醒", "扣款成功")) {
            return CAT_FINANCE;
        }

        // Express delivery, food delivery, ride-hailing, flights/trains, instant grocery/coffee
        if (containsAny(
                combined,
                "取件码",
                "提货码",
                "丰巢",
                "菜鸟驿站",
                "派送中",
                "已签收",
                "快件",
                "包裹已到",
                "骑手",
                "外卖",
                "航班",
                "登机口",
                "检票口",
                "叮咚买菜",
                "盒马",
                "山姆",
                "朴朴",
                "已代收",
                "家门口",
                "已送达",
                "订单配送",
                "商家已接单",
                "取餐码",
                "取茶码",
                "制作完成")) {
            return CAT_DELIVERY_TRAVEL;
        }

        // SMS special handling: if SMS is purely OTP or system security alert without personal chat, check finance/delivery/system
        if ("com.android.mms".equals(safePkg) || "com.google.android.apps.messaging".equals(safePkg)) {
            if (containsAny(combined, "验证码", "校验码", "动态码", "安全码", "verification code", "OTP")) {
                return CAT_IM;
            }
            return CAT_IM;
        }

        if (CORE_IM_PACKAGES.contains(safePkg)
                || containsAny(combined, "BOSS直聘", "面试邀约", "候选人", "牛人")) {
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
                || "event".equals(safeHint) || "reminder".equals(safeHint)
                || "alarm".equals(safeHint) || "call".equals(safeHint) || "missed_call".equals(safeHint)
                || safePkg.contains("calendar") || safePkg.contains("deskclock")
                || safePkg.contains("alarmclock") || safePkg.contains("dialer")
                || safePkg.contains("telecom") || safePkg.contains("incallui")
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
     * Strips only leading unread badges and trailing suffixes that have an explicit unit
     * (e.g. "[3条] ", "(5条新消息)", "(48人)"), preserving bare parenthesized numbers like "项目组(1)".
     */
    public static String stripExplicitBadges(String rawTitle) {
        if (rawTitle == null) return "";
        String cleaned = rawTitle.replace('\u00A0', ' ').trim();
        if (cleaned.isEmpty()) return "";

        for (int i = 0; i < 2; i++) {
            Matcher m = LEADING_UNREAD_PATTERN.matcher(cleaned);
            if (m.find()) {
                cleaned = cleaned.substring(m.end()).trim();
            } else {
                break;
            }
        }

        for (int i = 0; i < 2; i++) {
            Matcher m = TRAILING_EXPLICIT_COUNT_PATTERN.matcher(cleaned);
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

    /**
     * Strips unread count prefixes/suffixes so notifications from the same contact or group
     * normalize to a stable conversation title.
     */
    public static String cleanConversationTitle(String pkg, String rawTitle) {
        if (rawTitle == null) return "";
        String trimmed = rawTitle.replace('\u00A0', ' ').trim();
        if (trimmed.isEmpty()) return "";

        String afterLeading = trimmed;
        for (int i = 0; i < 2; i++) {
            Matcher m = LEADING_UNREAD_PATTERN.matcher(afterLeading);
            if (m.find()) {
                afterLeading = afterLeading.substring(m.end()).trim();
            } else {
                break;
            }
        }

        String cleaned = afterLeading;
        boolean strippedExplicitTrailing = false;
        for (int i = 0; i < 2; i++) {
            Matcher m = TRAILING_EXPLICIT_COUNT_PATTERN.matcher(cleaned);
            if (m.find()) {
                String prefix = cleaned.substring(0, m.start()).trim();
                if (!prefix.isEmpty()) {
                    cleaned = prefix;
                    strippedExplicitTrailing = true;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Strip at most one bare trailing number suffix (e.g. "研发群 (48)" or "项目组(1) (48)")
        // when no explicit-unit trailing suffix already consumed the outer badge.
        if (!strippedExplicitTrailing) {
            Matcher bare = TRAILING_BARE_COUNT_PATTERN.matcher(cleaned);
            if (bare.find()) {
                String prefix = cleaned.substring(0, bare.start()).trim();
                if (!prefix.isEmpty()) {
                    cleaned = prefix;
                    Matcher explicitInner = TRAILING_EXPLICIT_COUNT_PATTERN.matcher(cleaned);
                    if (explicitInner.find()) {
                        String innerPrefix = cleaned.substring(0, explicitInner.start()).trim();
                        if (!innerPrefix.isEmpty()) {
                            cleaned = innerPrefix;
                        }
                    }
                }
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

    /**
     * Checks whether a raw window title has a group member count suffix such as "测试群(5)" or "群聊（3人）".
     */
    public static boolean hasGroupMemberCountSuffix(String rawTitle) {
        return extractGroupMemberCount(rawTitle) >= 2;
    }

    /**
     * Extracts the trailing group member count from a raw window title (e.g. "研发群(12)" -> 12), or 0 if none.
     */
    public static int extractGroupMemberCount(String rawTitle) {
        if (rawTitle == null || rawTitle.isEmpty()) return 0;
        String s = rawTitle.trim();
        Matcher m = Pattern.compile("(?:\\(|（)(\\d{1,5})人?(?:\\)|）)$").matcher(s);
        if (m.find()) {
            try {
                return Integer.parseInt(m.group(1));
            } catch (NumberFormatException ignored) {
                return 0;
            }
        }
        return 0;
    }

    /**
     * Returns true if the conversation title is a generic placeholder (such as WeChat's privacy-masked "群聊")
     * rather than a specific contact or custom group name.
     */
    public static boolean isGenericPlaceholderTitle(String title) {
        if (title == null) return true;
        String s = cleanConversationTitle("", title).trim();
        if (s.isEmpty()) return true;
        if (s.startsWith("群聊")) {
            String rest = s.substring(2).trim();
            if (rest.isEmpty() || rest.startsWith("(") || rest.startsWith("（") || rest.startsWith("·")) {
                return true;
            }
        }
        String lower = s.toLowerCase(Locale.ROOT);
        return "群聊".equals(s)
            || "微信".equals(s)
            || "wechat".equals(lower)
            || "weixin".equals(lower)
            || "未知会话".equals(s)
            || "未知".equals(s)
            || "新消息".equals(s)
            || "消息".equals(s)
            || "聊天".equals(s)
            || "会话".equals(s)
            || "group chat".equals(lower)
            || "qq".equals(lower)
            || "钉钉".equals(s)
            || "飞书".equals(s)
            || "企业微信".equals(s);
    }

    /**
     * Extracts participant nickname hints from an enriched generic group title such as
     * "群聊 (邱-stepx、窝-StepX)", "群聊 · 邱-stepx、窝-StepX", or "群聊 (窝-StepX)".
     */
    public static List<String> extractGenericGroupParticipantHints(String title) {
        List<String> out = new ArrayList<String>();
        if (title == null || title.trim().isEmpty()) return out;
        String s = title.trim();
        int sep = s.indexOf("::");
        if (sep >= 0 && sep + 2 < s.length()) {
            s = s.substring(sep + 2).trim();
        }
        s = cleanConversationTitle("", s).trim();
        if (!s.startsWith("群聊")) return out;
        String rest = s.substring(2).trim();
        // Strip optional leading group member count like "(4)" before "·"
        Matcher leadingCount = Pattern.compile("^(?:\\(|（)\\d{1,5}人?(?:\\)|）)\\s*").matcher(rest);
        if (leadingCount.find()) {
            rest = rest.substring(leadingCount.end()).trim();
        }
        if (rest.startsWith("·")) {
            rest = rest.substring(1).trim();
        }
        if ((rest.startsWith("(") && rest.endsWith(")")) || (rest.startsWith("（") && rest.endsWith("）"))) {
            rest = rest.substring(1, rest.length() - 1).trim();
        } else if (rest.startsWith("(") || rest.startsWith("（")) {
            rest = rest.substring(1).trim();
            if (rest.endsWith(")") || rest.endsWith("）")) {
                rest = rest.substring(0, rest.length() - 1).trim();
            }
        }
        if (rest.endsWith("等") && rest.length() > 1) {
            rest = rest.substring(0, rest.length() - 1).trim();
        }
        if (rest.isEmpty()) return out;
        String[] parts = rest.split("[,，、·/\\\\|;；]+");
        for (String part : parts) {
            String tok = part == null ? "" : part.trim();
            while (tok.startsWith("(") || tok.startsWith("（")) {
                tok = tok.substring(1).trim();
            }
            while (tok.endsWith(")") || tok.endsWith("）")) {
                tok = tok.substring(0, tok.length() - 1).trim();
            }
            if (tok.endsWith("等") && tok.length() > 1 && parts.length > 1) {
                tok = tok.substring(0, tok.length() - 1).trim();
            }
            if (tok.isEmpty() || tok.matches("^\\d{1,5}人?$")) continue;
            if (isGenericPlaceholderTitle(tok) || isGenericSenderPlaceholder(tok) || "我".equals(tok)) {
                continue;
            }
            if (!out.contains(tok)) {
                out.add(tok);
            }
        }
        return out;
    }

    /**
     * Infers a media hint tag (e.g. "[图片]", "[动画表情]", "[语音] 20\"", "[视频]", "[小程序]") from message text.
     */
    public static String inferMediaHint(String text) {
        if (text == null || text.trim().isEmpty()) return "";
        SenderBody sb = extractSenderAndBody("", "", text.trim(), true);
        String candidate = !sb.body.isEmpty() ? sb.body.trim() : text.trim();
        if (candidate.startsWith("[语音]")) {
            return candidate;
        }
        String[] tags = new String[] { "[图片]", "[动画表情]", "[语音]", "[视频]", "[小程序]" };
        for (String tag : tags) {
            int idx = candidate.indexOf(tag);
            if (idx >= 0) {
                if ("[语音]".equals(tag)) {
                    String tail = candidate.substring(idx).trim();
                    if (!tail.isEmpty()) return tail;
                }
                return tag;
            }
        }
        return "";
    }

    /**
     * Returns true if the sender name is a generic placeholder (such as WeChat's "微信用户" or "群聊")
     * that should be upgraded when the real sender nickname is observed in the chat window or conversation list.
     */
    public static boolean isGenericSenderPlaceholder(String sender) {
        if (sender == null) return true;
        String s = sender.trim();
        if (s.isEmpty()) return true;
        String lower = s.toLowerCase(Locale.ROOT);
        return "微信用户".equals(s)
            || "群聊".equals(s)
            || "未知".equals(s)
            || "未知用户".equals(s)
            || "微信".equals(s)
            || "wechat".equals(lower)
            || "群成员".equals(s)
            || "对方".equals(s)
            || "用户".equals(s)
            || "qq用户".equals(lower);
    }

    /**
     * Extracts the sender nickname from an IM avatar ImageView's contentDescription
     * (e.g. "张三头像" -> "张三", "敏的头像" -> "敏", "Alex's avatar" -> "Alex").
     * Returns "" if the description is not an avatar description or only contains a generic label.
     */
    public static String extractAvatarSenderName(String contentDesc) {
        if (contentDesc == null) return "";
        String s = contentDesc.trim();
        if (s.isEmpty() || s.length() > 40) return "";
        String name = "";
        if (s.endsWith("的头像") && s.length() > 3) {
            name = s.substring(0, s.length() - 3).trim();
        } else if (s.endsWith("头像") && s.length() > 2) {
            name = s.substring(0, s.length() - 2).trim();
        } else if (s.toLowerCase(Locale.ROOT).endsWith("'s avatar") && s.length() > 9) {
            name = s.substring(0, s.length() - 9).trim();
        } else if (s.toLowerCase(Locale.ROOT).endsWith(" avatar") && s.length() > 7) {
            name = s.substring(0, s.length() - 7).trim();
        }
        if (name.isEmpty()) return "";
        if ("我".equals(name) || "自己".equals(name) || "本人".equals(name)
                || "默认".equals(name) || "群".equals(name)
                || isGenericSenderPlaceholder(name)
                || isGenericPlaceholderTitle(name)) {
            return "";
        }
        return name;
    }
}
