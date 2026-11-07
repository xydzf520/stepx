package com.stepx.companion.app;

public final class ContextProvenanceTest {
    private static void check(boolean ok) { if (!ok) throw new AssertionError(); }
    public static void main(String[] args) {
        check(!ContextProvenance.timeKnown("ui_bridge", false, false));
        check(!ContextProvenance.timeKnown("ui_capture", false, false));
        check(ContextProvenance.timeKnown("notification", false, false));
        check(ContextProvenance.timeKnown("ui_bridge", true, true));
        check(!ContextProvenance.timeKnown("notification", true, false));
        String first = ContextProvenance.captureId("我", "周五交方案", true, 0);
        check(first.equals(ContextProvenance.captureId("我", "周五交方案", true, 0)));
        check(!first.equals(ContextProvenance.captureId("我", "周五交方案", true, 1)));
        check(!first.equals(ContextProvenance.captureId("我", "周五交方案", false, 0)));
        check(!ContextProvenance.captureId("ab", "c", false, 0).equals(ContextProvenance.captureId("a", "bc", false, 0)));
        check(first.length() == 72);
        System.out.println("ContextProvenanceTest: 10 assertions passed");
    }
}
