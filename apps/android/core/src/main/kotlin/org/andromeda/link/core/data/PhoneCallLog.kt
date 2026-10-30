package org.andromeda.link.core.data

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.CallLog.Calls
import android.telecom.TelecomManager
import org.andromeda.link.ffi.CallKind
import org.andromeda.link.ffi.PhoneCall
import org.andromeda.link.ffi.TextOutcome

/**
 * The phone's call log, newest first, and calls desktops place through the phone; the audio stays on the phone.
 * [calls] and [dial] are called off the main thread; [changed] runs on it, at most once per [SETTLE_MS] of changes.
 */
class PhoneCallLog(
    private val context: Context,
    private val changed: () -> Unit,
) {
    private val main = Handler(Looper.getMainLooper())
    private val notify = Runnable { changed() }
    private var watching = false

    fun calls(
        offset: UInt,
        limit: UInt,
    ): List<PhoneCall>? {
        if (!granted(Manifest.permission.READ_CALL_LOG)) return null
        val calls = mutableListOf<PhoneCall>()
        context.contentResolver
            .query(
                Calls.CONTENT_URI,
                arrayOf(Calls.NUMBER, Calls.CACHED_NAME, Calls.DATE, Calls.DURATION, Calls.TYPE),
                null,
                null,
                "${Calls.DATE} DESC, ${Calls._ID} DESC",
            )?.use { cursor ->
                if (!cursor.moveToPosition(offset.toInt())) return@use
                do {
                    calls +=
                        PhoneCall(
                            number = cursor.getString(0).orEmpty(),
                            name = cursor.getString(1)?.takeIf { it.isNotBlank() },
                            date = cursor.getLong(2).toULong(),
                            duration = cursor.getLong(3).coerceIn(0, UInt.MAX_VALUE.toLong()).toUInt(),
                            kind = kind(cursor.getInt(4)),
                        )
                } while (calls.size < limit.toInt() && cursor.moveToNext())
            }
        return calls
    }

    /** Places a call to [number] as the phone's own dialer would, without opening an activity. */
    fun dial(number: String): TextOutcome {
        if (!granted(Manifest.permission.CALL_PHONE)) return TextOutcome.DENIED
        val telecom = context.getSystemService(TelecomManager::class.java) ?: return TextOutcome.FAILED
        return runCatching { telecom.placeCall(Uri.fromParts("tel", number, null), Bundle()) }
            .fold({ TextOutcome.SENT }, { if (it is SecurityException) TextOutcome.DENIED else TextOutcome.FAILED })
    }

    /** Starts watching the log once the permission is there, since the provider refuses observers before it. */
    fun watch() {
        if (watching || !granted(Manifest.permission.READ_CALL_LOG)) return
        val observer =
            object : ContentObserver(main) {
                override fun onChange(selfChange: Boolean) {
                    main.removeCallbacks(notify)
                    main.postDelayed(notify, SETTLE_MS)
                }
            }
        context.contentResolver.registerContentObserver(Calls.CONTENT_URI, true, observer)
        watching = true
        changed()
    }

    private fun kind(type: Int) =
        when (type) {
            Calls.OUTGOING_TYPE -> CallKind.OUTGOING
            Calls.MISSED_TYPE, Calls.REJECTED_TYPE, Calls.BLOCKED_TYPE -> CallKind.MISSED
            else -> CallKind.INCOMING
        }

    private fun granted(permission: String) = context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    private companion object {
        const val SETTLE_MS = 1_000L
    }
}
