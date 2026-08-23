package dev.lucasmoy.recpocket

import android.annotation.SuppressLint
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import android.media.MediaRecorder
import android.media.projection.MediaProjection
import android.util.DisplayMetrics
import android.view.Surface
import java.io.File
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean

/**
 * One recording: audio (microphone, playback, or both mixed) and optionally
 * the screen, into a single MP4/M4A file.
 *
 * Why this is hand-built rather than `MediaRecorder`, which would be four
 * lines: `MediaRecorder` takes exactly **one** audio source. Recording your
 * voice *and* what the phone is playing -- the entire point of saving a call
 * -- means two `AudioRecord` streams added together (see [Mixing]), and once
 * the audio is PCM in this process it has to be encoded and muxed here too.
 *
 * The shape:
 *
 *     mic AudioRecord ──┐
 *                       ├─ mix ─► AAC encoder ─┐
 *     playback capture ─┘                       ├─► MediaMuxer ─► file
 *     VirtualDisplay ─► Surface ─► AVC encoder ─┘
 *
 * Two encoders feeding one muxer is the part that needs care: a muxer
 * refuses samples until *every* track has been added, so writing starts only
 * once both formats are known ([maybeStart]), and everything before that is
 * held back rather than thrown away.
 *
 * What Android will not allow, said plainly: an app that plays call audio
 * can opt out of playback capture, and phone/WhatsApp calls do. The far end
 * of a call therefore reaches the file through the microphone (speaker on)
 * or not at all -- no app on an unrooted phone gets around that, and this
 * one does not pretend to.
 */
class Recorder(
    private val config: Config,
    private val projection: MediaProjection?,
) {
    data class Config(
        val sources: Settings.Sources,
        val audio: Settings.AudioQuality,
        val video: Settings.VideoQuality?,
        /** Screen size and density, when the screen is being recorded. */
        val metrics: DisplayMetrics? = null,
    )

    private val running = AtomicBoolean(false)
    private var muxer: MediaMuxer? = null
    private var audioTrack = -1
    private var videoTrack = -1
    private var muxing = false
    private var expectedTracks = 1

    private var micRecord: AudioRecord? = null
    private var playbackRecord: AudioRecord? = null
    private var audioEncoder: MediaCodec? = null
    private var videoEncoder: MediaCodec? = null
    private var virtualDisplay: android.hardware.display.VirtualDisplay? = null
    private var audioThread: Thread? = null
    private var videoThread: Thread? = null

    /** The first thing that went wrong, reported when [stop] is called. */
    @Volatile
    var failure: String? = null
        private set

    val isRunning: Boolean get() = running.get()

    @SuppressLint("MissingPermission")
    fun start(output: File) {
        require(!running.getAndSet(true)) { "ya está grabando" }
        val wantsPlayback = config.sources != Settings.Sources.MIC
        if (wantsPlayback && projection == null) {
            running.set(false)
            throw IllegalStateException("grabar la salida necesita el permiso de captura de pantalla")
        }
        val wantsVideo = config.video != null && config.metrics != null
        if (wantsVideo && projection == null) {
            running.set(false)
            throw IllegalStateException("grabar la pantalla necesita el permiso de captura")
        }

        muxer = MediaMuxer(output.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
        expectedTracks = if (wantsVideo) 2 else 1

        startAudio()
        if (wantsVideo) startVideo()
    }

    // ---- audio ----------------------------------------------------------

    private val channelMask
        get() = if (config.audio.channels == 2) {
            AudioFormat.CHANNEL_IN_STEREO
        } else {
            AudioFormat.CHANNEL_IN_MONO
        }

    @SuppressLint("MissingPermission")
    private fun startAudio() {
        val format = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setSampleRate(config.audio.sampleRate)
            .setChannelMask(channelMask)
            .build()
        val minBuffer = AudioRecord.getMinBufferSize(
            config.audio.sampleRate,
            channelMask,
            AudioFormat.ENCODING_PCM_16BIT,
        ).coerceAtLeast(4096)
        // Four times the minimum: a foreground service that also encodes
        // video can be late by a scheduling quantum, and an AudioRecord that
        // overruns drops audio silently.
        val bufferBytes = minBuffer * 4

        if (config.sources != Settings.Sources.PLAYBACK) {
            micRecord = AudioRecord.Builder()
                // VOICE_RECOGNITION rather than MIC: it is the source that
                // does *not* get the aggressive noise suppression and
                // automatic gain that make a recorded call sound like it is
                // underwater.
                .setAudioSource(MediaRecorder.AudioSource.VOICE_RECOGNITION)
                .setAudioFormat(format)
                .setBufferSizeInBytes(bufferBytes)
                .build()
        }
        if (config.sources != Settings.Sources.MIC) {
            val capture = AudioPlaybackCaptureConfiguration.Builder(projection!!)
                // Media and games, which is everything an app plays that it
                // has not marked as private. Call audio is excluded by the
                // apps themselves; see the class comment.
                .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                .addMatchingUsage(AudioAttributes.USAGE_GAME)
                .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                .build()
            playbackRecord = AudioRecord.Builder()
                .setAudioFormat(format)
                .setBufferSizeInBytes(bufferBytes)
                .setAudioPlaybackCaptureConfig(capture)
                .build()
        }

        val encoderFormat = MediaFormat.createAudioFormat(
            MediaFormat.MIMETYPE_AUDIO_AAC,
            config.audio.sampleRate,
            config.audio.channels,
        ).apply {
            setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
            setInteger(MediaFormat.KEY_BIT_RATE, config.audio.bitrate)
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, bufferBytes)
        }
        val encoder = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).apply {
            configure(encoderFormat, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            start()
        }
        audioEncoder = encoder

        micRecord?.startRecording()
        playbackRecord?.startRecording()

        audioThread = Thread({ pumpAudio(encoder, bufferBytes) }, "recpocket-audio").also { it.start() }
    }

    private fun pumpAudio(encoder: MediaCodec, bufferBytes: Int) {
        val mic = ByteArray(bufferBytes)
        val playback = ByteArray(bufferBytes)
        val info = MediaCodec.BufferInfo()
        // The encoder wants a monotonic timeline in microseconds. Deriving
        // it from the sample count rather than the clock keeps audio and
        // video in step even when a read is late.
        var samplesWritten = 0L
        val bytesPerSecond = config.audio.sampleRate * config.audio.channels * 2

        try {
            while (running.get()) {
                val micRead = micRecord?.read(mic, 0, mic.size)?.coerceAtLeast(0) ?: 0
                val playbackRead = playbackRecord?.read(playback, 0, playback.size)?.coerceAtLeast(0) ?: 0
                if (micRead == 0 && playbackRead == 0) {
                    Thread.sleep(5)
                    continue
                }
                val chunk = when {
                    micRead > 0 && playbackRead > 0 ->
                        // Playback capture arrives much hotter than a
                        // microphone; without this the far end buries the
                        // near one.
                        Mixing.mix(mic, micRead, playback, playbackRead, gainB = 0.7f)
                    micRead > 0 -> mic.copyOf(micRead)
                    else -> playback.copyOf(playbackRead)
                }

                var offset = 0
                while (offset < chunk.size) {
                    val index = encoder.dequeueInputBuffer(10_000)
                    if (index < 0) {
                        drain(encoder, info, video = false)
                        continue
                    }
                    val input = encoder.getInputBuffer(index) ?: continue
                    input.clear()
                    val count = minOf(input.capacity(), chunk.size - offset)
                    input.put(chunk, offset, count)
                    val presentationUs = samplesWritten * 1_000_000L / bytesPerSecond
                    encoder.queueInputBuffer(index, 0, count, presentationUs, 0)
                    offset += count
                    samplesWritten += count
                }
                drain(encoder, info, video = false)
            }
            // Tell the encoder the stream ended, then flush what is left.
            val index = encoder.dequeueInputBuffer(100_000)
            if (index >= 0) {
                encoder.queueInputBuffer(
                    index, 0, 0,
                    samplesWritten * 1_000_000L / bytesPerSecond,
                    MediaCodec.BUFFER_FLAG_END_OF_STREAM,
                )
            }
            drain(encoder, info, video = false, untilEnd = true)
        } catch (e: Throwable) {
            note(e)
        }
    }

    // ---- video ----------------------------------------------------------

    private fun startVideo() {
        val quality = config.video!!
        val metrics = config.metrics!!
        // Scaled to the requested height, keeping the aspect ratio, and both
        // dimensions rounded to even numbers -- an odd width is rejected by
        // most hardware AVC encoders.
        val scale = minOf(1f, quality.maxHeight.toFloat() / metrics.heightPixels)
        val width = ((metrics.widthPixels * scale).toInt() / 2) * 2
        val height = ((metrics.heightPixels * scale).toInt() / 2) * 2

        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height).apply {
            setInteger(
                MediaFormat.KEY_COLOR_FORMAT,
                MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface,
            )
            setInteger(MediaFormat.KEY_BIT_RATE, quality.bitrate)
            setInteger(MediaFormat.KEY_FRAME_RATE, quality.fps)
            // Two seconds between keyframes: seeking stays usable and the
            // bitrate is not spent on I-frames of a mostly static screen.
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 2)
        }
        val encoder = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        encoder.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        val surface: Surface = encoder.createInputSurface()
        encoder.start()
        videoEncoder = encoder

        virtualDisplay = projection!!.createVirtualDisplay(
            "recpocket",
            width,
            height,
            metrics.densityDpi,
            android.hardware.display.DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
            surface,
            null,
            null,
        )

        videoThread = Thread({
            val info = MediaCodec.BufferInfo()
            try {
                while (running.get()) drain(encoder, info, video = true)
                encoder.signalEndOfInputStream()
                drain(encoder, info, video = true, untilEnd = true)
            } catch (e: Throwable) {
                note(e)
            }
        }, "recpocket-video").also { it.start() }
    }

    // ---- muxing ---------------------------------------------------------

    /**
     * Move whatever the encoder has produced into the muxer.
     *
     * Synchronised because both encoder threads call it and a `MediaMuxer`
     * is explicitly not thread-safe -- two tracks writing at once corrupts
     * the file rather than failing loudly.
     */
    private fun drain(
        encoder: MediaCodec,
        info: MediaCodec.BufferInfo,
        video: Boolean,
        untilEnd: Boolean = false,
    ) {
        while (true) {
            val index = encoder.dequeueOutputBuffer(info, if (untilEnd) 50_000 else 5_000)
            when {
                index == MediaCodec.INFO_TRY_AGAIN_LATER -> if (untilEnd) continue else return
                index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> synchronized(this) {
                    val track = muxer?.addTrack(encoder.outputFormat) ?: return
                    if (video) videoTrack = track else audioTrack = track
                    maybeStart()
                }
                index >= 0 -> {
                    val buffer: ByteBuffer? = encoder.getOutputBuffer(index)
                    val isConfig = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                    if (buffer != null && info.size > 0 && !isConfig) {
                        synchronized(this) {
                            val track = if (video) videoTrack else audioTrack
                            if (muxing && track >= 0) {
                                buffer.position(info.offset)
                                buffer.limit(info.offset + info.size)
                                muxer?.writeSampleData(track, buffer, info)
                            }
                        }
                    }
                    encoder.releaseOutputBuffer(index, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return
                }
            }
        }
    }

    /** A muxer refuses samples until every track it will carry has been
     *  added, so writing waits for both formats. */
    private fun maybeStart() {
        if (muxing) return
        val ready = listOf(audioTrack, videoTrack).count { it >= 0 }
        if (ready < expectedTracks) return
        muxer?.start()
        muxing = true
    }

    private fun note(e: Throwable) {
        if (failure == null) failure = e.message ?: e.javaClass.simpleName
    }

    /**
     * Stop, and finish the file.
     *
     * Order matters and is the difference between a playable file and a
     * zero-byte one: the encoders are told the stream ended, their threads
     * are joined so the last samples reach the muxer, and only then is the
     * muxer stopped.
     */
    fun stop() {
        if (!running.getAndSet(false)) return
        runCatching { audioThread?.join(4000) }
        runCatching { videoThread?.join(4000) }

        runCatching { micRecord?.stop() }
        runCatching { micRecord?.release() }
        runCatching { playbackRecord?.stop() }
        runCatching { playbackRecord?.release() }
        micRecord = null
        playbackRecord = null

        runCatching { audioEncoder?.stop() }
        runCatching { audioEncoder?.release() }
        runCatching { videoEncoder?.stop() }
        runCatching { videoEncoder?.release() }
        audioEncoder = null
        videoEncoder = null

        runCatching { virtualDisplay?.release() }
        virtualDisplay = null

        synchronized(this) {
            if (muxing) runCatching { muxer?.stop() }
            runCatching { muxer?.release() }
            muxer = null
            muxing = false
            audioTrack = -1
            videoTrack = -1
        }
    }
}
