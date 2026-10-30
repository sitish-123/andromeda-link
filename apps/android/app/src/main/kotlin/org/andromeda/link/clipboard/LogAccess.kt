package org.andromeda.link.clipboard

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.provider.Settings
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import rikka.shizuku.Shizuku
import java.io.File

/**
 * The three ways this app reads other apps' log lines. Shizuku (started over wireless debugging, or by root) and `su`
 * run logcat outside this app, so Android asks nothing; READ_LOGS alone makes Android 13+ ask at every logcat start.
 */
object LogAccess {
    enum class Method { Automatic, Shizuku, Root, Android }

    /** Whether `su` granted this app a shell; null until [logcat] first tries it. */
    @Volatile private var root: Boolean? = null

    private val _method = MutableStateFlow(Method.Automatic)

    /** The user's pick in Settings > Advanced; Automatic tries Shizuku, then root, then READ_LOGS. */
    val method: StateFlow<Method> = _method.asStateFlow()

    fun load(context: Context) {
        val name = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY_METHOD, null)
        _method.value = Method.entries.firstOrNull { it.name == name } ?: Method.Automatic
    }

    fun choose(
        context: Context,
        method: Method,
    ) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit { putString(KEY_METHOD, method.name) }
        // A pick of Root asks the root manager again after an earlier denial.
        if (method == Method.Root && root == false) root = null
        _method.value = method
    }

    fun allowed(context: Context): Boolean =
        when (method.value) {
            Method.Automatic -> shizuku() || rootMayWork() || readLogs(context)
            Method.Shizuku -> shizuku()
            Method.Root -> rootMayWork()
            Method.Android -> readLogs(context)
        }

    /** Whether logcat starts without Android's log access dialog, so also from the background. */
    fun quiet(): Boolean =
        when (method.value) {
            Method.Automatic -> shizuku() || root == true
            Method.Shizuku -> shizuku()
            Method.Root -> root == true
            Method.Android -> false
        }

    /** The method logcat uses now; null when the picked one is not ready. */
    fun inUse(context: Context): Method? =
        when (method.value) {
            Method.Automatic ->
                when {
                    shizuku() -> Method.Shizuku
                    rootMayWork() -> Method.Root
                    readLogs(context) -> Method.Android
                    else -> null
                }
            else -> method.value.takeIf { allowed(context) }
        }

    fun shizukuReady(): Boolean = shizuku()

    fun rootGranted(): Boolean = root == true

    fun suFound(): Boolean = hasSu()

    fun readLogs(context: Context): Boolean =
        context.checkSelfPermission(Manifest.permission.READ_LOGS) == PackageManager.PERMISSION_GRANTED

    /** Shizuku runs but has not given this app access yet. */
    fun shizukuAsks(): Boolean = Shizuku.pingBinder() && !shizuku()

    fun askShizuku() = Shizuku.requestPermission(0)

    /** Blocks while the root manager asks the user the first time. */
    fun logcat(vararg args: String): Process {
        val pick = method.value
        if (pick.allows(Method.Shizuku) && shizuku()) {
            return newProcess.invoke(null, arrayOf("logcat", *args), null, null) as Process
        }
        if (pick.allows(Method.Root) && hasSu() && root == null) root = su("true")
        if (pick.allows(Method.Root) && root == true) {
            return ProcessBuilder("su", "-c", (listOf("logcat") + args).joinToString(" ") { "'$it'" }).start()
        }
        return ProcessBuilder("logcat", *args).start()
    }

    /** Whether the overlay [ClipboardOverlay] needs is allowed, or root may still allow it in [overlay]. */
    fun overlayAllowed(context: Context): Boolean = Settings.canDrawOverlays(context) || (hasSu() && root != false)

    /** With root the overlay is given here, so the user sets nothing in Settings; blocks like [logcat]. */
    fun overlay(context: Context): Boolean =
        Settings.canDrawOverlays(context) ||
            (hasSu() && root != false && su("appops set ${context.packageName} SYSTEM_ALERT_WINDOW allow"))

    private fun Method.allows(other: Method) = this == Method.Automatic || this == other

    private fun rootMayWork(): Boolean = hasSu() && root != false

    private fun su(command: String): Boolean =
        runCatching { ProcessBuilder("su", "-c", command).start().waitFor() == 0 }.getOrDefault(false).also { root = it }

    private fun shizuku(): Boolean = Shizuku.pingBinder() && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED

    // An app's PATH often leaves out where Magisk and KernelSU put su.
    private fun hasSu(): Boolean =
        (
            System.getenv("PATH").orEmpty().split(':') +
                listOf("/system/bin", "/system/xbin", "/sbin", "/debug_ramdisk")
        ).any { File(it, "su").canExecute() }

    // Shizuku 13 keeps newProcess private in favour of user services; a plain process is all logcat needs.
    private val newProcess by lazy {
        Shizuku::class.java
            .getDeclaredMethod("newProcess", Array<String>::class.java, Array<String>::class.java, String::class.java)
            .apply { isAccessible = true }
    }

    private const val PREFS = "log_access"
    private const val KEY_METHOD = "method"
}
