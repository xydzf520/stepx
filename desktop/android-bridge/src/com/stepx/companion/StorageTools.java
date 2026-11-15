package com.stepx.companion;

import android.os.StatFs;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.*;
import java.nio.file.*;
import java.nio.file.attribute.BasicFileAttributes;
import java.security.MessageDigest;
import java.util.*;

/** Read-only, bounded inspection of public files. Never follows symbolic links. */
final class StorageTools {
    static final String[] FOLDERS = {"Download", "Documents", "DCIM", "Pictures", "Movies", "Music"};
    static final int MAX_ENTRIES = 10000;
    static final long HASH_BUDGET = 256L * 1024 * 1024;

    static final class Item {
        final Path path;
        final long bytes, modified;
        Item(Path p, BasicFileAttributes a) { path = p; bytes = a.size(); modified = a.lastModifiedTime().toMillis(); }
        JSONObject json(Path root) throws Exception {
            return new JSONObject().put("path", "/sdcard/" + root.relativize(path))
                .put("bytes", bytes).put("modified_ms", modified);
        }
    }

    static JSONObject scan() throws Exception {
        Path root = new File("/sdcard").getCanonicalFile().toPath();
        if (!root.toString().equals("/storage/emulated/0")) throw new IOException("当前共享存储用户不受支持");
        long deadline = System.nanoTime() + 12_000_000_000L;
        List<Item> items = new ArrayList<>();
        JSONArray folders = new JSONArray();
        int[] visited = {0}, unreadable = {0};
        boolean[] limited = {false};
        long[] scannedBytes = {0};
        for (String name : FOLDERS) {
            Path dir = root.resolve(name);
            if (!Files.exists(dir, LinkOption.NOFOLLOW_LINKS)) continue;
            if (limited[0]) break;
            if (!Files.isDirectory(dir, LinkOption.NOFOLLOW_LINKS)) { unreadable[0]++; continue; }
            int before = items.size();
            long bytesBefore = scannedBytes[0];
            Files.walkFileTree(dir, new SimpleFileVisitor<Path>() {
                boolean limit() {
                    if (++visited[0] > MAX_ENTRIES || System.nanoTime() > deadline) limited[0] = true;
                    return limited[0];
                }
                @Override public FileVisitResult preVisitDirectory(Path p, BasicFileAttributes a) {
                    if (limit()) return FileVisitResult.TERMINATE;
                    if (p.getFileName().toString().startsWith(".")) return FileVisitResult.SKIP_SUBTREE;
                    return FileVisitResult.CONTINUE;
                }
                @Override public FileVisitResult visitFile(Path p, BasicFileAttributes a) {
                    if (limit()) return FileVisitResult.TERMINATE;
                    if (a.isRegularFile()) { items.add(new Item(p, a)); scannedBytes[0] += a.size(); }
                    return FileVisitResult.CONTINUE;
                }
                @Override public FileVisitResult visitFileFailed(Path p, IOException e) {
                    unreadable[0]++;
                    return limit() ? FileVisitResult.TERMINATE : FileVisitResult.CONTINUE;
                }
            });
            folders.put(new JSONObject().put("name", name).put("files", items.size() - before)
                .put("bytes", scannedBytes[0] - bytesBefore));
        }

        Map<Long, List<Item>> sizes = new TreeMap<>();
        for (Item item : items) if (item.bytes > 0) sizes.computeIfAbsent(item.bytes, k -> new ArrayList<>()).add(item);
        List<List<Item>> duplicates = new ArrayList<>();
        long remaining = HASH_BUDGET, duplicateBytes = 0;
        int hashed = 0, hashSkipped = 0;
        long hashDeadline = System.nanoTime() + 12_000_000_000L;
        for (List<Item> candidates : sizes.values()) {
            if (candidates.size() < 2) continue;
            Map<String, List<Item>> hashes = new TreeMap<>();
            for (Item item : candidates) {
                if (item.bytes > remaining || System.nanoTime() > hashDeadline) { hashSkipped++; continue; }
                remaining -= item.bytes;
                try {
                    BasicFileAttributes before = Files.readAttributes(item.path, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
                    if (!before.isRegularFile() || before.size() != item.bytes || before.lastModifiedTime().toMillis() != item.modified)
                        throw new IOException("文件已变化");
                    MessageDigest digest = MessageDigest.getInstance("SHA-256");
                    long read = 0;
                    try (InputStream in = Files.newInputStream(item.path, LinkOption.NOFOLLOW_LINKS)) {
                        byte[] buffer = new byte[65536];
                        int n;
                        while ((n = in.read(buffer)) != -1) {
                            read += n;
                            if (read > item.bytes || System.nanoTime() > hashDeadline) throw new IOException("达到扫描上限");
                            digest.update(buffer, 0, n);
                        }
                    }
                    BasicFileAttributes after = Files.readAttributes(item.path, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
                    if (read != item.bytes || after.size() != item.bytes || after.lastModifiedTime().toMillis() != item.modified)
                        throw new IOException("文件已变化");
                    String key = Base64.getEncoder().encodeToString(digest.digest());
                    hashes.computeIfAbsent(key, k -> new ArrayList<>()).add(item);
                    hashed++;
                } catch (IOException e) { hashSkipped++; }
            }
            for (List<Item> group : hashes.values()) if (group.size() > 1) {
                group.sort(Comparator.comparing(i -> i.path.toString()));
                duplicates.add(group);
                duplicateBytes += group.get(0).bytes * (group.size() - 1);
            }
        }
        items.sort(Comparator.comparingLong((Item i) -> i.bytes).reversed().thenComparing(i -> i.path.toString()));
        duplicates.sort(Comparator.comparingLong((List<Item> g) -> g.get(0).bytes * (g.size() - 1)).reversed());
        JSONArray largest = new JSONArray(), groups = new JSONArray(), documents = new JSONArray();
        for (int i = 0; i < Math.min(20, items.size()); i++) largest.put(items.get(i).json(root));
        for (Item item : items) {
            String name = item.path.getFileName().toString().toLowerCase(Locale.ROOT);
            if (name.endsWith(".gz")) name = name.substring(0, name.length() - 3);
            if (FileTools.TEXT_EXT.contains(ContextReadPolicy.fileExtension(name))) documents.put(item.json(root));
        }
        for (List<Item> group : duplicates) {
            JSONArray members = new JSONArray();
            for (Item item : group) members.put(item.json(root));
            groups.put(members);
        }
        StatFs data = new StatFs("/data");
        String fs = "未知";
        try (BufferedReader reader = new BufferedReader(new FileReader("/proc/mounts"))) {
            String line;
            while ((line = reader.readLine()) != null) {
                String[] fields = line.split(" ");
                if (fields.length > 2 && fields[1].equals("/data")) { fs = fields[2]; break; }
            }
        }
        return new JSONObject().put("total_bytes", data.getTotalBytes()).put("available_bytes", data.getAvailableBytes())
            .put("free_bytes", data.getFreeBytes()).put("filesystem", fs).put("scanned_files", items.size())
            .put("scanned_bytes", scannedBytes[0]).put("limited", limited[0]).put("unreadable", unreadable[0])
            .put("hashed_files", hashed).put("hash_skipped", hashSkipped).put("duplicate_bytes", duplicateBytes)
            .put("folders", folders).put("largest", largest).put("duplicates", groups).put("documents", documents)
            .put("scanned_at_ms", System.currentTimeMillis());
    }
}
