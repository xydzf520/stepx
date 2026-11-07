package com.stepx.callverify;
import android.telecom.*;
import android.net.Uri;
import java.nio.charset.StandardCharsets;
public final class CallService extends ConnectionService {
 static Connection pending;
 static CallService owner;
 static void expire() {
  if (pending == null || pending.getState() != Connection.STATE_RINGING) return;
  pending.setDisconnected(new DisconnectCause(DisconnectCause.MISSED));pending.destroy();pending=null;
  if (owner != null) owner.record("missed");
 }
 void record(String state) {
  try {try(java.io.FileOutputStream out=openFileOutput("state.txt",MODE_PRIVATE)) {out.write(state.getBytes(StandardCharsets.UTF_8));}} catch(Exception ignored){}
 }
 public Connection onCreateIncomingConnection(PhoneAccountHandle h,ConnectionRequest r) {
  Connection c=new Connection(){
   public void onAnswer(){setActive();record("active");}
   public void onAnswer(int videoState){onAnswer();}
   public void onReject(){setDisconnected(new DisconnectCause(DisconnectCause.REJECTED));destroy();record("rejected");}
   public void onDisconnect(){setDisconnected(new DisconnectCause(DisconnectCause.LOCAL));destroy();record("ended");}
  };
  c.setAddress(Uri.parse("tel:+15550163"),TelecomManager.PRESENTATION_ALLOWED);
  c.setCallerDisplayName("StepX 本机通话验收",TelecomManager.PRESENTATION_ALLOWED);
  c.setAudioModeIsVoip(true);c.setRinging();pending=c;owner=this;record("ringing");return c;
 }
}
