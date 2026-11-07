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
        System.out.println("MCP tool visibility and selected-agent control isolation passed");
    }
}
