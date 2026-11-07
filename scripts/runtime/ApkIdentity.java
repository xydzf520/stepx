import com.android.apksig.apk.ApkUtils;
import com.android.apksig.internal.apk.AndroidBinXmlParser;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.List;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/** Android's own binary XML parser; optional component metadata uses no native aapt. */
public final class ApkIdentity {
    private static String quote(String value) {
        StringBuilder out = new StringBuilder("\"");
        for (char ch : value.toCharArray()) {
            if (ch == '\\' || ch == '"') out.append('\\').append(ch);
            else if (ch < 32) out.append(String.format("\\u%04x", (int)ch));
            else out.append(ch);
        }
        return out.append('"').toString();
    }
    private static String metadata(byte[] manifest, String name) throws Exception {
        List<String> components = new ArrayList<>();
        long versionCode = -1;
        AndroidBinXmlParser parser = new AndroidBinXmlParser(ByteBuffer.wrap(manifest));
        for (int event = parser.getEventType(); event != AndroidBinXmlParser.EVENT_END_DOCUMENT; event = parser.next()) {
            if (event != AndroidBinXmlParser.EVENT_START_ELEMENT) continue;
            if (parser.getDepth() == 1 && parser.getName().equals("manifest")) {
                for (int i = 0; i < parser.getAttributeCount(); i++)
                    if ("http://schemas.android.com/apk/res/android".equals(parser.getAttributeNamespace(i)) && "versionCode".equals(parser.getAttributeName(i)))
                        versionCode = Integer.toUnsignedLong(parser.getAttributeIntValue(i));
            }
            if (parser.getDepth() != 3) continue;
            String type = parser.getName();
            if (!(type.equals("service") || type.equals("provider") || type.equals("activity"))) continue;
            StringBuilder item = new StringBuilder("{\"type\":").append(quote(type));
            for (int i = 0; i < parser.getAttributeCount(); i++) {
                if (!"http://schemas.android.com/apk/res/android".equals(parser.getAttributeNamespace(i))) continue;
                String key = parser.getAttributeName(i);
                if (!(key.equals("name") || key.equals("authorities") || key.equals("permission") || key.equals("enabled"))) continue;
                if (parser.getAttributeValueType(i) == AndroidBinXmlParser.VALUE_TYPE_STRING) {
                    item.append(',').append(quote(key)).append(':').append(quote(parser.getAttributeStringValue(i)));
                } else if (key.equals("enabled") && parser.getAttributeValueType(i) == AndroidBinXmlParser.VALUE_TYPE_BOOLEAN) {
                    item.append(",\"enabled\":").append(parser.getAttributeBooleanValue(i));
                }
            }
            components.add(item.append('}').toString());
        }
        return "{\"package\":" + quote(name) + ",\"versionCode\":" + versionCode + ",\"components\":[" + String.join(",", components) + "]}";
    }
    public static void main(String[] args) throws Exception {
        if (args.length < 1 || args.length > 2 || args.length == 2 && !args[1].equals("--manifest"))
            throw new IllegalArgumentException("Expected APK path and optional --manifest");
        try (ZipFile apk = new ZipFile(args[0])) {
            ZipEntry entry = apk.getEntry("AndroidManifest.xml");
            if (entry == null) throw new IllegalArgumentException("APK manifest is missing");
            try (InputStream stream = apk.getInputStream(entry)) {
                byte[] manifest = stream.readNBytes(2 * 1024 * 1024 + 1);
                if (manifest.length > 2 * 1024 * 1024) throw new IllegalArgumentException("APK manifest is too large");
                String name = ApkUtils.getPackageNameFromBinaryAndroidManifest(ByteBuffer.wrap(manifest));
                if (name == null || !name.matches("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+"))
                    throw new IllegalArgumentException("Invalid APK package name");
                System.out.println(args.length == 2 ? metadata(manifest, name) : name);
            }
        }
    }
}
