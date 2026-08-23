package dev.lucasmoy.livesubs

import com.google.mlkit.common.model.DownloadConditions
import com.google.mlkit.nl.translate.TranslateLanguage
import com.google.mlkit.nl.translate.Translation
import com.google.mlkit.nl.translate.Translator
import com.google.mlkit.nl.translate.TranslatorOptions
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * On-device translation, via ML Kit.
 *
 * The desktop app uses Argos (Python + PyTorch), which is not portable to a
 * phone in any form. ML Kit is the equivalent trade here: models run
 * offline once downloaded (~30MB per language), translation costs
 * milliseconds, and nothing about the audio leaves the device. The price is
 * a dependency on Play Services -- stated in the README rather than
 * discovered at runtime.
 *
 * Deliberately blocking: it is called from a capture thread that has just
 * spent a second inside whisper, and the caption cannot be shown until the
 * translation exists anyway. Making it suspend would add a coroutine hop
 * and change nothing about the ordering.
 */
class Translations {
    private val clients = ConcurrentHashMap<String, Translator>()

    /** Whether ML Kit knows this language at all. */
    fun supports(code: String): Boolean = TranslateLanguage.fromLanguageTag(code) != null

    /**
     * Translate, or return null when the pair isn't supported or the model
     * isn't available offline yet (the caller then shows the original --
     * far better than showing nothing).
     */
    fun translate(text: String, from: String, to: String, timeoutMs: Long = 8_000): String? {
        if (from == to || text.isBlank()) return text
        val source = TranslateLanguage.fromLanguageTag(from) ?: return null
        val target = TranslateLanguage.fromLanguageTag(to) ?: return null
        val translator = clients.getOrPut("$source>$target") {
            Translation.getClient(
                TranslatorOptions.Builder()
                    .setSourceLanguage(source)
                    .setTargetLanguage(target)
                    .build()
            )
        }
        return awaitTask(timeoutMs) { latch, result ->
            // Download conditions with no requireWifi: someone turning this
            // on mid-meeting on mobile data wants the subtitles, not a
            // policy about networks.
            translator.downloadModelIfNeeded(DownloadConditions.Builder().build())
                .addOnSuccessListener {
                    translator.translate(text)
                        .addOnSuccessListener { translated ->
                            result[0] = translated
                            latch.countDown()
                        }
                        .addOnFailureListener { latch.countDown() }
                }
                .addOnFailureListener { latch.countDown() }
        }
    }

    /**
     * Fetch the models for a pair up front, so the first caption of a
     * meeting isn't the one that waits for a download. Returns whether the
     * pair is ready.
     */
    fun prepare(from: String, to: String, timeoutMs: Long = 120_000): Boolean {
        if (from == to) return true
        val source = TranslateLanguage.fromLanguageTag(from) ?: return false
        val target = TranslateLanguage.fromLanguageTag(to) ?: return false
        val translator = clients.getOrPut("$source>$target") {
            Translation.getClient(
                TranslatorOptions.Builder()
                    .setSourceLanguage(source)
                    .setTargetLanguage(target)
                    .build()
            )
        }
        return awaitTask(timeoutMs) { latch, result ->
            translator.downloadModelIfNeeded(DownloadConditions.Builder().build())
                .addOnSuccessListener {
                    result[0] = "ok"
                    latch.countDown()
                }
                .addOnFailureListener { latch.countDown() }
        } != null
    }

    fun close() {
        clients.values.forEach { it.close() }
        clients.clear()
    }

    /** ML Kit speaks Tasks; this waits for one without dragging in another dependency. */
    private fun awaitTask(timeoutMs: Long, block: (CountDownLatch, Array<String?>) -> Unit): String? {
        val latch = CountDownLatch(1)
        val result = arrayOfNulls<String>(1)
        block(latch, result)
        return if (latch.await(timeoutMs, TimeUnit.MILLISECONDS)) result[0] else null
    }
}
