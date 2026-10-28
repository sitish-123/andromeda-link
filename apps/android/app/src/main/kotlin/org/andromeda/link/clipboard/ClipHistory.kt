package org.andromeda.link.clipboard

import android.util.Log
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.updateAndGet
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** A clip sent or received; [text] is null for one without text, which cannot be copied again. */
data class ClipEntry(
    val time: Long,
    val incoming: Boolean,
    val desktopName: String?,
    val text: String?,
    val html: String?,
    val mime: String,
)

/** The last [MAX] clips across restarts, newest first, in one JSON file rewritten whole. */
class ClipHistory(
    private val file: File,
) {
    private val _entries = MutableStateFlow(load())
    val entries: StateFlow<List<ClipEntry>> = _entries.asStateFlow()

    @Synchronized
    fun add(entry: ClipEntry) {
        val entries = _entries.updateAndGet { (listOf(entry) + it).take(MAX) }
        val temporary = File(file.parentFile, "${file.name}.tmp")
        temporary.writeText(JSONArray(entries.map(::json)).toString())
        if (!temporary.renameTo(file)) Log.w(TAG, "could not replace the clip history")
    }

    private fun load(): List<ClipEntry> =
        try {
            if (!file.exists()) {
                emptyList()
            } else {
                JSONArray(file.readText()).let { array -> (0 until array.length()).map { entry(array.getJSONObject(it)) } }
            }
        } catch (error: Exception) {
            Log.w(TAG, "reading the clip history: $error")
            emptyList()
        }

    private fun json(entry: ClipEntry) =
        JSONObject()
            .put("time", entry.time)
            .put("incoming", entry.incoming)
            .put("desktop", entry.desktopName ?: JSONObject.NULL)
            .put("text", entry.text ?: JSONObject.NULL)
            .put("html", entry.html ?: JSONObject.NULL)
            .put("mime", entry.mime)

    private fun entry(json: JSONObject) =
        ClipEntry(
            time = json.getLong("time"),
            incoming = json.getBoolean("incoming"),
            desktopName = json.optString("desktop").takeUnless { json.isNull("desktop") },
            text = json.optString("text").takeUnless { json.isNull("text") },
            html = json.optString("html").takeUnless { json.isNull("html") },
            mime = json.getString("mime"),
        )

    private companion object {
        const val TAG = "ClipHistory"
        const val MAX = 20
    }
}
