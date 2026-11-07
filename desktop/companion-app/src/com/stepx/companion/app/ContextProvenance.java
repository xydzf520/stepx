package com.stepx.companion.app;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;

/** Provenance rules shared by persisted messages and explicit screen capture. */
final class ContextProvenance {
    static boolean timeKnown(String via, boolean hasExplicitValue, boolean explicitValue) {
        if (hasExplicitValue) return explicitValue;
        return via == null || !via.startsWith("ui_");
    }

    static String captureId(String sender, String text, boolean outgoing, int occurrence) {
        try {
            String key = sender.length() + ":" + sender + text.length() + ":" + text
                + ":" + outgoing + ":" + occurrence;
            byte[] hash = MessageDigest.getInstance("SHA-256").digest(key.getBytes(StandardCharsets.UTF_8));
            StringBuilder out = new StringBuilder("capture_");
            for (byte b : hash) out.append(String.format(java.util.Locale.ROOT, "%02x", b & 255));
            return out.toString();
        } catch (java.security.NoSuchAlgorithmException impossible) {
            throw new IllegalStateException(impossible);
        }
    }
}
