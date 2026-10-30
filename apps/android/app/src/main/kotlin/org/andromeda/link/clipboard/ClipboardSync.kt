package org.andromeda.link.clipboard

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.net.Uri
import android.text.Html
import androidx.core.content.edit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.ClipPart
import org.andromeda.link.core.domain.IncomingClip

/**
 * Desktop to phone: text and HTML are set at once, other types as a `content://` URI [ClipProvider] pulls when an app
 * reads it. A clip set here carries [LABEL], so reading it back never offers it again. With "Clear synced clips" on, a
 * desktop clip is cleared after two minutes unless the clipboard changed since.
 */
class ClipboardSync(
    private val context: Context,
    private val repository: LinkRepository,
    private val history: ClipHistory,
    private val scope: CoroutineScope,
) {
    private val clipboard = context.getSystemService(ClipboardManager::class.java)
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private var clear: Job? = null

    var clearsSynced: Boolean
        get() = prefs.getBoolean(KEY_CLEAR, false)
        set(value) = prefs.edit { putBoolean(KEY_CLEAR, value) }

    var sendsScreenshots: Boolean
        get() = prefs.getBoolean(KEY_SCREENSHOTS, true)
        set(value) = prefs.edit { putBoolean(KEY_SCREENSHOTS, value) }

    fun start() {
        scope.launch { repository.clips.collect(::apply) }
    }

    /**
     * Sends every item's original content, not a URI coerced to text, and returns how many desktops got it now. Text
     * items are joined; of other types, the first item of each is sent.
     */
    suspend fun offer(clip: ClipData): Result<Int> {
        if (isFromDesktop(clip)) return Result.success(0)
        return offer(parts(clip))
    }

    /** Sends selected text, as "Send to desktop" does. */
    suspend fun offerText(text: String): Result<Int> = offer(listOf(ClipPart.Text(TEXT_MIME, text)))

    suspend fun offerImage(
        uri: Uri,
        mime: String,
    ): Result<Int> = offer(listOf(ClipPart.Content(mime, uri)))

    private suspend fun offer(parts: List<ClipPart>): Result<Int> {
        if (parts.isEmpty()) return Result.success(0)
        return repository.offerClip(parts).onSuccess {
            val text = parts.firstNotNullOfOrNull { (it as? ClipPart.Text)?.takeIf { part -> part.mime == TEXT_MIME }?.text }
            val html = parts.firstNotNullOfOrNull { (it as? ClipPart.Text)?.takeIf { part -> part.mime == HTML_MIME }?.text }
            history.add(ClipEntry(System.currentTimeMillis(), false, null, text, html, parts.first().mime))
        }
    }

    /** Puts a history entry back on the clipboard; one from a desktop keeps [LABEL], so it is not sent back. */
    fun copyAgain(entry: ClipEntry) {
        val text = entry.text ?: return
        val mimes = listOfNotNull(ClipDescription.MIMETYPE_TEXT_PLAIN, entry.html?.let { ClipDescription.MIMETYPE_TEXT_HTML })
        val label = if (entry.incoming) LABEL else HISTORY_LABEL
        clipboard.setPrimaryClip(ClipData(ClipDescription(label, mimes.toTypedArray()), ClipData.Item(text, entry.html)))
    }

    /** The clipboard changed here; a desktop clip is no longer there to clear. */
    fun changedLocally() {
        clear?.cancel()
        clear = null
    }

    private fun parts(clip: ClipData): List<ClipPart> {
        val texts = mutableListOf<String>()
        var html: String? = null
        val contents = linkedMapOf<String, ClipPart.Content>()
        for (index in 0 until clip.itemCount) {
            val item = clip.getItemAt(index)
            val uri = item.uri?.takeUnless(ClipProvider::owns)
            if (item.uri != null && uri == null) continue
            if (uri != null) {
                val mime =
                    context.contentResolver.getType(uri)?.takeUnless { '*' in it }
                        ?: concreteMimes(clip.description).firstOrNull { !it.startsWith("text/") }
                if (mime != null) contents.putIfAbsent(mime, ClipPart.Content(mime, uri))
            }
            if (html == null) html = item.htmlText
            val text = item.text?.toString() ?: if (uri == null) item.coerceToText(context)?.toString() else null
            text?.takeIf { it.isNotEmpty() }?.let(texts::add)
        }
        val text = texts.takeIf { it.isNotEmpty() }?.joinToString("\n")?.let { ClipPart.Text(TEXT_MIME, it) }
        return listOfNotNull(text, html?.let { ClipPart.Text(HTML_MIME, it) }) + contents.values
    }

    private suspend fun apply(clip: IncomingClip) {
        val pulledHtml =
            if (HTML_MIME in clip.mimes) {
                repository.pullClipText(clip.desktopId, clip.clipId, HTML_MIME, MAX_HTML_BYTES)
            } else {
                null
            }
        val plain = clip.mimes.firstOrNull { it.startsWith("text/plain") }
        val text =
            clip.text
                ?: plain?.let { repository.pullClipText(clip.desktopId, clip.clipId, it, MAX_TEXT_BYTES) }
                ?: pulledHtml?.takeIf { plain == null }?.let { Html.fromHtml(it, Html.FROM_HTML_MODE_COMPACT).toString() }
        // ClipData.Item takes HTML only with text; plain text over the limit goes as a URI instead.
        val html = pulledHtml?.takeIf { text != null }
        val other = clip.mimes.firstOrNull { !it.startsWith("text/") } ?: clip.mimes.first().takeIf { text == null }
        val uri = other?.let { ClipProvider.uri(clip.desktopId, clip.clipId, it) }
        val mimes =
            listOfNotNull(
                text?.let { ClipDescription.MIMETYPE_TEXT_PLAIN },
                html?.let { ClipDescription.MIMETYPE_TEXT_HTML },
                other,
            )
        clipboard.setPrimaryClip(ClipData(ClipDescription(LABEL, mimes.toTypedArray()), ClipData.Item(text, html, null, uri)))
        val desktop =
            repository.desktops.value
                .firstOrNull { it.id == clip.desktopId }
                ?.name
        history.add(ClipEntry(System.currentTimeMillis(), true, desktop, text, html, mimes.firstOrNull() ?: clip.mimes.first()))
        clear?.cancel()
        clear =
            if (clearsSynced) {
                scope.launch {
                    delay(CLEAR_AFTER_MS)
                    clipboard.clearPrimaryClip()
                }
            } else {
                null
            }
    }

    companion object {
        const val LABEL = "Andromeda Link"
        private const val HISTORY_LABEL = "Andromeda Link history"
        private const val PREFS = "clipboard"
        private const val KEY_CLEAR = "clear_synced"
        private const val KEY_SCREENSHOTS = "send_screenshots"
        private const val TEXT_MIME = "text/plain;charset=utf-8"
        private const val HTML_MIME = "text/html"
        private const val CLEAR_AFTER_MS = 120_000L

        /** Clip data travels through binder, which fails a transaction near 1 MiB. */
        private const val MAX_HTML_BYTES = 256 * 1024

        /** With [MAX_HTML_BYTES], as UTF-16 in one transaction, this stays under the binder limit. */
        private const val MAX_TEXT_BYTES = 128 * 1024

        fun isFromDesktop(clip: ClipData): Boolean = clip.description.label == LABEL

        private fun concreteMimes(description: ClipDescription): List<String> =
            (0 until description.mimeTypeCount).map(description::getMimeType).filter { '*' !in it }
    }
}
