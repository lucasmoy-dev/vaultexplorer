import dev.lucasmoy.livesubs.NativeEngine;
import java.io.DataInputStream;
import java.io.FileInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;

/** Feeds a 16kHz mono WAV through the native pipeline, like a capture thread would. */
public class Harness {
    public static void main(String[] args) throws Exception {
        String modelPath = args[0], modelName = args[1], wavPath = args[2];
        String language = args.length > 3 && !args[3].equals("auto") ? args[3] : null;

        NativeEngine engine = NativeEngine.INSTANCE;
        int frame = engine.frameSize();
        System.out.println("frameSize=" + frame);

        long model = engine.loadModel(modelPath, modelName);
        if (model == 0) throw new IllegalStateException("loadModel returned 0");
        long stream = engine.createStream(model, 1.0f);
        if (stream == 0) throw new IllegalStateException("createStream returned 0");

        float[] samples = readWav(wavPath);
        System.out.println("samples=" + samples.length);
        int captions = 0;
        for (int offset = 0; offset + frame <= samples.length; offset += frame) {
            float[] chunk = new float[frame];
            System.arraycopy(samples, offset, chunk, 0, frame);
            String json = engine.feed(stream, chunk, language);
            if (json != null) { System.out.println("CAPTION " + json); captions++; }
        }
        // Silence at the end, so the VAD closes the utterance like a real pause.
        float[] quiet = new float[frame];
        for (int i = 0; i < 60; i++) {
            String json = engine.feed(stream, quiet, language);
            if (json != null) { System.out.println("CAPTION " + json); captions++; }
        }
        String tail = engine.flush(stream, language);
        if (tail != null) { System.out.println("CAPTION " + tail); captions++; }

        engine.freeStream(stream);
        engine.freeModel(model);
        System.out.println("captions=" + captions);
    }

    /** 16-bit PCM WAV -> float, enough for a test fixture. */
    private static float[] readWav(String path) throws Exception {
        byte[] bytes = Files.readAllBytes(Paths.get(path));
        int offset = 12;
        while (offset + 8 <= bytes.length) {
            String id = new String(bytes, offset, 4, "US-ASCII");
            int size = (bytes[offset + 4] & 0xFF) | ((bytes[offset + 5] & 0xFF) << 8)
                     | ((bytes[offset + 6] & 0xFF) << 16) | ((bytes[offset + 7] & 0xFF) << 24);
            int body = offset + 8;
            if (id.equals("data")) {
                int count = Math.min(size, bytes.length - body) / 2;
                float[] out = new float[count];
                for (int i = 0; i < count; i++) {
                    int lo = bytes[body + 2 * i] & 0xFF, hi = bytes[body + 2 * i + 1];
                    out[i] = ((short) ((hi << 8) | lo)) / 32768f;
                }
                return out;
            }
            offset = body + size + (size & 1);
        }
        throw new IllegalStateException("no data chunk");
    }
}
