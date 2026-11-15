package com.stepx.companion.neo;

/** Original StepX integration policy; independent of the downloaded agent source. */
final class BridgePolicy {
    static boolean hostControls(long expires,long now,String foreground){return expires>now&&"com.stepx.companion.app".equals(foreground);}

    static final String COMPANION = "com.stepx.companion.app";
    static boolean caller(int uid, int ownUid, String[] packages, boolean signaturesMatch) {
        if (uid == ownUid || uid == 2000) return true;
        return signaturesMatch && packages != null && packages.length == 1
            && COMPANION.equals(packages[0]);
    }
    static boolean requestId(String id) {
        return id != null && id.matches("[A-Za-z0-9._:-]{1,128}");
    }
    static String protocol(String value) {
        if ("openai".equalsIgnoreCase(value)) return "OpenAI";
        if ("anthropic".equalsIgnoreCase(value)) return "Anthropic";
        if ("gemini".equalsIgnoreCase(value)) return "Gemini";
        return null;
    }
    static boolean terminal(String state) {
        return "completed".equals(state) || "reported".equals(state) || "failed".equals(state)
            || "stopped".equals(state) || "interrupted".equals(state) || "needs_input".equals(state);
    }
    static boolean activeMonitorState(String state) {
        return state != null && !state.isEmpty() && !terminal(state);
    }
    static boolean mdiAlreadyReleasedError(String message) {
        return message != null && (message.contains("不是当前 MDI 任务")
            || message.contains("already released")
            || message.contains("MDI 任务已回收")
            || message.contains("不存在该 MDI 任务"));
    }
    static boolean allowUnmatchedMdiStop(boolean active, boolean restartCleanup, String taskName, boolean released) {
        return !active && (restartCleanup || (taskName != null && !taskName.isEmpty() && !released));
    }
    static String diagnosticText(String value, String credential, int limit) {
        if (value == null) return "";
        // Redact before truncation so even an arbitrary-format key cannot leak
        // a surviving prefix or suffix at the length boundary.
        if (credential != null && !credential.isEmpty()) value = value.replace(credential, "[credential hidden]");
        value = value.replaceAll("(?i)sk-[a-z0-9_-]{12,}", "[credential hidden]")
            .replaceAll("(?i)Bearer\\s+[a-z0-9._~+/-]+=*", "Bearer [hidden]");
        return value.length() <= limit ? value : value.substring(value.length() - limit);
    }
    /** Only actual, terminal workflow evidence may establish completion. */
    static String finishedState(String[] phases, boolean[] verified, boolean callerVerified) {
        if (phases == null || verified == null || phases.length == 0 || phases.length != verified.length) return "stopped";
        boolean allVerified = true;
        for (int i = 0; i < phases.length; i++) {
            if (!"reported".equals(phases[i]) && !"verified".equals(phases[i])) return "stopped";
            allVerified &= "verified".equals(phases[i]) && verified[i];
        }
        // A caller's final-screen assertion proves only one stage. It cannot
        // retroactively verify earlier merely reported stages in a queue.
        return allVerified || (callerVerified && phases.length == 1) ? "completed" : "reported";
    }
}
