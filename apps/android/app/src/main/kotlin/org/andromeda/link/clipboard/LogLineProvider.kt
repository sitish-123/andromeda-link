package org.andromeda.link.clipboard

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import android.os.Binder
import android.os.ParcelFileDescriptor
import android.os.Process

/**
 * Where [LogHelper] writes its lines: each `content write` opens a pipe whose read end goes to [LogAccess]. The
 * manifest lets only holders of INTERACT_ACROSS_USERS_FULL in, and this checks for the shell or root as well, so no
 * app can feed the watcher lines.
 */
class LogLineProvider : ContentProvider() {
    override fun onCreate() = true

    override fun openFile(
        uri: Uri,
        mode: String,
    ): ParcelFileDescriptor {
        val caller = Binder.getCallingUid()
        if (caller != Process.SHELL_UID && caller != Process.ROOT_UID) throw SecurityException("uid $caller is not the shell")
        val (read, write) = ParcelFileDescriptor.createPipe()
        LogAccess.helperConnected(read)
        return write
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
}
