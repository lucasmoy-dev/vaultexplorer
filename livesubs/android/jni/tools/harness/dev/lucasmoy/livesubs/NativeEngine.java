package dev.lucasmoy.livesubs;

/**
 * A stand-in for the Kotlin `object NativeEngine`, with the same package and
 * class name so the JNI symbol names match exactly. Lets the whole native
 * path (VAD, whisper, JSON) be exercised on the host, with a real audio
 * file, without a phone in the loop.
 */
public final class NativeEngine {
    static { System.loadLibrary("livesubs"); }

    public static final NativeEngine INSTANCE = new NativeEngine();

    public native int frameSize();
    public native long loadModel(String path, String modelName);
    public native void freeModel(long handle);
    public native long createStream(long engine, float sensitivity);
    public native void freeStream(long handle);
    public native void setSensitivity(long handle, float sensitivity);
    public native String feed(long handle, float[] samples, String language);
    public native String flush(long handle, String language);
}
