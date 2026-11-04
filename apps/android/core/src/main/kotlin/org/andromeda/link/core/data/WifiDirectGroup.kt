package org.andromeda.link.core.data

import android.annotation.SuppressLint
import android.content.Context
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pGroup
import android.net.wifi.p2p.WifiP2pManager
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import org.andromeda.link.ffi.HotspotCredentials
import org.andromeda.link.ffi.PhoneWifiDirect
import java.security.SecureRandom
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * A Wi-Fi Direct group this phone owns, which a desktop joins with the credentials Link sends it over Bluetooth, so
 * the phone keeps its own Wi-Fi and the desktop may keep its own. No peer negotiates: an incoming negotiation or
 * invitation needs a tap on Android ("Invitation to connect"), and a client joining with the passphrase does not.
 * Needs Nearby devices (Android 13+) or location, like the hotspot.
 */
@SuppressLint("MissingPermission")
class WifiDirectGroup(
    private val context: Context,
) : PhoneWifiDirect {
    private val manager: WifiP2pManager? = context.getSystemService(WifiP2pManager::class.java)
    private var channel: WifiP2pManager.Channel? = null
    private val random = SecureRandom()

    override fun start(): HotspotCredentials? {
        val (manager, channel) = ready() ?: return null
        val config =
            WifiP2pConfig
                .Builder()
                // Android takes only names that start with "DIRECT-" and two characters.
                .setNetworkName("DIRECT-${text(2)}-Link")
                .setPassphrase(text(PASSPHRASE_LENGTH))
                .setGroupOperatingBand(WifiP2pConfig.GROUP_OWNER_BAND_AUTO)
                .build()
        // A group left from an earlier attempt would make Android refuse a new one.
        call(manager, channel, "removing the group", manager::removeGroup)
        if (!created(manager, channel, config)) return null
        val group = group(manager, channel) ?: return null.also { Log.i(TAG, "the group did not come up") }
        return HotspotCredentials(group.networkName, group.passphrase ?: return null)
    }

    override fun stop() {
        val (manager, channel) = ready() ?: return
        call(manager, channel, "removing the group", manager::removeGroup)
    }

    /** Retries while Android answers BUSY, as it does while it turns Wi-Fi Direct on for a freshly started app. */
    private fun created(
        manager: WifiP2pManager,
        channel: WifiP2pManager.Channel,
        config: WifiP2pConfig,
    ): Boolean {
        repeat(ATTEMPTS) {
            val reason = call(manager, channel, "creating the group") { c, done -> manager.createGroup(c, config, done) }
            if (reason == null) return true
            if (reason != WifiP2pManager.BUSY) return false
            SystemClock.sleep(RETRY_MS)
        }
        return false
    }

    /** The group once Android reports it up with this phone as its owner. */
    private fun group(
        manager: WifiP2pManager,
        channel: WifiP2pManager.Channel,
    ): WifiP2pGroup? {
        val deadline = SystemClock.elapsedRealtime() + FORM_TIMEOUT_MS
        while (SystemClock.elapsedRealtime() < deadline) {
            val answered = CountDownLatch(1)
            var group: WifiP2pGroup? = null
            try {
                manager.requestGroupInfo(channel) {
                    group = it
                    answered.countDown()
                }
            } catch (error: SecurityException) {
                return null.also { Log.w(TAG, "Wi-Fi Direct: ${error.message}") }
            }
            answered.await(ANSWER_TIMEOUT_S, TimeUnit.SECONDS)
            group?.takeIf { it.isGroupOwner && it.networkName != null }?.let { return it }
            SystemClock.sleep(POLL_MS)
        }
        return null
    }

    /** Runs a [WifiP2pManager] action and waits for it: null on success, else Android's failure reason. */
    private fun call(
        manager: WifiP2pManager,
        channel: WifiP2pManager.Channel,
        what: String,
        action: (WifiP2pManager.Channel, WifiP2pManager.ActionListener) -> Unit,
    ): Int? {
        val answered = CountDownLatch(1)
        var failure: Int? = ERROR_TIMEOUT
        val listener =
            object : WifiP2pManager.ActionListener {
                override fun onSuccess() {
                    failure = null
                    answered.countDown()
                }

                override fun onFailure(reason: Int) {
                    Log.i(TAG, "$what failed: $reason")
                    failure = reason
                    answered.countDown()
                }
            }
        try {
            action(channel, listener)
        } catch (error: SecurityException) {
            Log.w(TAG, "Wi-Fi Direct: ${error.message}")
            return WifiP2pManager.ERROR
        }
        answered.await(ANSWER_TIMEOUT_S, TimeUnit.SECONDS)
        return failure
    }

    private fun ready(): Pair<WifiP2pManager, WifiP2pManager.Channel>? {
        val manager = manager ?: return null
        val channel =
            synchronized(this) {
                channel ?: manager.initialize(context, Looper.getMainLooper(), null).also { channel = it }
            } ?: return null
        return manager to channel
    }

    private fun text(length: Int) = String(CharArray(length) { ALPHABET[random.nextInt(ALPHABET.length)] })

    private companion object {
        const val TAG = "WifiDirectGroup"
        const val ALPHABET = "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789"
        const val PASSPHRASE_LENGTH = 16
        const val ANSWER_TIMEOUT_S = 5L
        const val FORM_TIMEOUT_MS = 10_000L
        const val POLL_MS = 300L
        const val ATTEMPTS = 5
        const val RETRY_MS = 1_000L
        const val ERROR_TIMEOUT = -1
    }
}
