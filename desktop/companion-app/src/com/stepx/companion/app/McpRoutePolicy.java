package com.stepx.companion.app;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.List;

/** Keep the legacy per-action controller from competing with the selected Agent. */
final class McpRoutePolicy {
    static final int MAX_SOCKET_LINE_BYTES = 65536;
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
    static boolean isAuthorizedMcpCaller(int callingUid, int ownUid, boolean permissionGranted, boolean systemPackage) {
        if (callingUid == ownUid || callingUid == 1000 || callingUid == 2000) return true;
        return permissionGranted && systemPackage;
    }
    static boolean isTrustedShortcutAuthority(String authority) {
        if (authority == null || authority.isEmpty()) return false;
        return "com.android.externalstorage.documents".equals(authority)
            || "com.android.providers.downloads.documents".equals(authority)
            || "media".equals(authority)
            || "com.stepos.filemanager".equals(authority)
            || authority.startsWith("com.stepos.filemanager.")
            || authority.startsWith("com.android.fileexplorer")
            || authority.startsWith("com.stepx.companion.app.");
    }
    static String sanitizeStudioOp(String op) {
        return ("open".equals(op) || "pin".equals(op)) ? op : null;
    }
    static String sanitizeStudioId(String id) {
        return (id != null && id.matches("[A-Za-z0-9._:-]{1,128}")) ? id : null;
    }

    static final class BoundedUtf8LineReader {
        private final InputStream in;
        private final int maxBytes;
        private final byte[] buf = new byte[4096];
        private int pos = 0;
        private int limit = 0;

        BoundedUtf8LineReader(InputStream in, int maxBytes) {
            this.in = in;
            this.maxBytes = maxBytes;
        }

        String readLine() throws IOException {
            if (in == null || maxBytes <= 0) return null;
            ByteArrayOutputStream out = new ByteArrayOutputStream(256);
            boolean sawBytes = false;
            while (true) {
                if (pos >= limit) {
                    final int n;
                    try {
                        n = in.read(buf, 0, buf.length);
                    } catch (SocketTimeoutException timeout) {
                        if (out.size() > 0) {
                            throw new IOException("Incomplete socket line timed out", timeout);
                        }
                        throw timeout;
                    }
                    if (n < 0) {
                        if (!sawBytes && out.size() == 0) return null;
                        break;
                    }
                    if (n == 0) continue;
                    pos = 0;
                    limit = n;
                }
                sawBytes = true;
                byte b = buf[pos++];
                if (b == '\n') {
                    break;
                }
                if (out.size() >= maxBytes) {
                    throw new IOException("Socket line exceeds " + maxBytes + " bytes");
                }
                out.write(b);
            }
            byte[] raw = out.toByteArray();
            int len = raw.length;
            if (len > 0 && raw[len - 1] == '\r') {
                len--;
            }
            return new String(raw, 0, len, StandardCharsets.UTF_8);
        }
    }
}
