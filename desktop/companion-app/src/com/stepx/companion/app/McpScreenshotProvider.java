package com.stepx.companion.app;

import android.content.*;
import android.database.*;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.*;
import java.util.List;

/** Read-only, short-lived captures for the original permission-protected MCP uploader. */
public final class McpScreenshotProvider extends ContentProvider {
    static final String AUTHORITY="com.stepx.companion.app.mcp.screenshot";
    @Override public boolean onCreate(){return true;}
    private File capture(Uri uri)throws FileNotFoundException {
        NativeMcpService.enforceMcpCaller(getContext());
        List<String> parts=uri.getPathSegments();
        if(!AUTHORITY.equals(uri.getAuthority())||parts.size()!=1||!parts.get(0).matches("[a-f0-9]{32}\\.jpg"))throw new FileNotFoundException("Invalid capture");
        GuiAccessibilityService service=GuiAccessibilityService.instance;
        if(service==null||service.task.userStopped||!service.task.active()||!parts.get(0).equals(service.task.id+".jpg"))throw new FileNotFoundException("Task ended");
        File file=NativeMcpService.mcpCaptureFile(getContext(),parts.get(0));
        long age=System.currentTimeMillis()-file.lastModified();
        if(!file.isFile()||age<0||age>45000)throw new FileNotFoundException("Capture expired");
        return file;
    }
    @Override public String getType(Uri uri){
        NativeMcpService.enforceMcpCaller(getContext());
        return "image/jpeg";
    }
    @Override public Cursor query(Uri uri,String[] projection,String selection,String[] args,String order){
        NativeMcpService.enforceMcpCaller(getContext());
        try{File file=capture(uri);String[] columns=projection==null?new String[]{OpenableColumns.DISPLAY_NAME,OpenableColumns.SIZE}:projection;
            MatrixCursor result=new MatrixCursor(columns,1);Object[] values=new Object[columns.length];
            for(int i=0;i<columns.length;i++){if(OpenableColumns.DISPLAY_NAME.equals(columns[i]))values[i]="screen.jpg";else if(OpenableColumns.SIZE.equals(columns[i]))values[i]=file.length();}
            result.addRow(values);return result;
        }catch(FileNotFoundException e){return null;}
    }
    @Override public ParcelFileDescriptor openFile(Uri uri,String mode)throws FileNotFoundException {
        NativeMcpService.enforceMcpCaller(getContext());
        if(!"r".equals(mode))throw new FileNotFoundException("Read only");
        return ParcelFileDescriptor.open(capture(uri),ParcelFileDescriptor.MODE_READ_ONLY);
    }
    @Override public Uri insert(Uri uri,ContentValues values){throw new UnsupportedOperationException();}
    @Override public int update(Uri uri,ContentValues values,String selection,String[] args){throw new UnsupportedOperationException();}
    @Override public int delete(Uri uri,String selection,String[] args){throw new UnsupportedOperationException();}
}
