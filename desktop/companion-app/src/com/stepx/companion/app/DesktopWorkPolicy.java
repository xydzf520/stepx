package com.stepx.companion.app;

/** Phone-side task presentation must follow the task's own computer and receipts. */
final class DesktopWorkPolicy {
    private DesktopWorkPolicy() {}

    static boolean sameDesktopOnline(String taskDesktop, String connectedDesktop, boolean online) {
        return online && taskDesktop != null && !taskDesktop.isEmpty()
            && taskDesktop.equals(connectedDesktop);
    }

    static boolean active(String phase) {
        return "queued".equals(phase) || "running".equals(phase)
            || "awaiting_delivery".equals(phase) || "cancelling".equals(phase);
    }

    static String phaseLabel(String phase) {
        switch (phase) {
            case "queued": return "等待电脑";
            case "running": return "Amoo 正在处理";
            case "cancelling": return "正在停止";
            case "failed": return "处理失败";
            case "awaiting_delivery": return "成果正在返回";
            case "completed": return "已送达手机";
            case "cancelled": return "已取消";
            case "interrupted": return "处理被中断";
            default: return "需要查看";
        }
    }

    static String connectionHint(String phase, boolean targetOnline, boolean differentDesktop) {
        if (!active(phase) || targetOnline) return "";
        return differentDesktop ? "任务所在电脑未连接；当前连接的是另一台电脑"
            : "任务所在电脑离线，恢复连接后更新进度";
    }
}
