package org.andromeda.link.screen

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import android.os.Binder
import android.os.ParcelFileDescriptor
import android.os.Process
import android.util.Base64
import android.util.Log
import java.io.IOException
import java.io.OutputStream
import java.util.concurrent.LinkedBlockingQueue
import kotlin.concurrent.thread

/**
 * Where [InputRelay] reads Link's mirror input: each `content read` gets a pipe whose write end stays here. The
 * manifest lets only holders of INTERACT_ACROSS_USERS_FULL in, and this checks for the shell or root as well.
 */
class InputCommandProvider : ContentProvider() {
    override fun onCreate() = true

    override fun openFile(
        uri: Uri,
        mode: String,
    ): ParcelFileDescriptor {
        val caller = Binder.getCallingUid()
        if (caller != Process.SHELL_UID && caller != Process.ROOT_UID) throw SecurityException("uid $caller is not the shell")
        val (read, write) = ParcelFileDescriptor.createPipe()
        connected(ParcelFileDescriptor.AutoCloseOutputStream(write))
        return read
    }

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor? = null

    override fun getType(uri: Uri): String? = null

    override fun insert(
        uri: Uri,
        values: ContentValues?,
    ): Uri? = null

    override fun delete(
        uri: Uri,
        selection: String?,
        selectionArgs: Array<out String>?,
    ) = 0

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ) = 0

    companion object {
        @Volatile
        private var relay: OutputStream? = null

        /** Lines for the helper; a stalled helper fills this and new lines are dropped, never blocking the caller. */
        private val lines = LinkedBlockingQueue<String>(QUEUE)

        /** Whether the shell helper reads input now. */
        val connected: Boolean get() = relay != null

        private fun connected(stream: OutputStream) {
            val old = relay
            relay = stream
            old?.close()
            lines.clear()
            Log.i("ScreenInput", "the shell helper reads input")
        }

        init {
            thread(isDaemon = true, name = "input-relay") {
                while (true) {
                    val line = lines.take()
                    val stream = relay ?: continue
                    try {
                        stream.write(line.toByteArray())
                        stream.flush()
                    } catch (_: IOException) {
                        if (relay === stream) relay = null
                    }
                }
            }
        }

        /** One command to the helper; false when none reads. Coordinates in 1/10000 of the screen. */
        fun send(
            action: String,
            x: Int? = null,
            y: Int? = null,
            x2: Int? = null,
            y2: Int? = null,
            code: Int? = null,
            meta: Int? = null,
            text: String? = null,
        ): Boolean {
            if (relay == null) return false
            val encoded = text?.let { Base64.encodeToString(it.toByteArray(), Base64.NO_WRAP) }.orEmpty()
            val line = listOf(action, x ?: 0, y ?: 0, x2 ?: 0, y2 ?: 0, code ?: 0, meta ?: 0, encoded).joinToString(" ")
            if (!lines.offer(line + "\n")) Log.i("ScreenInput", "the helper is behind: dropped $action")
            return true
        }

        private const val QUEUE = 256
    }
}
