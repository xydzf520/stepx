package com.stepx.companion.app;

import android.app.PendingIntent;
import android.content.Context;
import android.service.notification.NotificationListenerService;
import java.util.concurrent.atomic.AtomicInteger;

public final class ImPushCatalogTest {
    private static void check(boolean cond, String msg) {
        if (!cond) throw new AssertionError(msg);
    }

    public static void main(String[] args) throws Exception {
        // 1. IM package detection & display names
        check(ImAppCatalog.isImPackage("com.tencent.mm"), "WeChat must be IM");
        check(ImAppCatalog.isImPackage("com.tencent.wework"), "WeCom must be IM");
        check(ImAppCatalog.isImPackage("com.alibaba.android.rimet"), "DingTalk must be IM");
        check(ImAppCatalog.isImPackage("com.ss.android.lark"), "Feishu must be IM");
        check(ImAppCatalog.isImPackage("com.tencent.mobileqq"), "QQ must be IM");
        check(ImAppCatalog.isImPackage("com.tencent.tim"), "TIM must be IM");
        check(ImAppCatalog.isImPackage("org.telegram.messenger"), "Telegram must be IM");
        check(ImAppCatalog.isImPackage("com.whatsapp"), "WhatsApp must be IM");
        check(ImAppCatalog.isImPackage("com.Slack"), "Slack must be IM");
        check(ImAppCatalog.isImPackage("com.discord"), "Discord must be IM");
        check("微信".equals(ImAppCatalog.appDisplayName(null, "com.tencent.mm")), "WeChat display name");
        check("企业微信".equals(ImAppCatalog.appDisplayName(null, "com.tencent.wework")), "WeCom display name");
        check("钉钉".equals(ImAppCatalog.appDisplayName(null, "com.alibaba.android.rimet")), "DingTalk display name");
        check("飞书".equals(ImAppCatalog.appDisplayName(null, "com.ss.android.lark")), "Feishu display name");
        check("小红书".equals(ImAppCatalog.appDisplayName(null, "com.xingin.xhs")), "Xiaohongshu display name");
        check("抖音".equals(ImAppCatalog.appDisplayName(null, "com.ss.android.ugc.aweme")), "Douyin display name");
        check("闲鱼".equals(ImAppCatalog.appDisplayName(null, "com.taobao.idlefish")), "Idlefish display name");
        check("淘宝".equals(ImAppCatalog.appDisplayName(null, "com.taobao.taobao")), "Taobao display name");
        check("短信".equals(ImAppCatalog.appDisplayName(null, "com.android.mms")), "SMS display name");

        // 2. Conversation title cleaning & threadId normalization
        check("张三".equals(ImAppCatalog.cleanConversationTitle("com.tencent.mm", "[3条] 张三")), "Leading [3条]");
        check("产品研发群".equals(ImAppCatalog.cleanConversationTitle("com.tencent.mm", "产品研发群 (5)")), "Trailing (5)");
        check("研发群".equals(ImAppCatalog.cleanConversationTitle("com.tencent.mm", "研发群(5条新消息)")), "Trailing (5条新消息)");
        check("技术讨论组".equals(ImAppCatalog.cleanConversationTitle("com.alibaba.android.rimet", "【12条】技术讨论组 (12条)")), "Both prefix & suffix");
        check(
            ImAppCatalog.buildThreadId("com.tencent.mm", "[2条] 产品研发群")
                .equals(ImAppCatalog.buildThreadId("com.tencent.mm", "产品研发群 (8条新消息)")),
            "Same group with different unread counts must normalize to identical threadId"
        );

        // 3. Group sender & body splitting
        ImAppCatalog.SenderBody sb1 = ImAppCatalog.extractSenderAndBody(
            "com.tencent.mm", "产品研发群", "王敏: 今晚8点发版", true
        );
        check("王敏".equals(sb1.sender) && "今晚8点发版".equals(sb1.body), "Group sender split failed: " + sb1.sender + " / " + sb1.body);

        ImAppCatalog.SenderBody sb2 = ImAppCatalog.extractSenderAndBody(
            "com.tencent.mm", "产品群", "[4条] 产品群: 李四: 方案发我一下", true
        );
        check("李四".equals(sb2.sender) && "方案发我一下".equals(sb2.body), "Prefixed group sender split failed: " + sb2.sender + " / " + sb2.body);

        ImAppCatalog.SenderBody sbUrl = ImAppCatalog.extractSenderAndBody(
            "com.tencent.mm", "张三", "https://stepx.com/docs", false
        );
        check("张三".equals(sbUrl.sender) && "https://stepx.com/docs".equals(sbUrl.body), "URL must not split as sender");

        // 4. OTP & pickup code extraction + false-positive avoidance
        check("839201".equals(ImAppCatalog.extractOtpCode("招商银行", "【招商银行】验证码 839201，5分钟内有效。")), "Bank OTP");
        check("492018".equals(ImAppCatalog.extractOtpCode("GitHub", "492018 is your GitHub verification code.")), "English OTP");
        check("4-2-1089".equals(ImAppCatalog.extractOtpCode("菜鸟", "您的顺丰包裹已到达丰巢智能柜，取件码 4-2-1089")), "Cabinet pickup code");
        check(ImAppCatalog.extractOtpCode("账单提醒", "您尾号8899卡片于2026年消费1680元") == null, "Price/year/card tail must not trigger OTP");

        // 5. URL extraction
        check(
            "https://feishu.cn/docx/StepX2026".equals(
                ImAppCatalog.extractFirstUrl("请大家看下文档 https://feishu.cn/docx/StepX2026，今晚评审。")
            ),
            "URL extraction with trailing Chinese punctuation"
        );

        // 6. 6-category notification classification
        check(ImAppCatalog.CAT_IM.equals(ImAppCatalog.classifyNotification("com.tencent.mm", "msg", "张三", "你好", true)), "IM category");
        check(ImAppCatalog.CAT_FINANCE.equals(ImAppCatalog.classifyNotification("com.eg.android.AlipayGphone", "", "付款成功", "向瑞幸咖啡付款 16.50 元", false)), "Finance category");
        check(ImAppCatalog.CAT_DELIVERY_TRAVEL.equals(ImAppCatalog.classifyNotification("com.cainiao.wireless", "", "派送通知", "包裹已到菜鸟驿站，取件码 3-1-2044", false)), "Delivery category");
        check(ImAppCatalog.CAT_SOCIAL.equals(ImAppCatalog.classifyNotification("com.xingin.xhs", "social", "小红书", "有人赞了你的笔记", false)), "Social category");
        check(ImAppCatalog.CAT_SYSTEM.equals(ImAppCatalog.classifyNotification("com.tencent.mm", "service", "微信", "微信正在后台运行", false)), "WeChat background noise -> system");

        // 7. PushStore thread merging, throttling, handle soft-retention & clientMsgId idempotency
        PushStore store = new PushStore(null);
        final AtomicInteger sendCounter = new AtomicInteger(0);
        store.setReplyDispatcherForTest(new PushStore.RemoteReplyDispatcher() {
            @Override
            public void send(Context context, PushStore.RemoteReplyHandle handle, String text)
                    throws PendingIntent.CanceledException {
                sendCounter.incrementAndGet();
            }
        });

        String threadId = "com.tencent.mm::产品研发群";
        PushStore.RemoteReplyHandle handle = new PushStore.RemoteReplyHandle(
            null, null, null, "sbn_wx_1", "com.tencent.mm", "产品研发群", threadId, 1000L
        );

        PushStore.NotificationRecord n1 = store.ingestRecord(
            "sbn_wx_1", "com.tencent.mm", "微信", "msg",
            "[1条] 产品研发群", "", "王敏: 今晚8点发版", "",
            1000L, false, true, false, null, handle, null
        );
        check(n1 != null && threadId.equals(n1.threadId), "First group notification must bind threadId");
        PushStore.ThreadRecord thread = store.getThreadRecord(threadId);
        check(thread != null && thread.unreadCount == 1 && thread.canReply, "Thread must be unread=1 and canReply=true");

        // Ongoing notification throttling within 2 seconds
        PushStore.NotificationRecord ongoing1 = store.ingestRecord(
            "sbn_dl_1", "com.android.providers.downloads", "下载管理", "progress",
            "正在下载更新", "", "10%", "", 2000L, true, false, true, null, null, null
        );
        PushStore.NotificationRecord ongoing2 = store.ingestRecord(
            "sbn_dl_1", "com.android.providers.downloads", "下载管理", "progress",
            "正在下载更新", "", "25%", "", 2500L, true, false, true, null, null, null
        );
        check(ongoing1 != null && ongoing2 == null, "Progress update within 2000ms must be throttled");

        // Crucial same-Wi-Fi stability check: REASON_APP_CANCEL clears unreadCount but soft-retains RemoteReplyHandle
        store.removeRecord("sbn_wx_1", NotificationListenerService.REASON_APP_CANCEL, 3000L);
        PushStore.NotificationRecord removed = store.getNotificationRecordByKey("sbn_wx_1");
        check(removed != null && !removed.active && removed.read && removed.dismissed, "Removed notification must be marked read & dismissed");
        check(thread.unreadCount == 0, "Thread unreadCount must reset to 0 on REASON_APP_CANCEL");
        check(store.hasReplyHandle(threadId) && thread.canReply, "RemoteReplyHandle must be soft-retained after REASON_APP_CANCEL");

        // Idempotent remote reply via soft-retained handle
        PushStore.ReplyOutcome r1 = store.executeRemoteReplyOutcome(null, threadId, "收到，准时发版", "msg_client_001");
        check(!r1.deduplicated && sendCounter.get() == 1, "First reply must dispatch once");
        PushStore.ReplyOutcome r2 = store.executeRemoteReplyOutcome(null, threadId, "收到，准时发版", "msg_client_001");
        check(r2.deduplicated && sendCounter.get() == 1, "Duplicate clientMsgId retry must be suppressed");

        // 2nd and 3rd consecutive replies on the same handle must succeed without re-posting notification
        store.executeRemoteReplyOutcome(null, threadId, "补充一下，回归测试已通过", "msg_client_002");
        check(sendCounter.get() == 2 && thread.messages.size() == 3, "Consecutive reply on retained handle must succeed");

        // Ringbuffer capacity limit check (100 messages per thread)
        for (int i = 0; i < 120; i++) {
            store.ingestRecord(
                "sbn_wx_bulk_" + i, "com.tencent.mm", "微信", "msg",
                "产品研发群", "", "成员" + i + ": 批量消息 " + i, "",
                10_000L + i * 100L, false, true, false, null, handle, null
            );
        }
        check(thread.messages.size() == PushStore.MAX_MESSAGES_PER_THREAD, "Messages per thread must be capped at 100");
        check(!store.isUnchangedSnapshot(0L), "Full snapshot (sinceRevision=0) must have unchanged=false");
        check(store.isUnchangedSnapshot(store.getRevision()), "Delta snapshot at current revision must have unchanged=true");
        check(!store.isUnchangedSnapshot(store.getRevision() - 1), "Delta snapshot at stale revision must have unchanged=false");

        System.out.println("ImPushCatalogTest passed");
    }
}
