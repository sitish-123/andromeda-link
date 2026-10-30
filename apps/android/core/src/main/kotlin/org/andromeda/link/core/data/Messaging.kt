package org.andromeda.link.core.data

import org.andromeda.link.ffi.PhoneMessages

/** Contacts and texts, for desktops whose messages switch is on, and the call log for those whose calls switch is on. */
class Messaging(
    private val contacts: PhoneContacts,
    private val texts: PhoneTexts,
    private val calls: PhoneCallLog,
) : PhoneMessages {
    override fun contacts(
        offset: UInt,
        limit: UInt,
    ) = contacts.contacts(offset, limit)

    override fun texts(
        offset: UInt,
        limit: UInt,
    ) = texts.texts(offset, limit)

    override fun sendText(
        to: String,
        body: String,
    ) = texts.send(to, body)

    override fun calls(
        offset: UInt,
        limit: UInt,
    ) = calls.calls(offset, limit)

    override fun dial(number: String) = calls.dial(number)
}
