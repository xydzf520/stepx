package com.stepx.companion.app;
public final class ChatReplyPolicyTest {
 private static void check(boolean b){if(!b)throw new AssertionError();}
 public static void main(String[] args){
  check(ChatReplyPolicy.sameRecipient("+8613800002916","138 0000 2916"));
  check(!ChatReplyPolicy.sameRecipient("13800002916","13800007650"));
  check(!ChatReplyPolicy.sameRecipient("Alice","Alice work"));
  check(!ChatReplyPolicy.sameRecipient("+441234567890","1234567890"));
  check(!ChatReplyPolicy.sameRecipient("", ""));
  check(!ChatReplyPolicy.verifiedAfterSend(1,1,true,true));
  check(!ChatReplyPolicy.verifiedAfterSend(1,2,false,true));
  check(!ChatReplyPolicy.verifiedAfterSend(1,2,true,false));
  check(ChatReplyPolicy.verifiedAfterSend(1,2,true,true));
  System.out.println("Chat recipient and send verification passed");
 }
}
