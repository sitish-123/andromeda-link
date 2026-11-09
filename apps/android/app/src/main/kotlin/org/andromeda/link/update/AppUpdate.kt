package org.andromeda.link.update

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/** Updates the app from the newest GitHub release: the APK streams into a PackageInstaller session, Android confirms. */
object AppUpdate {
    sealed interface State {
        data object Idle : State

        data object Checking : State

        data object Current : State

        data class Available(
            val version: String,
            val url: String,
            val bytes: Long,
        ) : State

        /** `progress` is 0..1, or null while the size is unknown. */
        data class Downloading(
            val version: String,
            val progress: Float?,
        ) : State

        data object Installing : State

        data class Failed(
            val message: String,
        ) : State
    }

    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state

    suspend fun check(context: Context) {
        _state.value = State.Checking
        _state.value =
            withContext(Dispatchers.IO) {
                runCatching {
                    val release = JSONObject(get(LATEST).inputStream.bufferedReader().use { it.readText() })
                    val version = release.getString("tag_name").removePrefix("v")
                    val assets = release.getJSONArray("assets")
                    val apk =
                        (0 until assets.length())
                            .map { assets.getJSONObject(it) }
                            .firstOrNull { it.getString("name").endsWith(".apk") }
                    val installed =
                        context.packageManager
                            .getPackageInfo(context.packageName, 0)
                            .versionName
                            .orEmpty()
                    if (apk == null || compareVersions(version, installed) <= 0) {
                        State.Current
                    } else {
                        State.Available(version, apk.getString("browser_download_url"), apk.optLong("size"))
                    }
                }.getOrElse { State.Failed(it.message ?: it.javaClass.simpleName) }
            }
    }

    suspend fun install(
        context: Context,
        update: State.Available,
    ) {
        _state.value = State.Downloading(update.version, null)
        val failure =
            withContext(Dispatchers.IO) {
                runCatching {
                    val installer = context.packageManager.packageInstaller
                    val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
                    params.setAppPackageName(context.packageName)
                    val id = installer.createSession(params)
                    installer.openSession(id).use { session ->
                        try {
                            val connection = get(update.url)
                            val total = connection.contentLengthLong.takeIf { it > 0 } ?: update.bytes
                            connection.inputStream.use { input ->
                                session.openWrite("link.apk", 0, total.takeIf { it > 0 } ?: -1).use { output ->
                                    val buffer = ByteArray(64 * 1024)
                                    var written = 0L
                                    while (true) {
                                        val read = input.read(buffer)
                                        if (read < 0) break
                                        output.write(buffer, 0, read)
                                        written += read
                                        if (total > 0) _state.value = State.Downloading(update.version, written.toFloat() / total)
                                    }
                                    session.fsync(output)
                                }
                            }
                            _state.value = State.Installing
                            val result =
                                PendingIntent.getBroadcast(
                                    context,
                                    id,
                                    Intent(context, UpdateReceiver::class.java),
                                    // Mutable: the installer adds the status extras.
                                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
                                )
                            session.commit(result.intentSender)
                        } catch (e: Exception) {
                            session.abandon()
                            throw e
                        }
                    }
                }.exceptionOrNull()
            }
        if (failure != null) _state.value = State.Failed(failure.message ?: failure.javaClass.simpleName)
    }

    internal fun finished(
        context: Context,
        intent: Intent,
    ) {
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION")
                val confirm = intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT) ?: return
                context.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            PackageInstaller.STATUS_SUCCESS -> _state.value = State.Current
            PackageInstaller.STATUS_FAILURE_ABORTED -> _state.value = State.Idle
            else ->
                _state.value =
                    State.Failed(intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: "Install failed")
        }
    }

    private fun get(url: String): HttpURLConnection =
        (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 15_000
            readTimeout = 30_000
            setRequestProperty("Accept", "application/vnd.github+json")
            setRequestProperty("User-Agent", "AndromedaLink")
            if (responseCode !in 200..299) throw IllegalStateException("GitHub answered $responseCode")
        }

    /** Compares dotted numeric versions ("0.2.10" > "0.2.9"); a missing part counts as 0. */
    internal fun compareVersions(
        a: String,
        b: String,
    ): Int {
        val x = a.split('.', '-').map { it.toIntOrNull() ?: 0 }
        val y = b.split('.', '-').map { it.toIntOrNull() ?: 0 }
        for (i in 0 until maxOf(x.size, y.size)) {
            val c = x.getOrElse(i) { 0 }.compareTo(y.getOrElse(i) { 0 })
            if (c != 0) return c
        }
        return 0
    }

    private const val LATEST = "https://api.github.com/repos/sitish-123/andromeda-link/releases/latest"
}

class UpdateReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) = AppUpdate.finished(context, intent)
}
