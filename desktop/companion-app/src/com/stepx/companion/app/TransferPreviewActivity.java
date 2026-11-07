package com.stepx.companion.app;

import android.app.Activity;
import android.content.*;
import android.os.Bundle;
import android.graphics.Typeface;
import android.text.Spannable;
import android.text.SpannableStringBuilder;
import android.text.style.*;
import java.util.regex.*;
import android.view.*;
import android.widget.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.*;
import java.util.zip.*;
import javax.xml.parsers.DocumentBuilderFactory;
import org.w3c.dom.*;

/** Read-only local preview. No model, network, macros or external document entities. */
public final class TransferPreviewActivity extends Activity {
    private LinearLayout page;
    private TextView status;
    private TransferStore.Entry entry;
    private Button mode;
    private TextView reader;
    private ScrollView documentScroll;
    private CharSequence readingBody;
    private String originalBody;
    private boolean reading = true;
    static final int MAX_PREVIEW_CHARS = 120000;
    static boolean supports(String name){return name.toLowerCase(Locale.ROOT).matches(".*\\.(txt|md|csv|tsv|json|xlsx)$");}
    static void open(Context c,String direction,String id){c.startActivity(new Intent(c,TransferPreviewActivity.class).putExtra("direction",direction).putExtra("id",id));}
    private int dp(int value){return Math.round(value*getResources().getDisplayMetrics().density);}
    private TextView text(String value,int size){TextView view=new TextView(this);view.setText(value);view.setTextSize(size);view.setTextColor(0xff202124);view.setPadding(dp(18),dp(12),dp(18),dp(12));view.setTextIsSelectable(true);return view;}
    private void style(Button b){b.setAllCaps(false);b.setTextSize(14);b.setTextColor(0xff202124);b.setMinWidth(0);b.setMinimumWidth(0);b.setPadding(dp(8),0,dp(8),0);b.setMaxLines(2);b.setBackgroundTintList(null);b.setBackground(new CompanionUi(this).touch(0xffffffff,0xffe4e4ea,12));LinearLayout.LayoutParams p=new LinearLayout.LayoutParams(0,dp(48),1);p.setMargins(dp(4),dp(8),dp(4),dp(8));b.setLayoutParams(p);}
    @Override public void onCreate(Bundle saved){super.onCreate(saved);
        page=new LinearLayout(this);page.setOrientation(LinearLayout.VERTICAL);page.setBackgroundColor(0xfff8f8fa);
        page.setOnApplyWindowInsetsListener((v,i)->{android.graphics.Insets b=i.getInsets(WindowInsets.Type.systemBars());v.setPadding(b.left,b.top,b.right,b.bottom);return i;});setContentView(page);
        LinearLayout toolbar=new LinearLayout(this);toolbar.setPadding(dp(10),0,dp(10),0);page.addView(toolbar,new LinearLayout.LayoutParams(-1,-2));
        Button back=new Button(this);back.setText("返回");style(back);toolbar.addView(back);back.setOnClickListener(v->finish());
        mode=new Button(this);mode.setText("查看原文");mode.setContentDescription("查看原文");style(mode);mode.setVisibility(View.GONE);toolbar.addView(mode);mode.setOnClickListener(v->{reading=!reading;showDocument();});
        Button external=new Button(this);external.setText("其他应用");external.setContentDescription("用其他应用打开");style(external);toolbar.addView(external);external.setOnClickListener(v->{if(entry==null)return;try{Intent i=new Intent(Intent.ACTION_VIEW).setDataAndType(entry.uri(),TransferStore.mime(entry.name)).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);i.setClipData(ClipData.newRawUri(entry.name,entry.uri()));startActivity(Intent.createChooser(i,"打开成果"));}catch(Exception e){status.setText("没有可用的查看应用，可到文件页保存或分享。");}});
        status=text("正在校验并打开文件…",14);page.addView(status);
        new Thread(()->{try{TransferStore store=new TransferStore(this);TransferStore.Entry e=store.get(getIntent().getStringExtra("direction"),getIntent().getStringExtra("id"));File file=store.verifiedFile(e.direction,e.id);runOnUiThread(()->{if(!isDestroyed())entry=e;});
            final List<List<String>> rows;final String body;final CharSequence formatted;
            final boolean markdown=e.name.toLowerCase(Locale.ROOT).endsWith(".md");
            if(e.name.toLowerCase(Locale.ROOT).endsWith(".xlsx")){rows=spreadsheet(file);body=null;formatted=null;}else{if(file.length()>1024*1024)throw new IOException("文件较大，请使用其他应用打开");String source=new String(Files.readAllBytes(file.toPath()),StandardCharsets.UTF_8);body=boundedPreview(source);formatted=markdown?markdownPreview(body):body;rows=null;}
            runOnUiThread(()->{if(isFinishing()||isDestroyed())return;entry=e;status.setText(e.name+" · "+TransferStore.size(e.size)+" · 已校验");ScrollView scroll=new ScrollView(this);scroll.setFillViewport(true);page.addView(scroll,new LinearLayout.LayoutParams(-1,0,1));
                if(rows==null){originalBody=body;readingBody=formatted;documentScroll=scroll;reader=text("",16);reader.setLineSpacing(dp(5),1.12f);reader.setPadding(dp(20),dp(18),dp(20),dp(32));reader.setBackgroundColor(0xffffffff);reader.setAutoLinkMask(0);scroll.addView(reader);mode.setVisibility(markdown?View.VISIBLE:View.GONE);reading=markdown;showDocument();}else{
                    status.append("\n首张工作表 · 最多显示 200 行、30 列 · 公式显示文件中保存的值");HorizontalScrollView horizontal=new HorizontalScrollView(this);scroll.addView(horizontal);TableLayout table=new TableLayout(this);table.setPadding(dp(10),dp(10),dp(10),dp(20));horizontal.addView(table);
                    for(int row=0;row<rows.size();row++){TableRow line=new TableRow(this);line.setBackgroundColor(row%2==0?0xffffffff:0xfff0f0f4);for(String cell:rows.get(row)){TextView value=text(cell,15);value.setMinWidth(dp(100));value.setMaxWidth(dp(320));line.addView(value);}table.addView(line);}
                }});
        }catch(Exception e){runOnUiThread(()->{if(!isDestroyed())status.setText("无法预览："+e.getMessage()+"。可在文件页保存后用其他应用打开。");});}},"file-preview").start();
    }
    private void showDocument(){
        if(reader==null)return;
        reader.setTypeface(reading?Typeface.DEFAULT:Typeface.MONOSPACE);
        reader.setText(reading?readingBody:originalBody);
        mode.setText(reading?"查看原文":"阅读模式");mode.setContentDescription(reading?"查看原文":"阅读模式");
        if(entry!=null)status.setText(entry.name+" · "+TransferStore.size(entry.size)+" · 已校验 · "+(reading?"阅读模式":"原文（可选中复制）"));
        if(originalBody!=null&&originalBody.endsWith(PREVIEW_LIMIT_NOTE))status.append("\n预览已截短，完整文件可用其他应用打开");
        documentScroll.post(()->documentScroll.scrollTo(0,0));
    }
    private static final String PREVIEW_LIMIT_NOTE="\n\n[预览已到长度上限；完整原件未修改，请用其他应用打开。]";
    static String boundedPreview(String source){
        if(source.length()<=MAX_PREVIEW_CHARS)return source;
        int end=MAX_PREVIEW_CHARS;if(Character.isHighSurrogate(source.charAt(end-1)))end--;
        return source.substring(0,end)+PREVIEW_LIMIT_NOTE;
    }
    // Normalize only paragraph separators; preserve every line inside fences.
    // Original/source mode always retains the exact received document bytes.
    static String compactMarkdownSpacing(String source){
        StringBuilder out=new StringBuilder();boolean code=false,blank=false;String fence="";
        for(String line:source.split("\\r?\\n",-1)){
            String trimmed=line.trim();boolean marker=trimmed.startsWith("```")||trimmed.startsWith("~~~");
            if(!code&&trimmed.isEmpty()){
                if(blank||out.length()==0)continue;
                blank=true;
            }else blank=false;
            if(out.length()>0)out.append('\n');out.append(line);
            if(marker){String value=trimmed.substring(0,3);if(!code){code=true;fence=value;}else if(value.equals(fence)){code=false;fence="";}}
        }
        return out.toString();
    }
    static CharSequence markdownPreview(String source){
        SpannableStringBuilder out=new SpannableStringBuilder();boolean code=false;String fence="";
        String[] lines=compactMarkdownSpacing(source).split("\\r?\\n",-1);int count=0;
        for(String line:lines){
            if(++count>4000){out.append("\n[阅读模式最多显示4000行；可切换原文核对。]\n");break;}
            String trimmed=line.trim();
            if(trimmed.startsWith("```")||trimmed.startsWith("~~~")){
                String marker=trimmed.substring(0,3);
                if(!code){code=true;fence=marker;out.append('\n');continue;}
                if(marker.equals(fence)){code=false;out.append('\n');continue;}
            }
            if(code){int a=out.length();out.append(line).append('\n');span(out,new TypefaceSpan("monospace"),a);span(out,new BackgroundColorSpan(0xfff1f2f5),a);continue;}
            int level=0;while(level<trimmed.length()&&level<6&&trimmed.charAt(level)=='#')level++;
            if(level>0&&trimmed.length()>level&&trimmed.charAt(level)==' '){
                int a=out.length();out.append(trimmed.substring(level+1));span(out,new StyleSpan(Typeface.BOLD),a);span(out,new RelativeSizeSpan(level==1?1.55f:level==2?1.3f:1.12f),a);out.append('\n');continue;
            }
            if(trimmed.matches("(?:-{3,}|\\*{3,}|_{3,})")){out.append("────────────\n\n");continue;}
            int a=out.length();String content=line;
            boolean quote=trimmed.startsWith(">");
            if(quote)content=trimmed.substring(1).trim();
            Matcher bullet=Pattern.compile("^\\s*[-+*]\\s+(.*)$").matcher(content);
            Matcher ordered=Pattern.compile("^\\s*(\\d{1,6}[.)])\\s+(.*)$").matcher(content);
            if(bullet.matches()){String item=bullet.group(1);if(item.startsWith("[ ] "))content="☐ "+item.substring(4);else if(item.startsWith("[x] ")||item.startsWith("[X] "))content="☑ "+item.substring(4);else content="• "+item;}
            else if(ordered.matches())content=ordered.group(1)+" "+ordered.group(2);
            appendInline(out,content);out.append('\n');
            if(quote){span(out,new LeadingMarginSpan.Standard(24),a);span(out,new ForegroundColorSpan(0xff626773),a);span(out,new BackgroundColorSpan(0xfff5f5f7),a);}
        }
        return out;
    }
    private static void span(SpannableStringBuilder out,Object style,int from){if(out.length()>from)out.setSpan(style,from,out.length(),Spannable.SPAN_EXCLUSIVE_EXCLUSIVE);}
    private static void appendInline(SpannableStringBuilder out,String text){
        Matcher m=Pattern.compile("`([^`\\n]+)`|\\*\\*([^*\\n]+)\\*\\*").matcher(text);int offset=0;
        while(m.find()){out.append(text,offset,m.start());int a=out.length();if(m.group(1)!=null){out.append(m.group(1));span(out,new TypefaceSpan("monospace"),a);span(out,new BackgroundColorSpan(0xfff1f2f5),a);}else{out.append(m.group(2));span(out,new StyleSpan(Typeface.BOLD),a);}offset=m.end();}
        out.append(text,offset,text.length());
    }
    private static String xml(ZipFile zip,String name)throws Exception {ZipEntry e=zip.getEntry(name);if(e==null)throw new IOException("缺少表格内容");try(InputStream in=zip.getInputStream(e);ByteArrayOutputStream out=new ByteArrayOutputStream()){byte[] b=new byte[8192];int n;while((n=in.read(b))!=-1){if(out.size()+n>2*1024*1024)throw new IOException("表格较大，请用其他应用查看");out.write(b,0,n);}String s=new String(out.toByteArray(),StandardCharsets.UTF_8);String upper=s.toUpperCase(Locale.ROOT);if(upper.contains("<!DOCTYPE")||upper.contains("<!ENTITY"))throw new IOException("不支持含外部实体的文件");return s;}}
    private static Document document(String value)throws Exception{DocumentBuilderFactory f=DocumentBuilderFactory.newInstance();f.setExpandEntityReferences(false);return f.newDocumentBuilder().parse(new ByteArrayInputStream(value.getBytes(StandardCharsets.UTF_8)));}
    private static String value(Element e,String tag){NodeList nodes=e.getElementsByTagName(tag);return nodes.getLength()==0?"":nodes.item(0).getTextContent();}
    private static List<List<String>> spreadsheet(File file)throws Exception {
        try(ZipFile zip=new ZipFile(file)){
            List<String> strings=new ArrayList<>();if(zip.getEntry("xl/sharedStrings.xml")!=null){NodeList si=document(xml(zip,"xl/sharedStrings.xml")).getElementsByTagName("si");if(si.getLength()>20000)throw new IOException("共享文本过多");for(int i=0;i<si.getLength();i++)strings.add(si.item(i).getTextContent());}
            Document book=document(xml(zip,"xl/workbook.xml"));Element first=(Element)book.getElementsByTagName("sheet").item(0);if(first==null)throw new IOException("没有工作表");String id=first.getAttribute("r:id"),target=null;
            NodeList rels=document(xml(zip,"xl/_rels/workbook.xml.rels")).getElementsByTagName("Relationship");for(int i=0;i<rels.getLength();i++){Element r=(Element)rels.item(i);if(id.equals(r.getAttribute("Id"))&&!"External".equals(r.getAttribute("TargetMode")))target=r.getAttribute("Target");}
            if(target==null||target.contains("..")||target.contains(":"))throw new IOException("工作表路径无效");String path=target.startsWith("/xl/")?target.substring(1):"xl/"+target;
            NodeList rows=document(xml(zip,path)).getElementsByTagName("row");List<List<String>> result=new ArrayList<>();
            for(int i=0;i<Math.min(200,rows.getLength());i++){List<String> values=new ArrayList<>();NodeList cells=((Element)rows.item(i)).getElementsByTagName("c");for(int j=0;j<cells.getLength();j++){Element c=(Element)cells.item(j);String ref=c.getAttribute("r"),kind=c.getAttribute("t"),v=value(c,"v");int column=0;for(char ch:ref.toCharArray()){if(ch<'A'||ch>'Z')break;column=column*26+ch-'A'+1;if(column>30)break;}if(column==0)column=values.size()+1;if(column>30)continue;while(values.size()<column)values.add("");if("s".equals(kind)){int index=Integer.parseInt(v);if(index<0||index>=strings.size())throw new IOException("表格文本引用无效");v=strings.get(index);}else if("inlineStr".equals(kind))v=value(c,"is");else if("b".equals(kind))v="1".equals(v)?"是":"否";values.set(column-1,v);}result.add(values);}
            return result;
        }
    }
}
