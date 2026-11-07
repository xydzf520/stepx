package com.stepx.context.verification;

import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.os.Bundle;
import android.widget.TextView;

/** Temporary real Android notification source. No network, accounts or personal data access. */
public final class MainActivity extends Activity {
    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        String marker = getIntent().getStringExtra("marker");
        if (marker == null || !marker.matches("[A-Za-z0-9-]{8,80}")) {
            finish(); return;
        }
        String scenario = getIntent().getStringExtra("scenario");
        if (scenario == null) scenario = "legacy";
        String title;
        String body;
        int notificationId;
        if ("changes".equals(scenario)) {
            title = "[项目: " + marker + "] 读书会安排更新（验收资料）";
            body = "[项目: " + marker + "] 仅用于验收：12人读书会预算上限由600元改为480元；活动时间由2026年10月31日14:00改为2026年11月1日14:00；场地询价负责人由我改为小林，手机报名入口负责人由小林改为我。场地报价仍未知，尚未订场地；是否收报名费待决定。请以本通知的更新为准，保留纸质签到。";
            notificationId = 2;
        } else if ("newsletter".equals(scenario)) {
            // Deliberately no project marker: this should not become a task or decision.
            title = marker + " Newsletter";
            body = marker + " 资讯测试：假想城市今日云量较多，咖啡知识文章已更新。每日资讯订阅，点击退订。";
            notificationId = 3;
        } else if ("legacy".equals(scenario)) {
            title = "[项目: " + marker + "] 读书会变更（验收资料）";
            body = "[项目: " + marker + "] 验收测试通知：读书会预算上限由600元改为480元，参与者仍为12人；活动日期和场地报价仍待确认。方案请保留手机报名入口与纸质签到，不要把尚未确认的日期写成已安排。";
            notificationId = 1;
        } else { finish(); return; }
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel("verification", "上下文闭环验收", NotificationManager.IMPORTANCE_DEFAULT));
        manager.notify(marker, notificationId, new Notification.Builder(this, "verification")
            .setSmallIcon(android.R.drawable.ic_dialog_info).setContentTitle(title).setContentText(body)
            .setStyle(new Notification.BigTextStyle().bigText(body)).setCategory(Notification.CATEGORY_EVENT)
            .setOngoing(false).build());
        TextView view = new TextView(this);
        view.setTextSize(20); view.setPadding(28, 60, 28, 28);
        view.setText("StepX 跨 App 闭环验收\n\n以下为测试资料，未读取个人数据：\n\n" + body);
        setContentView(view);
    }
}
