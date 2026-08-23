package dev.lucasmoy.livesubs

import android.annotation.SuppressLint
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.MediaRecorder
import android.media.projection.MediaProjection

/**
 * The two audio sources, both delivered as 16kHz mono float -- exactly what
 * whisper wants, so nothing has to resample.
 *
 * The microphone is straightforward. "System audio" is not: Android has no
 * equivalent of the desktop's monitor source. The only way in is
 * [AudioPlaybackCaptureConfiguration] (Android 10+), which comes with a
 * hard limit worth being loud about: an app's playback is only capturable
 * if its audio policy allows it. `USAGE_VOICE_COMMUNICATION` -- what every
 * calling app uses -- is never capturable, and apps can opt out entirely.
 * So this captures YouTube, videos, browsers and podcasts, and cannot
 * capture the other side of a Meet/Zoom/WhatsApp call. There is no API that
 * can; the workaround is the phone's speaker and the microphone.
 */
object Capture {
    const val SAMPLE_RATE = 16_000

    private fun bufferSize(): Int {
        val minimum = AudioRecord.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_FLOAT,
        )
        // Four times the minimum: the reader thread also runs whisper, and
        // during a transcription nothing is draining this buffer. Too small
        // and the tail of an utterance is dropped on the floor.
        return (if (minimum > 0) minimum else 4096) * 4
    }

    private fun format(): AudioFormat = AudioFormat.Builder()
        .setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
        .setSampleRate(SAMPLE_RATE)
        .setChannelMask(AudioFormat.CHANNEL_IN_MONO)
        .build()

    /** The user's own voice. */
    @SuppressLint("MissingPermission") // the service checks RECORD_AUDIO before calling
    fun microphone(): AudioRecord = AudioRecord.Builder()
        // VOICE_RECOGNITION, not MIC: it is the source tuned for speech,
        // with the platform's noise suppression and no AGC surprises.
        .setAudioSource(MediaRecorder.AudioSource.VOICE_RECOGNITION)
        .setAudioFormat(format())
        .setBufferSizeInBytes(bufferSize())
        .build()

    /** What other apps are playing, as far as they allow. */
    @SuppressLint("MissingPermission")
    fun playback(projection: MediaProjection): AudioRecord {
        val configuration = AudioPlaybackCaptureConfiguration.Builder(projection)
            // Everything that is capturable at all: media (video, music),
            // games, and unknown (which is where a lot of web audio lands).
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .build()
        return AudioRecord.Builder()
            .setAudioFormat(format())
            .setBufferSizeInBytes(bufferSize())
            .setAudioPlaybackCaptureConfig(configuration)
            .build()
    }
}
