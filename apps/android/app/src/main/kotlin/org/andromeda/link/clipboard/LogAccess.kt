package org.andromeda.link.clipboard

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.ParcelFileDescriptor
import android.provider.Settings
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import rikka.shizuku.Shizuku
import java.io.ByteArrayInputStream
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread

/**
 * The ways this app reads other apps' log lines. Shizuku (started over wireless debugging, or by root), `su` and the
 * [LogHelper] the desktop starts over adb run logcat outside this app, so Android asks nothing; READ_LOGS alone makes
 * Android 13+ ask at every logcat start.
 */
object LogAccess {
    enum class Method { Automatic, Shizuku, Root, Helper, Android }

    /** Whether `su` granted this app a shell; null until [logcat] first tries it. */
    @Volatile private var root: Boolean? = null

    private val _method = MutableStateFlow(Method.Automatic)

    /** The user's pick in Settings > Advanced; Automatic tries Shizuku, then root, then the helper, then READ_LOGS. */
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
            Method.Automatic -> shizuku() || rootMayWork() || helper() || readLogs(context)
            Method.Shizuku -> shizuku()
            Method.Root -> rootMayWork()
            Method.Helper -> helper()
            Method.Android -> readLogs(context)
        }

    /** Whether logcat starts without Android's log access dialog, so also from the background. */
    fun quiet(): Boolean =
        when (method.value) {
            Method.Automatic -> shizuku() || root == true || helper()
            Method.Shizuku -> shizuku()
            Method.Root -> root == true
            Method.Helper -> helper()
            Method.Android -> false
        }

    /** The method logcat uses now; null when the picked one is not ready. */
    fun inUse(context: Context): Method? =
        when (method.value) {
            Method.Automatic ->
                when {
                    shizuku() -> Method.Shizuku
                    rootMayWork() -> Method.Root
                    helper() -> Method.Helper
                    readLogs(context) -> Method.Android
                    else -> null
                }
            else -> method.value.takeIf { allowed(context) }
        }

    fun shizukuReady(): Boolean = shizuku()

    fun rootGranted(): Boolean = root == true

    fun suFound(): Boolean = hasSu()

    fun helperRunning(): Boolean = helper()

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
        if (pick.allows(Method.Root) && hasSu() && rootShell()) {
            val logcat = (listOf("logcat") + args).joinToString(" ") { "'$it'" }
            return RootProcess(ProcessBuilder("su", "-c", "echo \$\$; exec $logcat").start())
        }
        // The helper runs its own logcat and sends only this app's denial lines, whatever args asks for.
        if (pick.allows(Method.Helper) && helper()) {
            val (input, output) = ParcelFileDescriptor.createPipe()
            val writer = ParcelFileDescriptor.AutoCloseOutputStream(output)
            helperReaders += writer
            return HelperProcess(input, writer)
        }
        return ProcessBuilder("logcat", *args).start()
    }

    /** Whether the overlay [ClipboardOverlay] needs is allowed, or root may still allow it in [overlay]. */
    fun overlayAllowed(context: Context): Boolean = Settings.canDrawOverlays(context) || (hasSu() && root != false)

    /** With root the overlay is given here, so the user sets nothing in Settings; blocks like [logcat]. */
    fun overlay(context: Context): Boolean =
        Settings.canDrawOverlays(context) ||
            (hasSu() && rootShell() && su("appops set ${context.packageName} SYSTEM_ALERT_WINDOW allow"))

    private fun Method.allows(other: Method) = this == Method.Automatic || this == other

    private fun rootMayWork(): Boolean = hasSu() && root != false

    /** Only this probe sets [root]: a later command can fail with root granted. */
    private fun rootShell(): Boolean {
        if (root == null) root = su("true")
        return root == true
    }

    private fun su(command: String): Boolean =
        runCatching { ProcessBuilder("su", "-c", command).start().waitFor() == 0 }.getOrDefault(false)

    /**
     * A root logcat. Destroying the su client leaves logcat running as root, out of this app's reach, so destroy kills
     * it through su by the PID its shell printed first.
     */
    private class RootProcess(
        private val su: Process,
    ) : Process() {
        private val input = su.inputStream
        private val pid: Int? =
            runCatching {
                generateSequence { input.read().takeIf { it >= 0 && it != '\n'.code } }
                    .map { it.toChar() }
                    .joinToString("")
                    .toInt()
            }.getOrNull()

        override fun getInputStream(): InputStream = input

        override fun getOutputStream(): OutputStream = su.outputStream

        override fun getErrorStream(): InputStream = su.errorStream

        override fun waitFor(): Int = su.waitFor()

        override fun exitValue(): Int = su.exitValue()

        override fun isAlive(): Boolean = su.isAlive

        override fun destroy() {
            if (su.isAlive) pid?.let { runCatching { ProcessBuilder("su", "-c", "kill $it").start() } }
            su.destroy()
        }
    }

    /** The open stream from [LogHelper], drained here so it never blocks; null while no helper writes. */
    @Volatile private var helperStream: ParcelFileDescriptor? = null

    /** Write ends of the pipes [logcat] gave out for the helper; each gets the helper's lines from now on. */
    private val helperReaders = CopyOnWriteArrayList<ParcelFileDescriptor.AutoCloseOutputStream>()

    /** Runs on a binder thread when a helper connects; the watcher moves to it. */
    @Volatile var onHelper: () -> Unit = {}

    private fun helper(): Boolean = helperStream != null

    /** From [LogLineProvider]: the helper's newest stream; an older one ends when its writer goes. */
    internal fun helperConnected(stream: ParcelFileDescriptor) {
        helperStream = stream
        thread(isDaemon = true, name = "log-helper") {
            runCatching {
                ParcelFileDescriptor.AutoCloseInputStream(stream).bufferedReader().forEachLine { line ->
                    if (line.isEmpty()) return@forEachLine
                    for (reader in helperReaders) {
                        runCatching {
                            reader.write((line + "\n").toByteArray())
                            reader.flush()
                        }.onFailure { helperReaders -= reader }
                    }
                }
            }
            // The helper went away: readers see the end of their stream, as when logcat ends.
            if (helperStream === stream) {
                helperStream = null
                for (reader in helperReaders) runCatching { reader.close() }
                helperReaders.clear()
            }
        }
        onHelper()
    }

    /** The helper's lines as a logcat [Process]; destroying it ends its pipe. */
    private class HelperProcess(
        private val input: ParcelFileDescriptor,
        private val output: ParcelFileDescriptor.AutoCloseOutputStream,
    ) : Process() {
        @Volatile private var closed = false

        override fun getInputStream(): InputStream = ParcelFileDescriptor.AutoCloseInputStream(input)

        override fun getOutputStream(): OutputStream =
            object : OutputStream() {
                override fun write(b: Int) = Unit
            }

        override fun getErrorStream(): InputStream = ByteArrayInputStream(ByteArray(0))

        override fun waitFor(): Int {
            while (!closed) Thread.sleep(WAIT_MS)
            return 0
        }

        override fun exitValue(): Int = if (closed) 0 else throw IllegalThreadStateException()

        override fun isAlive(): Boolean = !closed

        // Closing the write end ends a read blocked on the read end.
        override fun destroy() {
            closed = true
            helperReaders -= output
            runCatching { output.close() }
            runCatching { input.close() }
        }
    }

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

    private const val WAIT_MS = 200L
    private const val PREFS = "log_access"
    private const val KEY_METHOD = "method"
}
