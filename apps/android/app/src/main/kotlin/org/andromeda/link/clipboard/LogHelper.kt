package org.andromeda.link.clipboard

import android.net.LocalServerSocket
import org.andromeda.link.screen.InputRelay
import java.io.IOException
import kotlin.concurrent.thread
import kotlin.system.exitProcess

/**
 * The log helper the desktop starts over adb, the way Shizuku starts its server: `app_process` runs this class from
 * Link's APK as the shell uid, which reads other apps' log lines without Android's dialog. It lives until the phone
 * restarts. SELinux keeps apps from connecting to a shell's socket, so the helper writes to Link instead: a
 * `content write` into [LogLineProvider], which only the shell or root may open.
 *
 * Its limits: it writes only to Link's own provider; it sends only Link's clipboard-denial lines ([follow]); its one
 * input is mirror input from Link's own provider, which [org.andromeda.link.screen.InputRelay] turns into touch, key
 * and panel-power events and nothing else, so it is no shell; it exits when Link is uninstalled or has not run for
 * [IDLE_MS] ([main]).
 *
 * Started as `CLASSPATH=<Link's APK> app_process /system/bin org.andromeda.link.clipboard.LogHelper <package>`.
 */
object LogHelper {
    private const val LOCK = "org.andromeda.link.loghelper"
    private const val IDLE_MS = 30 * 60 * 1000L
    private const val BEAT_MS = 60 * 1000L
    private const val TICK_MS = 3000L

    /** Bound while this helper lives, so a second start is a no-op. */
    private var lock: LocalServerSocket? = null

    /** `content write` into Link's provider; its stdin reaches Link. */
    @Volatile private var link: Process? = null

    @JvmStatic
    fun main(args: Array<String>) {
        val pkg = args.firstOrNull() ?: exit("usage: LogHelper <package>")
        val appId = appId(pkg) ?: exit("$pkg is not installed")
        lock = runCatching { LocalServerSocket(LOCK) }.getOrElse { exit("already running") }
        println("helper for $pkg (uid $appId)")
        thread(isDaemon = true, name = "logcat") { follow(pkg) }
        thread(isDaemon = true, name = "input") {
            while (true) {
                if (running(pkg)) InputRelay.follow(pkg)
                Thread.sleep(TICK_MS)
            }
        }
        var lastRun = System.currentTimeMillis()
        var lastBeat = 0L
        var lastCheck = lastRun
        while (true) {
            Thread.sleep(TICK_MS)
            val now = System.currentTimeMillis()
            if (now - lastCheck > BEAT_MS) {
                lastCheck = now
                if (appId(pkg) != appId) exit("$pkg was uninstalled")
            }
            when {
                // A heartbeat lets a closed reader (Link died or stopped watching) end `content write`.
                link?.isAlive == true -> {
                    lastRun = now
                    if (now - lastBeat > BEAT_MS) {
                        lastBeat = now
                        send("")
                    }
                }
                // Connects only to a running Link: opening its provider would start Link after a force stop.
                running(pkg) -> {
                    lastRun = now
                    link = connect(pkg)
                }
                now - lastRun > IDLE_MS -> exit("$pkg has not run for ${IDLE_MS / 60000} min")
            }
        }
    }

    /** Sends Link's own clipboard-denial lines and nothing else; logcat ending ends the helper. */
    private fun follow(pkg: String) {
        val denial = "Denying clipboard access to $pkg"
        val logcat = ProcessBuilder("logcat", "-T", "1", "-v", "brief", "ClipboardService:E", "*:S").start()
        logcat.inputStream.bufferedReader().useLines { lines ->
            for (line in lines) if (denial in line) send(line)
        }
        exit("logcat ended")
    }

    @Synchronized
    private fun send(line: String) {
        val process = link ?: return
        try {
            process.outputStream.write((line + "\n").toByteArray())
            process.outputStream.flush()
        } catch (_: IOException) {
            process.destroy()
            link = null
        }
    }

    private fun connect(pkg: String): Process? =
        runCatching {
            ProcessBuilder("content", "write", "--uri", "content://$pkg.loghelper/lines")
                .redirectOutput(ProcessBuilder.Redirect.INHERIT)
                .redirectError(ProcessBuilder.Redirect.INHERIT)
                .start()
        }.getOrNull()

    private fun running(pkg: String): Boolean = runCatching { ProcessBuilder("pidof", pkg).start().waitFor() == 0 }.getOrDefault(false)

    private fun appId(pkg: String): Int? =
        runCatching {
            val process = ProcessBuilder("cmd", "package", "list", "packages", "-U", pkg).start()
            val prefix = "package:$pkg uid:"
            process.inputStream
                .bufferedReader()
                .readLines()
                .firstOrNull { it.startsWith(prefix) }
                ?.removePrefix(prefix)
                ?.takeWhile(Char::isDigit)
                ?.toInt()
                ?.rem(PER_USER)
        }.getOrNull()

    private fun exit(why: String): Nothing {
        println(why)
        link?.destroy()
        exitProcess(0)
    }

    private const val PER_USER = 100000
}
