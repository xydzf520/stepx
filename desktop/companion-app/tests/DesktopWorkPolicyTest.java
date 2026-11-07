package com.stepx.companion.app;

public final class DesktopWorkPolicyTest {
    private static void check(boolean value) { if (!value) throw new AssertionError(); }
    public static void main(String[] args) {
        check(DesktopWorkPolicy.sameDesktopOnline("computer-a", "computer-a", true));
        check(!DesktopWorkPolicy.sameDesktopOnline("computer-a", "computer-b", true));
        check(!DesktopWorkPolicy.sameDesktopOnline("computer-a", "computer-a", false));
        check(!DesktopWorkPolicy.sameDesktopOnline("", "", true));
        check(!DesktopWorkPolicy.sameDesktopOnline(null, "computer-a", true));
        check(DesktopWorkPolicy.connectionHint("running", false, true).contains("另一台电脑"));
        check(DesktopWorkPolicy.connectionHint("queued", false, false).contains("恢复连接"));
        check(DesktopWorkPolicy.connectionHint("running", true, false).isEmpty());
        // Delivered files remain readable locally; don't ask users to reconnect for them.
        check(DesktopWorkPolicy.connectionHint("completed", false, false).isEmpty());
        check(DesktopWorkPolicy.connectionHint("failed", false, true).isEmpty());
        check(DesktopWorkPolicy.active("cancelling"));
        check(!DesktopWorkPolicy.active("interrupted"));
        System.out.println("Desktop work continuity policy passed");
    }
}
