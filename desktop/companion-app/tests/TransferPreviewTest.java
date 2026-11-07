package com.stepx.companion.app;
import java.nio.file.*;import java.util.*;import java.util.zip.*;import java.io.*;import java.lang.reflect.*;
public class TransferPreviewTest {
 static void put(ZipOutputStream out,String path,String content)throws Exception{out.putNextEntry(new ZipEntry(path));out.write(content.getBytes("UTF-8"));out.closeEntry();}
 static File fixture(String sheet)throws Exception{File f=File.createTempFile("stepx-preview-",".xlsx");f.deleteOnExit();try(ZipOutputStream z=new ZipOutputStream(new FileOutputStream(f))){put(z,"xl/workbook.xml","<workbook xmlns:r='r'><sheets><sheet r:id='rId1'/></sheets></workbook>");put(z,"xl/_rels/workbook.xml.rels","<Relationships><Relationship Id='rId1' Target='worksheets/sheet1.xml'/></Relationships>");put(z,"xl/sharedStrings.xml","<sst><si><t>项目</t></si></sst>");put(z,"xl/worksheets/sheet1.xml",sheet);}return f;}
 static void markdownSpacing(){
  String source="# 标题\n\n\n## 判断\n\n正文\n\n\n后段";
  String expected="# 标题\n\n## 判断\n\n正文\n\n后段";
  if(!TransferPreviewActivity.compactMarkdownSpacing(source).equals(expected))throw new AssertionError("paragraph spacing not compacted");
  String code="开头\n\n```text\na\n\n\nb\n```\n\n\n结尾";
  String compact=TransferPreviewActivity.compactMarkdownSpacing(code);
  if(!compact.contains("```text\na\n\n\nb\n```"))throw new AssertionError("code blank lines changed");
  if(!compact.endsWith("```\n\n结尾"))throw new AssertionError("post-code spacing not compacted");
  String tilde="~~~\n\n\ncode\n~~~";
  if(!TransferPreviewActivity.compactMarkdownSpacing(tilde).equals(tilde))throw new AssertionError("tilde code whitespace changed");
 }
 public static void main(String[] a)throws Exception{markdownSpacing();Method m=TransferPreviewActivity.class.getDeclaredMethod("spreadsheet",File.class);m.setAccessible(true);Object rows=m.invoke(null,fixture("<worksheet><sheetData><row><c r='A1' t='s'><v>0</v></c><c r='C1'><f>SUM(A2:A3)</f><v>95</v></c></row></sheetData></worksheet>"));if(!rows.toString().equals("[[项目, , 95]]"))throw new AssertionError(rows);boolean refused=false;try{m.invoke(null,fixture("<!DOCTYPE worksheet [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><worksheet>&x;</worksheet>"));}catch(InvocationTargetException e){refused=e.getCause() instanceof IOException;}if(!refused)throw new AssertionError("external entity accepted");System.out.println("TransferPreviewTest: compact Markdown paragraphs, preserved code whitespace, cached formula, sparse columns, shared strings, external entity rejection passed");}
}
