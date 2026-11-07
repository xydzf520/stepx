package com.stepx.context.verification;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Person;
import android.app.RemoteInput;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

/** Shell-only, local simulation. Never opens an Activity, uses a wake lock or contacts a server. */
public final class ScreenOffReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        String marker = intent.getStringExtra("marker");
        int sequence = intent.getIntExtra("sequence", 0);
        if (marker == null || !marker.matches("STEPX-VERIFY-[A-Za-z0-9-]{1,60}")
                || sequence < 1 || sequence > 99) { setResultCode(1); return; }
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if ("clear".equals(intent.getStringExtra("op"))) {
            manager.cancel(marker, 100 + sequence);
            setResultCode(0); return;
        }
        // A separate channel leaves the existing interactive fixtures untouched.
        // MIN plus silent avoids sound, heads-up, vibration and ambient pulses.
        NotificationChannel channel = new NotificationChannel("screen-off-verification",
            "熄屏同步模拟验收（静默）", NotificationManager.IMPORTANCE_MIN);
        channel.setSound(null, null); channel.enableVibration(false); channel.enableLights(false);
        manager.createNotificationChannel(channel);
        String body = marker + " · 序号 " + sequence
            + "：这是本机生成的同步测试通知，不是微信消息，也不代表真实聊天内容。";
        Notification.Builder builder = new Notification.Builder(context, channel.getId())
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("熄屏同步模拟验收 · " + sequence).setContentText(body)
            .setStyle(new Notification.BigTextStyle().bigText(body))
            .setCategory(Notification.CATEGORY_EVENT).setOnlyAlertOnce(true)
            .setGroup("stepx-verification-silent").setGroupAlertBehavior(Notification.GROUP_ALERT_SUMMARY).setVisibility(Notification.VISIBILITY_SECRET);
        if (intent.getBooleanExtra("reply", false)) {
            Intent reply = new Intent(context, ReplyReceiver.class)
                .setAction(marker + ":" + sequence).putExtra("marker", marker).putExtra("sequence", sequence);
            PendingIntent pending = PendingIntent.getBroadcast(context, sequence, reply,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_MUTABLE);
            RemoteInput input = new RemoteInput.Builder("fixture_reply").setLabel("仅本机回复验收").build();
            builder.setCategory(Notification.CATEGORY_MESSAGE)
                .setStyle(new Notification.MessagingStyle(new Person.Builder().setName("本机验收").build())
                    .setConversationTitle(marker).addMessage(body, System.currentTimeMillis(),
                        new Person.Builder().setName("本机通知源").build()))
                .addAction(new Notification.Action.Builder(android.R.drawable.ic_menu_send, "本机回复", pending)
                    .addRemoteInput(input).setSemanticAction(Notification.Action.SEMANTIC_ACTION_REPLY).build());
        }
        manager.notify(marker, 100 + sequence, builder.build());
        setResultCode(0);
    }
}
