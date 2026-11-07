package com.stepx.companion.app;

import android.app.PendingIntent;
import android.content.Context;
import android.service.notification.NotificationListenerService;
import java.io.File;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.List;
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
        check("研发群".equals(ImAppCatalog.cleanConversationTitle("com.tencent.mm", "研发群 (48)")), "Trailing group count (48)");
        check("研发群".equals(ImAppCatalog.cleanConversationTitle("com.tencent.mm", "研发群 (48人)")), "Trailing group count (48人)");
        check("研发群".equals(ImAppCatalog.cleanConversationTitle("com.tencent.mm", "研发群(5条新消息)")), "Trailing (5条新消息)");
        check("技术讨论组".equals(ImAppCatalog.cleanConversationTitle("com.alibaba.android.rimet", "【12条】技术讨论组 (12条)")), "Both prefix & suffix");
        check(
            ImAppCatalog.buildThreadId("com.tencent.mm", "[2条] 产品研发群")
                .equals(ImAppCatalog.buildThreadId("com.tencent.mm", "产品研发群 (8条新消息)")),
            "Same group with different unread counts must normalize to identical threadId"
        );
        check(
            "项目组(一期)".equals(ImAppCatalog.cleanConversationTitle("com.tencent.mm", "项目组(一期) (48)")),
            "Group title ending with parentheses must preserve (一期) while stripping (48)"
        );
        check(
            "项目组(1)".equals(ImAppCatalog.cleanConversationTitle("com.tencent.mm", "项目组(1) (48)")),
            "Group title ending with (1) must preserve (1) when outer member count (48) is stripped"
        );
        check(
            "招商银行 (1069800095555)".equals(
                ImAppCatalog.cleanConversationTitle("com.android.mms", "招商银行 (1069800095555)")
            ),
            "Long service/phone numbers in parentheses must not be stripped as unread counts"
        );
        check(
            ChatUiBridge.matchesRecipientNodeText("com.tencent.mm", "研发群", "研发群 (48)"),
            "Group chat top bar with member count (48) must pass recipient verification"
        );
        check(
            ChatUiBridge.matchesRecipientNodeText("com.ss.android.lark", "研发群", "研发群（48人）"),
            "Group chat top bar with Chinese parentheses member count must pass recipient verification"
        );
        check(
            ChatUiBridge.matchesRecipientNodeText("com.tencent.mm", "项目组(一期)", "项目组(一期) (48)"),
            "Group chat with parenthesized suffix (一期) and member count (48) must pass verification"
        );
        check(
            ChatUiBridge.matchesRecipientNodeText("com.tencent.mm", "项目组(1)", "项目组(1) (48)"),
            "Group chat 项目组(1) must match top bar 项目组(1) (48)"
        );
        check(
            !ChatUiBridge.matchesRecipientNodeText("com.tencent.mm", "项目组(1)", "项目组(2)"),
            "Distinct numbered groups 项目组(1) and 项目组(2) must not false-match"
        );
        check(
            !ChatUiBridge.matchesRecipientNodeText("com.tencent.mm", "项目组(1)", "项目组(2) (48)"),
            "Group 项目组(1) must not false-match top bar of 项目组(2) (48)"
        );
        check(
            !ChatUiBridge.matchesRecipientNodeText("com.tencent.mm", "研发群", "联系人B"),
            "Different contact window must fail recipient verification"
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

        testGenericGroupAndSenderUpgrade();
        testParseWindowSnapshot();
        testConvertAbilityViewNode();
        testMcpToolsSchemaAndExpansionFilter();

        System.out.println("ImPushCatalogTest passed");
    }

    private static void testGenericGroupAndSenderUpgrade() throws Exception {
        check("张三".equals(ImAppCatalog.extractAvatarSenderName("张三头像")), "Avatar desc '张三头像' -> '张三'");
        check("敏".equals(ImAppCatalog.extractAvatarSenderName("敏的头像")), "Avatar desc '敏的头像' -> '敏'");
        check("".equals(ImAppCatalog.extractAvatarSenderName("头像")), "Generic avatar desc -> empty");
        check(ImAppCatalog.isGenericPlaceholderTitle("群聊"), "'群聊' must be generic title");
        check(ImAppCatalog.isGenericPlaceholderTitle("群聊(4) · 敏"), "'群聊(4) · 敏' must be generic title");
        check(!ImAppCatalog.isGenericPlaceholderTitle("StepX内测群"), "Real group name must not be generic");
        check(ImAppCatalog.isGenericSenderPlaceholder("微信用户"), "'微信用户' must be generic sender");
        check(!ImAppCatalog.isGenericSenderPlaceholder("敏"), "'敏' must not be generic sender");
        check(ImAppCatalog.hasGroupMemberCountSuffix("StepX内测群(4)"), "Group member count suffix must be detected");

        PushStore store = new PushStore(null);
        final AtomicInteger groupReplyCount = new AtomicInteger(0);
        store.setReplyDispatcherForTest(new PushStore.RemoteReplyDispatcher() {
            @Override
            public void send(Context context, PushStore.RemoteReplyHandle handle, String text)
                    throws PendingIntent.CanceledException {
                groupReplyCount.incrementAndGet();
            }
        });

        PushStore.RemoteReplyHandle maskedHandle = new PushStore.RemoteReplyHandle(
            null, null, null, "sbn_masked_2", "com.tencent.mm", "群聊", "com.tencent.mm::群聊", 2000L
        );

        // 1. Ingest privacy-masked WeChat group notifications ("群聊" / "微信用户: ...")
        store.ingestRecord(
            "sbn_masked_1", "com.tencent.mm", "微信", "msg",
            "群聊", "", "微信用户: Hi everyone", "",
            1000L, false, true, false, null, null, null
        );
        store.ingestRecord(
            "sbn_masked_2", "com.tencent.mm", "微信", "msg",
            "群聊", "", "微信用户: 什么概念", "",
            2000L, false, true, false, null, maskedHandle, null
        );

        PushStore.ThreadRecord before = store.getThreadRecord("com.tencent.mm::群聊");
        check(before != null && before.messages.size() == 2, "Generic '群聊' thread must have 2 messages initially");
        check("微信用户".equals(before.messages.get(0).sender), "Initial sender is '微信用户'");

        // 2. Merge captured chat window with real group title "StepX内测群(4)" and real senders
        List<PushStore.CapturedBubble> bubbles = new ArrayList<PushStore.CapturedBubble>();
        bubbles.add(new PushStore.CapturedBubble("敏", "Hi everyone", false, 0L));
        bubbles.add(new PushStore.CapturedBubble("张三", "什么概念", false, 0L));
        bubbles.add(new PushStore.CapturedBubble("我", "hi", true, 0L));

        store.mergeCapturedWindowMessagesOutcome(
            "com.tencent.mm", "微信", "StepX内测群(4)", bubbles, true, "群聊"
        );

        PushStore.ThreadRecord upgraded = store.getThreadRecord("com.tencent.mm::StepX内测群");
        check(upgraded != null, "Generic thread must be migrated to 'com.tencent.mm::StepX内测群'");
        check(store.getThreadRecord("com.tencent.mm::群聊") == upgraded, "Old generic threadId must alias to upgraded thread");
        check(store.getThreadRecord("com.tencent.mm::StepX内测群(4)") == upgraded, "Raw group title with suffix must resolve to upgraded thread");
        check("StepX内测群".equals(upgraded.title), "Upgraded thread title must be 'StepX内测群'");
        check(upgraded.messages.size() == 3, "Existing messages must be upgraded in-place without duplication");
        check("敏".equals(upgraded.messages.get(0).sender), "Message 0 sender must be upgraded to '敏'");
        check("张三".equals(upgraded.messages.get(1).sender), "Message 1 sender must be upgraded to '张三'");
        check("我".equals(upgraded.messages.get(2).sender) && upgraded.messages.get(2).outgoing, "Message 2 must be outgoing '我'");

        PushStore.NotificationRecord notif1 = store.getNotificationRecordByKey("sbn_masked_1");
        PushStore.NotificationRecord notif2 = store.getNotificationRecordByKey("sbn_masked_2");
        check(notif1 != null && "StepX内测群".equals(notif1.title) && "敏".equals(notif1.sender)
            && "敏: Hi everyone".equals(notif1.text), "Notification 1 must be upgraded to real title and sender");
        check(notif2 != null && "StepX内测群".equals(notif2.title) && "张三".equals(notif2.sender)
            && "张三: 什么概念".equals(notif2.text), "Notification 2 must be upgraded to real title and sender");

        // 3. Replying via the old generic threadId ("com.tencent.mm::群聊") or new threadId ("com.tencent.mm::StepX内测群")
        // must find the migrated RemoteReplyHandle and append the outgoing message to "com.tencent.mm::StepX内测群"
        // WITHOUT recreating a split "com.tencent.mm::群聊" thread!
        check(store.hasReplyHandle("com.tencent.mm::群聊"), "hasReplyHandle must resolve old generic alias");
        check(store.hasReplyHandle("com.tencent.mm::StepX内测群"), "hasReplyHandle must resolve upgraded threadId");
        PushStore.ReplyOutcome replyViaAlias = store.executeRemoteReplyOutcome(
            null, "com.tencent.mm::群聊", "锁屏直接回复群聊", "client_alias_reply_1"
        );
        check(groupReplyCount.get() == 1 && !replyViaAlias.deduplicated, "Remote reply via generic alias must dispatch");
        check("com.tencent.mm::StepX内测群".equals(replyViaAlias.threadId),
            "ReplyOutcome must return canonical upgraded threadId, got " + replyViaAlias.threadId);
        check(upgraded.messages.size() == 4 && "锁屏直接回复群聊".equals(upgraded.messages.get(3).text),
            "Outgoing reply must be appended to upgraded thread without splitting");

        // 4. Stale notification rescan must NOT overwrite outgoing '我: ...' preview or inflate unreadCount
        store.ingestRecord(
            "sbn_masked_2", "com.tencent.mm", "微信", "msg",
            "StepX内测群", "", "张三: 什么概念", "",
            2000L, false, true, false, null, maskedHandle, null
        );
        check("我".equals(upgraded.lastSender) && "我: 锁屏直接回复群聊".equals(upgraded.lastMessage),
            "Stale notification rescan must not clobber outgoing lastMessage, got: " + upgraded.lastMessage);
        check(upgraded.unreadCount == 0, "Stale notification rescan must not increment unreadCount");

        // 5. Mark unread / read, delete single message, clear messages, delete thread & prune deleted phone rows
        store.markThreadReadRecord(null, upgraded.threadId, "com.tencent.mm", "StepX内测群", false);
        check(upgraded.unreadCount == 1, "markThreadReadRecord(false) must set unreadCount=1");
        store.markThreadReadRecord(null, upgraded.threadId, "com.tencent.mm", "StepX内测群", true);
        check(upgraded.unreadCount == 0, "markThreadReadRecord(true) must reset unreadCount=0");

        store.deleteMessageRecord(upgraded.threadId, "", "什么概念");
        check(upgraded.messages.size() == 3, "deleteMessageRecord must remove the matching single message");

        store.clearThreadMessagesRecord(upgraded.threadId, "com.tencent.mm", "StepX内测群");
        check(upgraded.messages.isEmpty() && "".equals(upgraded.lastMessage),
            "clearThreadMessagesRecord must empty thread messages while keeping thread in store");

        store.deleteThreadRecord(null, upgraded.threadId, "com.tencent.mm", "StepX内测群");
        check(store.getThreadRecord(upgraded.threadId) == null, "deleteThreadRecord must remove thread from store");

        // Stale notification at <= deletedAtMs must not resurrect deleted thread
        store.ingestRecord(
            "sbn_masked_2", "com.tencent.mm", "微信", "msg",
            "StepX内测群", "", "张三: 旧通知", "",
            2000L, false, true, false, null, maskedHandle, null
        );
        check(store.getThreadRecord("com.tencent.mm::StepX内测群") == null,
            "Stale notification with postTimeMs <= deletedAtMs must not resurrect deleted thread");

        // Window capture on a brand-new thread (without prior notifications) must set lastTimeMs > 0L
        List<PushStore.CapturedBubble> fhBubbles = new ArrayList<PushStore.CapturedBubble>();
        fhBubbles.add(new PushStore.CapturedBubble("我", "桌面端新发消息", true, 0L));
        store.mergeCapturedWindowMessagesOutcome(
            "com.tencent.mm", "微信", "文件传输助手", fhBubbles, false, ""
        );
        PushStore.ThreadRecord fhThread = store.getThreadRecord("com.tencent.mm::文件传输助手");
        check(fhThread != null && fhThread.lastTimeMs > 0L,
            "mergeCapturedWindowMessagesOutcome must update thread.lastTimeMs > 0L even when bubbles have timestampMs=0");

        // Seed an old thread that is absent from phone's LauncherUI conversation list and verify pruning when validRowCount >= 3
        store.ingestRecord(
            "sbn_stale_contact", "com.tencent.mm", "微信", "msg",
            "已删除旧好友", "", "旧消息", "",
            1000L, false, true, false, null, null, null
        );
        check(store.getThreadRecord("com.tencent.mm::已删除旧好友") != null, "Old contact thread must exist before list sync");
        List<PushStore.CapturedConversationRow> launcherRows = new ArrayList<PushStore.CapturedConversationRow>();
        launcherRows.add(new PushStore.CapturedConversationRow("文件传输助手", "我: 桌面端新发消息", "我", 0));
        launcherRows.add(new PushStore.CapturedConversationRow("产品研发群(8)", "敏: 收到", "敏", 1));
        launcherRows.add(new PushStore.CapturedConversationRow("设计评审群(5)", "李四: OK", "李四", 0));
        store.mergeCapturedConversationList("com.tencent.mm", "微信", launcherRows);
        check(store.getThreadRecord("com.tencent.mm::已删除旧好友") == null,
            "mergeCapturedConversationList with >= 3 visible rows must prune absent old thread");

        // Verify 1-row top-of-list conversation list sync also prunes deleted threads that are not recent
        store.ingestRecord(
            "sbn_stale_single", "com.ss.android.lark", "飞书", "msg",
            "已删飞书会话", "", "旧消息", "",
            1000L, false, false, false, null, null, null
        );
        check(store.getThreadRecord("com.ss.android.lark::已删飞书会话") != null, "Feishu stale thread must exist before 1-row sync");
        List<PushStore.CapturedConversationRow> singleRow = new ArrayList<PushStore.CapturedConversationRow>();
        singleRow.add(new PushStore.CapturedConversationRow("飞书助手", "欢迎使用飞书", "飞书助手", 0));
        store.mergeCapturedConversationList("com.ss.android.lark", "飞书", singleRow, true);
        check(store.getThreadRecord("com.ss.android.lark::已删飞书会话") == null,
            "mergeCapturedConversationList atTopOfList=true with 1 visible row must prune absent stale thread");

        // Verify native SMS package detection and exclusion from IM threads while keeping push notification & OTP
        check(ImAppCatalog.isNativeSmsPackage("com.android.mms"), "com.android.mms must be recognized as native SMS package");
        check(ImAppCatalog.isNativeSmsPackage("com.google.android.apps.messaging"), "Google Messages must be recognized as native SMS package");
        check(!ImAppCatalog.isNativeSmsPackage("com.tencent.mm"), "WeChat must not be recognized as native SMS package");
        long nowSmsMs = System.currentTimeMillis();
        PushStore.NotificationRecord smsNotif = store.ingestRecord(
            "sbn_sms_otp", "com.android.mms", "短信", "msg",
            "1069800095555", "", "【丰巢】凭取件码79983898至丰巢柜取件", "",
            nowSmsMs, false, false, false, null, null, null
        );
        check(smsNotif != null && "79983898".equals(smsNotif.otpCode), "Native SMS notification must extract OTP code");
        check(smsNotif.threadId == null, "Native SMS notification must not assign an IM threadId");
        check(store.getThreadRecord("com.android.mms::1069800095555") == null,
            "Native SMS notification must not create an IM thread in PushStore");
        int reconciled = store.reconcileActiveNotifications(new java.util.HashSet<String>(), nowSmsMs + 1000L);
        check(reconciled >= 1, "reconcileActiveNotifications with empty activeKeys must mark active notifications dismissed/read");
        check(!smsNotif.active && smsNotif.dismissed && smsNotif.read,
            "Reconciled SMS notification must be inactive, dismissed, and read");

        // Verify validRowCount >= 8 atTopOfList=true preserves untimed thread updated < 60s ago and prunes > 60s old untimed thread
        List<PushStore.CapturedConversationRow> seedUiRows = new ArrayList<PushStore.CapturedConversationRow>();
        seedUiRows.add(new PushStore.CapturedConversationRow("近期UI会话", "刚看到的消息", "近期UI会话", 0));
        seedUiRows.add(new PushStore.CapturedConversationRow("超期UI会话", "很久以前的消息", "超期UI会话", 0));
        store.mergeCapturedConversationList("com.tencent.mm", "微信", seedUiRows, false);
        PushStore.ThreadRecord recentUiThread = store.getThreadRecord("com.tencent.mm::近期UI会话");
        PushStore.ThreadRecord expiredUiThread = store.getThreadRecord("com.tencent.mm::超期UI会话");
        check(recentUiThread != null && expiredUiThread != null, "Both UI-captured threads must exist initially");
        long nowUiMs = System.currentTimeMillis();
        if (!recentUiThread.messages.isEmpty()) {
            recentUiThread.messages.get(0).observedAtMs = nowUiMs - 30_000L; // 30s ago (< 60s)
        }
        if (!expiredUiThread.messages.isEmpty()) {
            expiredUiThread.messages.get(0).observedAtMs = nowUiMs - 90_000L; // 90s ago (> 60s)
        }
        List<PushStore.CapturedConversationRow> eightRows = new ArrayList<PushStore.CapturedConversationRow>();
        for (int i = 1; i <= 8; i++) {
            eightRows.add(new PushStore.CapturedConversationRow("置顶联系人" + i, "消息" + i, "置顶联系人" + i, 0));
        }
        store.mergeCapturedConversationList("com.tencent.mm", "微信", eightRows, true);
        check(store.getThreadRecord("com.tencent.mm::近期UI会话") != null,
            "Untimed thread updated 30s ago (< 60s) must be preserved when validRowCount >= 8 atTopOfList=true");
        check(store.getThreadRecord("com.tencent.mm::超期UI会话") == null,
            "Untimed thread updated 90s ago (> 60s) must be pruned when validRowCount >= 8 atTopOfList=true");
    }

    private static void testParseWindowSnapshot() {
        ChatUiBridge.UiBounds rootBounds = new ChatUiBridge.UiBounds(0, 0, 1080, 2400);
        ChatUiBridge.UiNode root = new ChatUiBridge.UiNode(
            "", "", "", "android.widget.FrameLayout", "", rootBounds, false, false, false, true, 0, null
        );
        // Top-bar group title
        root.children.add(new ChatUiBridge.UiNode(
            "研发讨论群(6)", "", "", "android.widget.TextView", "",
            new ChatUiBridge.UiBounds(220, 90, 680, 160), false, false, false, true, 1, null
        ));
        // Left-side avatar for incoming message
        root.children.add(new ChatUiBridge.UiNode(
            "", "敏头像", "", "android.widget.ImageView", "",
            new ChatUiBridge.UiBounds(30, 400, 130, 500), true, false, false, true, 1, null
        ));
        // Nickname header right above incoming bubble
        root.children.add(new ChatUiBridge.UiNode(
            "敏", "", "", "android.widget.TextView", "",
            new ChatUiBridge.UiBounds(150, 395, 260, 435), false, false, false, true, 1, null
        ));
        // Incoming message bubble
        root.children.add(new ChatUiBridge.UiNode(
            "Hi everyone", "", "", "android.widget.TextView", "",
            new ChatUiBridge.UiBounds(150, 445, 520, 530), true, false, false, true, 1, null
        ));
        // Right-side outgoing message bubble
        root.children.add(new ChatUiBridge.UiNode(
            "收到", "", "", "android.widget.TextView", "",
            new ChatUiBridge.UiBounds(680, 600, 920, 680), true, false, false, true, 1, null
        ));
        // Long multi-line right-aligned outgoing message bubble (left=180, right=920 -> centerX=550 <= 1080*0.54, right >= 1080*0.74)
        root.children.add(new ChatUiBridge.UiNode(
            "这是一条跨越多行的长回复消息，左侧延伸较宽但右侧紧贴自己头像区域", "", "", "android.widget.TextView", "",
            new ChatUiBridge.UiBounds(180, 720, 920, 840), true, false, false, true, 1, null
        ));
        // Bottom chat EditText
        root.children.add(new ChatUiBridge.UiNode(
            "", "", "", "android.widget.EditText", "",
            new ChatUiBridge.UiBounds(140, 2150, 860, 2260), true, true, false, true, 1, null
        ));

        ChatUiBridge.UiWindowSnapshot snap = new ChatUiBridge.UiWindowSnapshot(
            "com.tencent.mm", "", 1080, 2400, root, "test", true
        );
        ChatUiBridge.ParsedChatWindow parsed = ChatUiBridge.parseWindowSnapshot(snap);
        check(!parsed.isConversationList, "Must be recognized as chat window, not conversation list");
        check("研发讨论群(6)".equals(parsed.rawTitle) && "研发讨论群".equals(parsed.cleanTitle), "Must extract group title");
        check(parsed.isGroup, "Must be marked as group window");
        check(parsed.bubbles.size() == 3, "Nickname header '敏' must not be emitted as a separate bubble (got " + parsed.bubbles.size() + ")");
        check("敏".equals(parsed.bubbles.get(0).sender) && "Hi everyone".equals(parsed.bubbles.get(0).text) && !parsed.bubbles.get(0).outgoing,
            "Incoming bubble must be attributed to '敏'");
        check("我".equals(parsed.bubbles.get(1).sender) && "收到".equals(parsed.bubbles.get(1).text) && parsed.bubbles.get(1).outgoing,
            "Outgoing bubble must be attributed to '我'");
        check("我".equals(parsed.bubbles.get(2).sender) && parsed.bubbles.get(2).outgoing,
            "Long right-aligned outgoing bubble must be attributed to '我'");
    }

    public static final class FakeAbilityRect {
        private final int left;
        private final int top;
        private final int right;
        private final int bottom;

        public FakeAbilityRect(int left, int top, int right, int bottom) {
            this.left = left;
            this.top = top;
            this.right = right;
            this.bottom = bottom;
        }

        public int getLeft() { return left; }
        public int getTop() { return top; }
        public int getRight() { return right; }
        public int getBottom() { return bottom; }
    }

    public static final class FakeAbilityViewNode {
        private final String text;
        private final String desc;
        private final String className;
        private final FakeAbilityRect bounds;
        private final boolean clickable;
        private final boolean textEditor;
        private final List<FakeAbilityViewNode> children = new ArrayList<FakeAbilityViewNode>();

        public FakeAbilityViewNode(
                String text,
                String desc,
                String className,
                FakeAbilityRect bounds,
                boolean clickable,
                boolean textEditor) {
            this.text = text;
            this.desc = desc;
            this.className = className;
            this.bounds = bounds;
            this.clickable = clickable;
            this.textEditor = textEditor;
        }

        public boolean isVisible() { return true; }
        public String getText() { return text; }
        public String getContentDescription() { return desc; }
        public String getHint() { return ""; }
        public String getClassName() { return className; }
        public String getViewId() { return "id/test"; }
        public Object getBounds() { return bounds; }
        public boolean isClickable() { return clickable; }
        public boolean isTextEditor() { return textEditor; }
        public boolean isVerticalScrollable() { return false; }
        public List<FakeAbilityViewNode> getChildren() { return children; }
    }

    private static void testConvertAbilityViewNode() {
        FakeAbilityViewNode rootVn = new FakeAbilityViewNode(
            "", "", "android.widget.FrameLayout", new FakeAbilityRect(0, 0, 1264, 2800), false, false
        );
        FakeAbilityViewNode childVn = new FakeAbilityViewNode(
            "StepX内测群(4)", "群聊标题", "com.tencent.mm.ui.base.NoMeasuredTextView",
            new FakeAbilityRect(180, 96, 640, 168), true, false
        );
        rootVn.getChildren().add(childVn);

        ChatUiBridge.UiNode converted = ChatUiBridge.convertAbilityViewNode(rootVn, 0);
        check(converted != null, "convertAbilityViewNode must convert synthetic ability.apk ViewNode");
        check(converted.bounds.width() == 1264 && converted.bounds.height() == 2800,
            "Custom com.mdisdk.capability.model.Rect bounds must be extracted via getLeft/getTop/getRight/getBottom");
        check(converted.children.size() == 1, "Child ViewNode must be converted");
        ChatUiBridge.UiNode child = converted.children.get(0);
        check("StepX内测群(4)".equals(child.text) && "群聊标题".equals(child.desc) && child.clickable,
            "Child text, desc, and clickable flags must be preserved");
        check(child.bounds.left == 180 && child.bounds.top == 96 && child.bounds.right == 640 && child.bounds.bottom == 168,
            "Child custom Rect bounds must match");
    }

    private static void testMcpToolsSchemaAndExpansionFilter() throws Exception {
        java.nio.file.Path schemaPath = java.nio.file.Paths.get("desktop/companion-app/res/raw/stepx_mcp_tools.json");
        if (!java.nio.file.Files.exists(schemaPath)) {
            schemaPath = java.nio.file.Paths.get("res/raw/stepx_mcp_tools.json");
        }
        String rawSchema = new String(java.nio.file.Files.readAllBytes(schemaPath), java.nio.charset.StandardCharsets.UTF_8);
        check(rawSchema.contains("\"name\": \"stepx_gui_control\""), "stepx_mcp_tools.json must define stepx_gui_control");
        check(rawSchema.contains("\"launch\""), "stepx_gui_control action.type enum must include launch");
        check(rawSchema.contains("\"package\""), "stepx_gui_control action.properties must include package");
        check(rawSchema.contains("\"name\": \"stepx_expansion\""), "stepx_mcp_tools.json must define stepx_expansion");
        check(rawSchema.contains("\"status\"") && rawSchema.contains("\"list\"") && rawSchema.contains("\"get\"") && rawSchema.contains("\"request\""),
            "stepx_expansion op enum must include status, list, get, request");

        check(ExpansionRules.matchesQuery("财报", "0123456789abcdef0123456789abcdef", "2026_Q3_财报.pdf", "Documents/2026_Q3_财报.pdf", "documents", "archived"),
            "ExpansionRules.matchesQuery must match filename query");
        check(!ExpansionRules.matchesQuery("财报", "fedcba9876543210fedcba9876543210", "IMG_20261024.jpg", "DCIM/Camera/IMG_20261024.jpg", "photos", "archived"),
            "ExpansionRules.matchesQuery must reject non-matching items");
        check(ExpansionRules.matchesQuery("", "fedcba9876543210fedcba9876543210", "IMG_20261024.jpg", "DCIM/Camera/IMG_20261024.jpg", "photos", "archived"),
            "ExpansionRules.matchesQuery must match all items when query is empty");
        check(ExpansionRules.normalizeLimit(0) == 20 && ExpansionRules.normalizeLimit(500) == 100 && ExpansionRules.normalizeLimit(5) == 5,
            "ExpansionRules.normalizeLimit must clamp limits to [1, 100] with default 20");

        // Instant grocery, coffee, and recruitment app catalog + ongoing capsule tests
        check("叮咚买菜".equals(ImAppCatalog.appDisplayName(null, "com.yaya.zone")), "Dingdong Maicai display name");
        check("盒马".equals(ImAppCatalog.appDisplayName(null, "com.wudaokou.hippo")), "Hema display name");
        check("山姆会员商店".equals(ImAppCatalog.appDisplayName(null, "cn.samsclub.app")), "Sam's Club display name");
        check("BOSS直聘".equals(ImAppCatalog.appDisplayName(null, "com.hpbr.bosszhipin")), "BOSS Zhipin display name");
        check(
            ImAppCatalog.CAT_DELIVERY_TRAVEL.equals(
                ImAppCatalog.classifyNotification(
                    "com.yaya.zone",
                    null,
                    "订单已送达",
                    "【叮咚买菜】您在叮咚买菜购买的商品已代收（家门口），请及时取走。",
                    false
                )
            ),
            "Dingdong Maicai doorstep arrival must classify as delivery_travel"
        );
        check(
            ImAppCatalog.CAT_IM.equals(
                ImAppCatalog.classifyNotification(
                    "com.hpbr.bosszhipin",
                    "msg",
                    "张女士 · 字节跳动",
                    "您好，看到您的简历非常匹配，请问近期方便沟通吗？",
                    true
                )
            ),
            "BOSS Zhipin recruitment message must classify as im"
        );

        PushStore capsuleStore = new PushStore(null);
        PushStore.NotificationRecord ongoingCapsule = capsuleStore.ingestRecord(
            "key-meituan-ongoing",
            "com.sankuai.meituan",
            "美团",
            "transport",
            "骑手正在配送",
            null,
            "距离您 320m，预计 3 分钟送达",
            "",
            1700000100000L,
            true,
            false,
            true,
            null,
            null,
            null,
            "",
            75,
            100,
            false
        );
        check(ongoingCapsule != null, "Ongoing capsule must be ingested");
        check(ongoingCapsule.active, "Ongoing live capsule must keep active=true");
        check(ongoingCapsule.ongoing, "Ongoing live capsule must keep ongoing=true");
        check(ongoingCapsule.progressCurrent == 75 && ongoingCapsule.progressMax == 100,
            "Ongoing live capsule must store progressCurrent and progressMax");
    }
}
