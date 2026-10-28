package org.andromeda.link.clipboard

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import rikka.shizuku.Shizuku
import java.io.File

/**
 * The three ways this app reads other apps' log lines. Shizuku (started over wireless debugging, or by root) and `su`
 * run logcat outside this app, so Android asks nothing; READ_LOGS alone makes Android 13+ ask at every logcat start.
 */
object LogAccess {
    /** Whether `su` granted this app a shell; null until [logcat] first tries it. */
    @Volatile private var root: Boolean? = null

    fun allowed(context: Context): Boolean = shizuku() || (hasSu() && root != false) ||
        context.checkSelfPermission(Manifest.permission.READ_LOGS) == PackageManager.PERMISSION_GRANTED

    /** Whether logcat starts without Android's log access dialog, so also from the background. */
    fun quiet(): Boolean = shizuku() || root == true

    /** Shizuku runs but has not given this app access yet. */
    fun shizukuAsks(): Boolean = Shizuku.pingBinder() && !shizuku()

    fun askShizuku() = Shizuku.requestPermission(0)

    /** Blocks while the root manager asks the user the first time. */
    fun logcat(vararg args: String): Process {
        if (shizuku()) return newProcess.invoke(null, arrayOf("logcat", *args), null, null) as Process
        if (hasSu() && root == null) root = runCatching { ProcessBuilder("su", "-c", "true").start().waitFor() == 0 }.getOrDefault(false)
        if (root == true) return ProcessBuilder("su", "-c", (listOf("logcat") + args).joinToString(" ") { "'$it'" }).start()
        return ProcessBuilder("logcat", *args).start()
    }

    private fun shizuku(): Boolean =
        Shizuku.pingBinder() && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED

    // An app's PATH often leaves out where Magisk and KernelSU put su.
    private fun hasSu(): Boolean = (System.getenv("PATH").orEmpty().split(':') + listOf("/system/bin", "/system/xbin", "/sbin", "/debug_ramdisk"))
        .any { File(it, "su").canExecute() }

    // Shizuku 13 keeps newProcess private in favour of user services; a plain process is all logcat needs.
    private val newProcess by lazy {
        Shizuku::class.java.getDeclaredMethod("newProcess", Array<String>::class.java, Array<String>::class.java, String::class.java)
            .apply { isAccessible = true }
    }
}
