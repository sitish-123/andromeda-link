package org.andromeda.link.presence

import android.content.ComponentName
import android.content.Context
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.andromeda.link.LinkApplication
import org.andromeda.link.R
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.Desktop

/** A tile that runs one [DesktopActions] command; unavailable while no desktop can take it. */
abstract class DesktopTileService : TileService() {
    protected val repository: LinkRepository get() = (application as LinkApplication).container.repository

    abstract fun targets(): List<Desktop>

    abstract suspend fun run()

    abstract fun label(name: String?): String

    override fun onStartListening() {
        val targets = targets()
        qsTile?.apply {
            state = if (targets.isEmpty()) Tile.STATE_UNAVAILABLE else Tile.STATE_INACTIVE
            label = label(targets.singleOrNull()?.name)
            updateTile()
        }
    }

    override fun onClick() {
        CoroutineScope(SupervisorJob() + Dispatchers.Default).launch { run() }
    }

    companion object {
        /** SystemUI keeps a tile's last state and drops clicks on an unavailable one, so tiles refresh when targets change. */
        fun refresh(context: Context) {
            for (tile in listOf(LockTileService::class.java, RingTileService::class.java)) {
                requestListeningState(context, ComponentName(context, tile))
            }
        }
    }
}

class LockTileService : DesktopTileService() {
    override fun targets() = DesktopActions.controllable(repository)

    override suspend fun run() = DesktopActions.lock(repository)

    override fun label(name: String?) = name?.let { getString(R.string.tile_lock_named, it) } ?: getString(R.string.tile_lock)
}

class RingTileService : DesktopTileService() {
    override fun targets() = DesktopActions.connected(repository)

    override suspend fun run() = DesktopActions.ring(repository)

    override fun label(name: String?) = name?.let { getString(R.string.tile_ring_named, it) } ?: getString(R.string.ring_desktop)
}
