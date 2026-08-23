package dev.lucasmoy.ytpocket;

/**
 * A stand-in for the Kotlin `object Native`, with the same package and class
 * name so the JNI symbols match exactly. Lets the whole native surface be
 * exercised on the host -- real YouTube, real transcode -- without a phone.
 */
public final class Native {
    static { System.loadLibrary("ytpocket"); }

    public static final Native INSTANCE = new Native();

    public native void initCache(String dir);
    public native String search(String query, int limit);
    public native String resolve(String video);
    public native String fileName(String title, String ext);
    public native String transcodeMp3(String source, String destination, String title, String artist);
}
