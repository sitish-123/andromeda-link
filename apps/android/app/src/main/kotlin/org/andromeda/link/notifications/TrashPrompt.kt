package org.andromeda.link.notifications

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import androidx.core.app.NotificationCompat
import org.andromeda.link.R

/** A desktop asked to move a photo to the trash and Android wants the user to confirm: tapping opens its prompt. */
fun showTrashPrompt(
    context: Context,
    prompt: PendingIntent,
) {
    val notification =
        NotificationCompat
            .Builder(context, Channels.SHARES)
            .setSmallIcon(R.drawable.ic_stat_link)
            .setContentTitle(context.getString(R.string.trash_prompt_title))
            .setContentText(context.getString(R.string.trash_prompt_text))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setContentIntent(prompt)
            .setAutoCancel(true)
            .setTimeoutAfter(TIMEOUT_MS)
            .build()
    context.getSystemService(NotificationManager::class.java).notify(NOTIFICATION, notification)
}

private const val NOTIFICATION = 45

/** The phone stops waiting for the answer after 55 s (MediaLibrary). */
private const val TIMEOUT_MS = 55_000L
