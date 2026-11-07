package com.stepx.companion.app;

import java.util.Arrays;
import java.util.List;

/** Keep the legacy per-action controller from competing with the selected Agent. */
final class McpRoutePolicy {
    static final List<String> SUPPORTED_TOOLS = Arrays.asList(
        "stepx_gui_control",
        "stepx_gui_agent",
        "stepx_desktop_work",
        "stepx_expansion"
    );
    static final List<String> EXPANSION_OPERATIONS = Arrays.asList(
        "status",
        "list",
        "get",
        "request"
    );
    static boolean isSupportedTool(String tool) {
        return tool != null && SUPPORTED_TOOLS.contains(tool);
    }
    static boolean isExpansionOperation(String operation) {
        return operation != null && EXPANSION_OPERATIONS.contains(operation);
    }
    static boolean visible(String tool, boolean neoSelected) {
        if (!isSupportedTool(tool)) return false;
        return !neoSelected || !"stepx_gui_control".equals(tool);
    }
    static boolean legacyOperation(String operation, boolean neoSelected) {
        if (!Arrays.asList("status", "start", "observe", "action", "pause", "resume", "stop").contains(operation)) {
            return false;
        }
        return !neoSelected || "status".equals(operation) || "stop".equals(operation);
    }
}
