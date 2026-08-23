import dev.lucasmoy.ytpocket.Native;

/** Calls every JNI entry point the app uses, in the order the app uses them. */
public class Harness {
    public static void main(String[] args) throws Exception {
        String query = args.length > 0 ? args[0] : "rick astley never gonna give you up";
        Native n = Native.INSTANCE;

        // Same first call the app makes: rustypipe's cache must point at a
        // writable directory (on Android the default is `/`).
        String cache = System.getProperty("java.io.tmpdir") + "/ytpocket-harness-cache";
        new java.io.File(cache).mkdirs();
        n.initCache(cache);
        System.out.println("CACHE " + cache);

        String hits = n.search(query, 5);
        System.out.println("SEARCH " + shorten(hits));
        if (hits == null || hits.startsWith("{\"error")) throw new IllegalStateException("search failed");

        // First video id out of the JSON, without pulling in a JSON library
        // for a harness.
        String id = between(hits, "\"id\":\"", "\"");
        System.out.println("FIRST ID " + id);

        String resolved = n.resolve(id);
        System.out.println("RESOLVE " + shorten(resolved));
        if (resolved == null || resolved.startsWith("{\"error")) throw new IllegalStateException("resolve failed");

        String title = between(resolved, "\"title\":\"", "\",");
        System.out.println("MP3 NAME " + n.fileName(title, "mp3"));
        System.out.println("MP4 NAME " + n.fileName(title, "mp4"));
        // The nasty cases, since this is the app's promise.
        System.out.println("SLASHES  " + n.fileName("AC/DC: Back In Black? \"live\"", "mp3"));
        System.out.println("EMPTY    " + n.fileName("   ", "mp3"));
        System.out.println("OK");
    }

    private static String shorten(String text) {
        if (text == null) return "null";
        return text.length() <= 220 ? text : text.substring(0, 220) + "…";
    }

    private static String between(String text, String open, String close) {
        int start = text.indexOf(open);
        if (start < 0) throw new IllegalStateException("missing " + open);
        start += open.length();
        int end = text.indexOf(close, start);
        return end < 0 ? text.substring(start) : text.substring(start, end);
    }
}
