package com.stepx.companion.neo;
public final class BridgePolicyTest {
    static void check(boolean value) { if (!value) throw new AssertionError(); }
    public static void main(String[] args) {
        check("核验".equals(CompanionOverlayPolicy.action("finish")));
        check("观察".equals(CompanionOverlayPolicy.action("unrecognized-action")));
        check("暂停".equals(CompanionOverlayPolicy.phase(true,"正在点击")));
        check("观察".equals(CompanionOverlayPolicy.phase(false,"滑动已执行")));
        check(BridgePolicy.hostControls(5000,4999,"com.stepx.companion.app"));
        check(!BridgePolicy.hostControls(5000,5000,"com.stepx.companion.app"));
        check(!BridgePolicy.hostControls(5000,4999,"com.android.settings"));
        check(!BridgePolicy.hostControls(5000,4999,null));
        check(!BridgePolicy.hostControls(0,100,"com.stepx.companion.app"));
        check(BridgePolicy.caller(2000, 10200, null, false));
        check(BridgePolicy.caller(10200, 10200, null, false));
        check(BridgePolicy.caller(10201, 10200, new String[]{"com.stepx.companion.app"}, true));
        check(!BridgePolicy.caller(10201, 10200, new String[]{"com.stepx.companion.app"}, false));
        check(!BridgePolicy.caller(10201, 10200, new String[]{"untrusted.app"}, true));
        check(!BridgePolicy.caller(10201, 10200, new String[]{"com.stepx.companion.app", "extra.shared.uid"}, true));
        check(!BridgePolicy.caller(0, 10200, null, true));
        check(BridgePolicy.requestId("task-123:jd"));
        check(!BridgePolicy.requestId("task\n123"));
        check(!BridgePolicy.requestId(null));
        check(!BridgePolicy.requestId(""));
        check(!BridgePolicy.requestId(new String(new char[129]).replace('\0', 'a')));
        check("OpenAI".equals(BridgePolicy.protocol("openai")));
        check("Anthropic".equals(BridgePolicy.protocol("ANTHROPIC")));
        check("Gemini".equals(BridgePolicy.protocol("Gemini")));
        check(BridgePolicy.protocol("unknown") == null);
        check(BridgePolicy.protocol(null) == null);
        check("prefix [credential hidden] suffix".equals(BridgePolicy.diagnosticText("prefix arbitrary-secret suffix", "arbitrary-secret", 3000)));
        check(!BridgePolicy.diagnosticText("prefix arbitrary-secret suffix", "arbitrary-secret", 18).contains("secret"));
        check(!BridgePolicy.diagnosticText("sk-abcdefghijklmnop Bearer token123", "", 3000).contains("token123"));
        check(!BridgePolicy.diagnosticText("sk-abcdefghijklmnop", "", 3000).contains("abcdefghijklmnop"));
        check("456".equals(BridgePolicy.diagnosticText("123456", "", 3)));
        check("".equals(BridgePolicy.diagnosticText(null, "", 3000)));
        check(!BridgePolicy.terminal("planning"));
        check(!BridgePolicy.terminal("running"));
        check(!BridgePolicy.terminal("paused"));
        check(!BridgePolicy.terminal("paused_planning"));
        check(!BridgePolicy.terminal("queued"));
        check(!BridgePolicy.terminal(null));
        check(BridgePolicy.terminal("reported"));
        check(BridgePolicy.activeMonitorState("planning"));
        check(BridgePolicy.activeMonitorState("running"));
        check(BridgePolicy.activeMonitorState("paused"));
        check(!BridgePolicy.activeMonitorState(""));
        check(!BridgePolicy.activeMonitorState(null));
        check(!BridgePolicy.activeMonitorState("completed"));
        check(!BridgePolicy.activeMonitorState("interrupted"));
        check(BridgePolicy.mdiAlreadyReleasedError("不是当前 MDI 任务"));
        check(BridgePolicy.mdiAlreadyReleasedError("already released"));
        check(!BridgePolicy.mdiAlreadyReleasedError("connect timeout"));
        check(BridgePolicy.allowUnmatchedMdiStop(false, true, "stepx-mdi-1", false));
        check(BridgePolicy.allowUnmatchedMdiStop(false, false, "stepx-mdi-1", false));
        check(!BridgePolicy.allowUnmatchedMdiStop(true, true, "stepx-mdi-1", false));
        check(!BridgePolicy.allowUnmatchedMdiStop(false, false, "stepx-mdi-1", true));
        check("reported".equals(BridgePolicy.finishedState(new String[]{"reported"}, new boolean[]{false}, false)));
        check("reported".equals(BridgePolicy.finishedState(new String[]{"reported"}, new boolean[]{true}, false)));
        check("reported".equals(BridgePolicy.finishedState(new String[]{"verified"}, new boolean[]{false}, false)));
        check("reported".equals(BridgePolicy.finishedState(new String[]{"verified", "reported"}, new boolean[]{true, false}, false)));
        check("completed".equals(BridgePolicy.finishedState(new String[]{"verified", "verified"}, new boolean[]{true, true}, false)));
        check("completed".equals(BridgePolicy.finishedState(new String[]{"reported"}, new boolean[]{false}, true)));
        check("reported".equals(BridgePolicy.finishedState(new String[]{"reported", "reported"}, new boolean[]{false, false}, true)));
        check("reported".equals(BridgePolicy.finishedState(new String[]{"verified", "reported"}, new boolean[]{true, false}, true)));
        // Caller expectations cannot upgrade a failed or unfinished workflow.
        for (String phase : new String[]{"queued", "planning", "running", "paused", "failed", "stopped", "", null})
            check("stopped".equals(BridgePolicy.finishedState(new String[]{phase}, new boolean[]{true}, true)));
        check("stopped".equals(BridgePolicy.finishedState(new String[]{}, new boolean[]{}, true)));
        check("stopped".equals(BridgePolicy.finishedState(new String[]{"verified"}, new boolean[]{}, true)));
        check("stopped".equals(BridgePolicy.finishedState(null, null, true)));
        System.out.println("Bridge caller, protocol, request identity and completion evidence boundaries passed");
    }
}
