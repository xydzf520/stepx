package com.stepx.companion.app;
import java.util.*;
/** Shared path and age rules for preview and the destructive commit recheck. */
final class ExpansionRules {
    static String kind(String relative) {
        String n=relative.toLowerCase(Locale.ROOT);
        if(n.startsWith("/")||n.contains("\\")||n.contains("\u0000"))return "";
        for(String part:n.split("/"))if(part.isEmpty()||part.startsWith("."))return "";
        String ext=n.substring(n.lastIndexOf('.')+1);
        boolean camera=n.startsWith("dcim/camera/") || (n.startsWith("dcim/") && n.indexOf('/',5)<0);
        if(camera&&Arrays.asList("jpg","jpeg","png","heic","heif","dng","webp").contains(ext))return "photos";
        if(camera&&Arrays.asList("mp4","mov","m4v","3gp","mkv","webm").contains(ext))return "videos";
        if((n.startsWith("documents/")||n.startsWith("download/"))&&Arrays.asList("pdf","doc","docx","xls","xlsx","ppt","pptx","txt","md","csv","rtf","odt","ods","odp","epub").contains(ext))return "documents";
        return "";
    }
    static long readyAt(long modified,int days){return modified+Math.max(120000L,days*86400000L);}
    static int normalizeLimit(int limit){return Math.max(1,Math.min(100,limit<=0?20:limit));}
    static boolean matchesQuery(String query,String id,String name,String path,String kind,String state){
        if(query==null||query.trim().isEmpty())return true;
        String q=query.trim().toLowerCase(Locale.ROOT);
        String hay=((name==null?"":name)+" "+(path==null?"":path)+" "+(kind==null?"":kind)+" "+(id==null?"":id)+" "+(state==null?"":state)).toLowerCase(Locale.ROOT);
        return hay.contains(q);
    }
}
