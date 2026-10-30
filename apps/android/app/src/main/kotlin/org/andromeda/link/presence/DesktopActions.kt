package org.andromeda.link.presence

import android.util.Log
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.Desktop
import org.andromeda.link.devices.CAN_CONTROL

/** One-tap commands from outside the app (tiles, the presence notification, the widget) to every connected desktop. */
object DesktopActions {
    private const val TAG = "LinkDesktopActions"

    fun connected(repository: LinkRepository): List<Desktop> = repository.desktops.value.filter { it.connected }

    /** Connected desktops that granted this phone Control system. */
    fun controllable(repository: LinkRepository): List<Desktop> =
        connected(repository).filter { repository.capabilities.value[it.id]?.contains(CAN_CONTROL) == true }

    suspend fun lock(repository: LinkRepository) =
        controllable(repository).forEach { desktop ->
            repository.lockDesktop(desktop.id).onFailure { Log.w(TAG, "lock ${desktop.name}: ${it.message}") }
        }

    suspend fun ring(repository: LinkRepository) =
        connected(repository).forEach { desktop ->
            repository.ringDesktop(desktop.id, true).onFailure { Log.w(TAG, "ring ${desktop.name}: ${it.message}") }
        }
}
