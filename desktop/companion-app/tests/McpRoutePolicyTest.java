package com.stepx.companion.app;

public final class McpRoutePolicyTest {
    private static void check(boolean value){if(!value)throw new AssertionError();}
    public static void main(String[] args){
        check(McpRoutePolicy.visible("stepx_gui_control",false));
        check(McpRoutePolicy.visible("stepx_gui_agent",false));
        check(McpRoutePolicy.visible("stepx_desktop_work",false));
        check(McpRoutePolicy.visible("stepx_expansion",false));
        check(!McpRoutePolicy.visible("stepx_gui_control",true));
        check(McpRoutePolicy.visible("stepx_gui_agent",true));
        check(McpRoutePolicy.visible("stepx_desktop_work",true));
        check(McpRoutePolicy.visible("stepx_expansion",true));
        check(!McpRoutePolicy.visible("unknown_tool",false));
        check(!McpRoutePolicy.visible(null,false));
        for(String op:new String[]{"status","list","get","request"}){
            check(McpRoutePolicy.isExpansionOperation(op));
        }
        check(!McpRoutePolicy.isExpansionOperation("delete"));
        check(!McpRoutePolicy.isExpansionOperation(null));
        for(String op:new String[]{"status","stop"})check(McpRoutePolicy.legacyOperation(op,true));
        for(String op:new String[]{"start","observe","action","pause","resume"}){
            check(McpRoutePolicy.legacyOperation(op,false));
            check(!McpRoutePolicy.legacyOperation(op,true));
        }
        check(!McpRoutePolicy.legacyOperation(null,true));
        check(!McpRoutePolicy.legacyOperation("unknown",true));
        // A cached tool descriptor cannot reopen the retired control path.
        check(!McpRoutePolicy.legacyOperation("start",true));
        check(McpRoutePolicy.legacyOperation("start",false));
        // [P0-1] NativeMcpService & McpScreenshotProvider caller identity checks:
        check(McpRoutePolicy.isAuthorizedMcpCaller(10250, 10250, false, false));
        check(McpRoutePolicy.isAuthorizedMcpCaller(1000, 10250, false, false));
        check(McpRoutePolicy.isAuthorizedMcpCaller(2000, 10250, false, false));
        check(McpRoutePolicy.isAuthorizedMcpCaller(10120, 10250, true, true));
        check(!McpRoutePolicy.isAuthorizedMcpCaller(10333, 10250, true, false));
        check(!McpRoutePolicy.isAuthorizedMcpCaller(10120, 10250, false, true));
        check(!McpRoutePolicy.isAuthorizedMcpCaller(10333, 10250, false, false));
        // [P1-5] Shortcut authority & Studio Intent sanitization:
        check(McpRoutePolicy.isTrustedShortcutAuthority("com.android.externalstorage.documents"));
        check(McpRoutePolicy.isTrustedShortcutAuthority("media"));
        check(McpRoutePolicy.isTrustedShortcutAuthority("com.stepx.companion.app.expansion.documents"));
        check(!McpRoutePolicy.isTrustedShortcutAuthority("com.untrusted.attacker.provider"));
        check(!McpRoutePolicy.isTrustedShortcutAuthority(null));
        check("open".equals(McpRoutePolicy.sanitizeStudioOp("open")));
        check("pin".equals(McpRoutePolicy.sanitizeStudioOp("pin")));
        check(McpRoutePolicy.sanitizeStudioOp("delete") == null);
        check("work-123".equals(McpRoutePolicy.sanitizeStudioId("work-123")));
        check(McpRoutePolicy.sanitizeStudioId("../bad") == null);
        // [P2-1] Bounded UTF-8 socket line reader:
        try {
            McpRoutePolicy.BoundedUtf8LineReader emptyReader =
                new McpRoutePolicy.BoundedUtf8LineReader(new java.io.ByteArrayInputStream(new byte[0]), 64);
            check(emptyReader.readLine() == null);
            byte[] twoLines = "{\"op\":\"ping\"}\r\n{\"op\":\"subscribe\"}\n".getBytes(java.nio.charset.StandardCharsets.UTF_8);
            McpRoutePolicy.BoundedUtf8LineReader multi =
                new McpRoutePolicy.BoundedUtf8LineReader(new java.io.ByteArrayInputStream(twoLines), 64);
            check("{\"op\":\"ping\"}".equals(multi.readLine()));
            check("{\"op\":\"subscribe\"}".equals(multi.readLine()));
            check(multi.readLine() == null);
            boolean rejectedOversize = false;
            try {
                byte[] huge = new byte[McpRoutePolicy.MAX_SOCKET_LINE_BYTES + 1];
                java.util.Arrays.fill(huge, (byte) 'x');
                new McpRoutePolicy.BoundedUtf8LineReader(
                    new java.io.ByteArrayInputStream(huge), McpRoutePolicy.MAX_SOCKET_LINE_BYTES
                ).readLine();
            } catch (java.io.IOException expected) {
                rejectedOversize = true;
            }
            check(rejectedOversize);
        } catch (java.io.IOException unexpected) {
            throw new AssertionError(unexpected);
        }
        System.out.println("MCP tool visibility and selected-agent control isolation passed");
    }
}
