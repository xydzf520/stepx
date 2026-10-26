package com.stepx.companion.app;

import android.content.Context;
import android.net.Uri;
import android.util.AtomicFile;
import org.json.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.regex.Pattern;

/** A private, bounded phone library. Entries are links, never fabricated skills. */
final class StudioStore {
    private static final Pattern ID_PATTERN=Pattern.compile("studio-[a-f0-9]{32}");
    private static final Pattern CNTRL_PATTERN=Pattern.compile("(?s).*[\\p{Cntrl}].*");
    private static final Pattern PATH_PATTERN=Pattern.compile("/t/[A-Za-z0-9_-]{1,128}");
    private final AtomicFile file;
    StudioStore(Context c){file=new AtomicFile(new File(c.getFilesDir(),"studio-works.json"));}
    static void validate(JSONObject w)throws Exception {
        String id=w.getString("id"),name=w.getString("name"),url=w.getString("url"),type=w.getString("category");
        if(!ID_PATTERN.matcher(id).matches()||name.trim().isEmpty()||name.length()>120||CNTRL_PATTERN.matcher(name).matches())throw new IllegalArgumentException("作品名称或编号无效");
        if(!type.equals("应用")&&!type.equals("智能体")&&!type.equals("技能"))throw new IllegalArgumentException("作品类型无效");
        Uri u=Uri.parse(url);
        if(url.length()>2048||url.contains("\\")||!"https".equals(u.getScheme())||!"studio-api.stepx.com".equals(u.getHost())||u.getUserInfo()!=null||u.getPort()!=-1||u.getQuery()!=null||u.getFragment()!=null||u.getEncodedPath()==null||!PATH_PATTERN.matcher(u.getEncodedPath()).matches())throw new IllegalArgumentException("只接受官方 Studio 体验链接");
    }
    JSONArray list()throws Exception {synchronized(StudioStore.class){
        if(!file.getBaseFile().exists())return new JSONArray();
        try(InputStream in=file.openRead()){
            byte[] bytes=in.readNBytes(1024*1024+1);if(bytes.length>1024*1024)throw new IOException("作品记录过大");
            JSONArray all=new JSONArray(new String(bytes,StandardCharsets.UTF_8));if(all.length()>200)throw new IOException("作品数量超出限制");
            for(int i=0;i<all.length();i++)validate(all.getJSONObject(i));return all;
        }
    }}
    JSONObject get(String id)throws Exception {JSONArray all=list();for(int i=0;i<all.length();i++)if(all.getJSONObject(i).getString("id").equals(id))return all.getJSONObject(i);throw new IllegalArgumentException("作品尚未保存到这台手机，请从 desktop 重新发送");}
    void save(JSONObject work)throws Exception {synchronized(StudioStore.class){
        validate(work);JSONArray old=list(),out=new JSONArray();out.put(work);
        for(int i=0;i<old.length();i++){JSONObject item=old.getJSONObject(i);if(!item.getString("id").equals(work.getString("id"))&&!item.getString("url").equals(work.getString("url")))out.put(item);}
        if(out.length()>200)throw new IOException("手机作品库已达 200 条");
        FileOutputStream stream=null;
        try{stream=file.startWrite();stream.write(out.toString().getBytes(StandardCharsets.UTF_8));file.finishWrite(stream);}catch(Exception e){if(stream!=null)file.failWrite(stream);throw e;}
    }}
}
