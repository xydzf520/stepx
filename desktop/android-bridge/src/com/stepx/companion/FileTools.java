package com.stepx.companion;

import android.os.StatFs;
import org.json.JSONObject;
import java.io.*;
import java.nio.file.*;
import java.nio.file.attribute.BasicFileAttributes;
import java.security.MessageDigest;
import java.util.*;
import java.util.zip.*;

/** User-confirmed operations on public documents; no app data or system writes. */
final class FileTools {
    static final long MAX_BYTES = 256L * 1024 * 1024;
    static final Set<String> TEXT_EXT = new HashSet<>(Arrays.asList("txt", "log", "csv", "json", "xml", "md", "html", "css", "svg"));

    static Path publicFile(String logical) throws Exception {
        if (!logical.startsWith("/sdcard/") || logical.indexOf('\0') >= 0) throw new IOException("仅支持共享存储文件");
        String relative = logical.substring(8);
        String[] parts = relative.split("/", -1);
        if (parts.length < 2 || !Arrays.asList(StorageTools.FOLDERS).contains(parts[0])) throw new IOException("请选择下载、文档、照片、视频或音乐目录内的文件");
        Path root = new File("/sdcard").getCanonicalFile().toPath();
        if (!root.toString().equals("/storage/emulated/0")) throw new IOException("共享存储用户不受支持");
        Path path = root;
        for (String part : parts) {
            if (part.isEmpty() || part.equals(".") || part.equals("..")) throw new IOException("文件路径无效");
            path = path.resolve(part);
            if (Files.isSymbolicLink(path)) throw new IOException("不支持符号链接");
        }
        if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) throw new IOException("文件已不存在或不是普通文件");
        return path;
    }

    static BasicFileAttributes attrs(Path path) throws Exception {
        BasicFileAttributes a = Files.readAttributes(path, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
        if (!a.isRegularFile()) throw new IOException("文件类型已变化");
        return a;
    }

    static void unchanged(Path path, BasicFileAttributes before) throws Exception {
        BasicFileAttributes after = attrs(path);
        if (before.size() != after.size() || !before.lastModifiedTime().equals(after.lastModifiedTime())
            || !Objects.equals(before.fileKey(), after.fileKey())) throw new IOException("文件正在被修改，请关闭使用它的应用后重试");
    }

    static byte[] digest(InputStream in, OutputStream out, long deadline) throws Exception {
        MessageDigest hash = MessageDigest.getInstance("SHA-256");
        long total = 0;
        byte[] buffer = new byte[65536];
        int n;
        while ((n = in.read(buffer)) != -1) {
            total += n;
            if (total > MAX_BYTES || System.nanoTime() > deadline) throw new IOException("超过单文件 256 MiB 或处理时限；原文件已保留");
            hash.update(buffer, 0, n);
            if (out != null) out.write(buffer, 0, n);
        }
        return hash.digest();
    }

    static byte[] hash(Path path, long deadline) throws Exception {
        try (InputStream in = Files.newInputStream(path, LinkOption.NOFOLLOW_LINKS)) { return digest(in, null, deadline); }
    }

    // Settle delayed allocations and orphan reclamation before comparing free space.
    static boolean syncStorage() throws Exception {
        Process sync = new ProcessBuilder("/system/bin/sync").start();
        if (!sync.waitFor(5, java.util.concurrent.TimeUnit.SECONDS)) {
            sync.destroyForcibly();
            return false;
        }
        return sync.exitValue() == 0;
    }

    static JSONObject run(JSONObject request) throws Exception {
        Path source = publicFile(request.getString("path"));
        BasicFileAttributes before = attrs(source);
        if (before.size() != request.getLong("bytes") || before.lastModifiedTime().toMillis() != request.getLong("modified_ms"))
            throw new IOException("文件已变化，请重新扫描并确认");
        String action = request.getString("action");
        long deadline = System.nanoTime() + 45_000_000_000L;
        syncStorage();
        long availableBefore = new StatFs("/data").getAvailableBytes();
        long logicalSaved = 0;
        String resultingPath = request.getString("path");
        if (action.equals("delete_file")) {
            unchanged(source, before);
            Files.delete(source);
            logicalSaved = before.size();
        } else if (action.equals("delete_duplicate")) {
            Path keep = publicFile(request.getString("keep"));
            if (Files.isSameFile(source, keep)) throw new IOException("必须保留另一份独立文件");
            BasicFileAttributes keepBefore = attrs(keep);
            if (before.size() != keepBefore.size() || !Arrays.equals(hash(source, deadline), hash(keep, deadline)))
                throw new IOException("两个文件内容不同，未删除");
            unchanged(source, before);
            unchanged(keep, keepBefore);
            Files.delete(source);
            logicalSaved = before.size();
            resultingPath = request.getString("keep");
        } else if (action.equals("compress") || action.equals("restore")) {
            boolean restore = action.equals("restore");
            String name = source.getFileName().toString();
            String originalName = restore && name.endsWith(".gz") ? name.substring(0, name.length() - 3) : name;
            String ext = ContextReadPolicy.fileExtension(originalName);
            if (!TEXT_EXT.contains(ext) || (restore && !name.endsWith(".gz")))
                throw new IOException("仅支持文本文档压缩与还原（txt、log、csv、json、xml、md、html、css、svg）");
            if (before.size() > MAX_BYTES) throw new IOException("仅支持不超过 256 MiB 的文件");
            Path target = source.resolveSibling(restore ? originalName : name + ".gz");
            if (Files.exists(target, LinkOption.NOFOLLOW_LINKS)) throw new IOException("目标文件已存在，请先处理同名文件");
            Path temp = Files.createTempFile(source.getParent(), ".stepx-compress-", ".tmp");
            try {
                byte[] sourceHash = hash(source, deadline);
                byte[] contentHash;
                try (InputStream raw = Files.newInputStream(source, LinkOption.NOFOLLOW_LINKS);
                     InputStream in = restore ? new GZIPInputStream(raw) : raw;
                     FileOutputStream file = new FileOutputStream(temp.toFile())) {
                    if (restore) { contentHash = digest(in, file, deadline); file.getFD().sync(); }
                    else {
                        GZIPOutputStream out = new GZIPOutputStream(file);
                        contentHash = digest(in, out, deadline);
                        out.finish(); out.flush(); file.getFD().sync();
                    }
                }
                byte[] checked;
                try (InputStream raw = Files.newInputStream(temp);
                     InputStream in = restore ? raw : new GZIPInputStream(raw)) { checked = digest(in, null, deadline); }
                if (!Arrays.equals(contentHash, checked) || !Arrays.equals(sourceHash, hash(source, deadline)))
                    throw new IOException("内容校验失败，原文件已保留");
                unchanged(source, before);
                long outputBytes = Files.size(temp);
                if (!restore && outputBytes >= before.size()) throw new IOException("压缩后不会节省空间，原文件已保留");
                Files.setLastModifiedTime(temp, before.lastModifiedTime());
                Files.move(temp, target); // No REPLACE_EXISTING: never overwrite user files.
                if (!syncStorage()) throw new IOException("新文件已生成，存储同步未完成；原文件已保留，请重新扫描");
                try {
                    unchanged(source, before);
                    Files.delete(source); // Only after the verified replacement is durable and visible.
                } catch (Exception e) {
                    throw new IOException("已生成 " + target.getFileName() + "，原文件未删除，请检查后重新扫描", e);
                }
                logicalSaved = before.size() - outputBytes;
                resultingPath = request.getString("path").substring(0, request.getString("path").lastIndexOf('/') + 1) + target.getFileName();
            } finally { Files.deleteIfExists(temp); }
        } else throw new IOException("不支持的存储操作");
        syncStorage();
        long availableAfter = new StatFs("/data").getAvailableBytes();
        return new JSONObject().put("available_before", availableBefore).put("available_after", availableAfter)
            .put("logical_saved", logicalSaved).put("result_path", resultingPath).put("completed", true);
    }

    static JSONObject clearCache() throws Exception {
        syncStorage();
        StatFs before = new StatFs("/data");
        Process process = new ProcessBuilder("/system/bin/cmd", "package", "trim-caches",
            Long.toString(before.getTotalBytes()), "internal").redirectErrorStream(true).start();
        if (!process.waitFor(45, java.util.concurrent.TimeUnit.SECONDS)) {
            process.destroyForcibly();
            throw new IOException("系统缓存清理超时，可能已部分完成；请重新扫描容量");
        }
        String output;
        try (InputStream in = process.getInputStream()) { output = new String(in.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8); }
        String lower = output.toLowerCase(Locale.ROOT);
        if (lower.contains("permission") || lower.contains("securityexception")) throw new IOException("系统未授权缓存清理，请使用手机存储设置");
        syncStorage();
        long after = new StatFs("/data").getAvailableBytes();
        // The requested target can exceed all reclaimable caches. PackageManager may
        // report failure after doing some work; retain the actual capacity observation.
        boolean completed = process.exitValue() == 0 && !lower.contains("fail") && !lower.contains("error");
        String note = completed
            ? "系统缓存清理结束；应用数据和账号由系统保留"
            : "系统未达到目标空闲量，可能仅清理了部分缓存；结果以容量变化为准";
        return new JSONObject().put("available_before", before.getAvailableBytes()).put("available_after", after)
            .put("logical_saved", 0).put("result_path", note).put("completed", completed);
    }
}
