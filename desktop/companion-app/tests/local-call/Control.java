package com.stepx.callverify;
import android.content.*;
import android.telecom.*;
import android.net.Uri;
import android.os.Bundle;
public final class Control extends BroadcastReceiver {
 public static PhoneAccountHandle handle(Context c) { return new PhoneAccountHandle(new ComponentName(c, CallService.class),"stepx-local"); }
 public void onReceive(Context c, Intent i) {
  try {
   TelecomManager manager=c.getSystemService(TelecomManager.class);
   String op=i.getStringExtra("op");
   if ("register".equals(op)) manager.registerPhoneAccount(PhoneAccount.builder(handle(c),"StepX 本机验收").setCapabilities(PhoneAccount.CAPABILITY_CALL_PROVIDER).addSupportedUriScheme("tel").build());
   if ("incoming".equals(op)) {
    Bundle b=new Bundle();b.putParcelable(TelecomManager.EXTRA_INCOMING_CALL_ADDRESS,Uri.parse("tel:+15550163"));manager.addNewIncomingCall(handle(c),b);
   }
   if ("unregister".equals(op)) manager.unregisterPhoneAccount(handle(c));
   if ("expire".equals(op)) CallService.expire();
   setResultData("ok");
  } catch(Exception e) { setResultData(e.getClass().getSimpleName()+": "+e.getMessage()); }
 }
}
