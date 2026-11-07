package com.stepx.context.verification;

import android.app.RemoteInput;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import java.io.FileOutputStream;
import org.json.JSONObject;

/** Non-exported local receipt. Never contacts another app, user or server. */
public final class ReplyReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        String marker = intent.getStringExtra("marker");
        int sequence = intent.getIntExtra("sequence", 0);
        Bundle result = RemoteInput.getResultsFromIntent(intent);
        if (marker == null || !marker.matches("STEPX-VERIFY-[A-Za-z0-9-]{1,60}")
                || sequence < 1 || sequence > 99 || result == null) return;
        CharSequence text = result.getCharSequence("fixture_reply");
        if (text == null) return;
        try {
            JSONObject receipt = new JSONObject().put("marker", marker).put("sequence", sequence)
                .put("text", text.toString()).put("received_ms", System.currentTimeMillis());
            try (FileOutputStream out = context.openFileOutput(marker + "-" + sequence + ".json", Context.MODE_PRIVATE)) {
                out.write(receipt.toString().getBytes("UTF-8"));
            }
        } catch (Exception error) { throw new IllegalStateException("Local reply receipt failed", error); }
    }
}
