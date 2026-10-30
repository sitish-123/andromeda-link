import QtQuick
import org.andromeda.Link

QtObject {
    property int unreadCount: 2
    property string currentThread: "1"
    property string threadName: "Maya"
    property string threadAddress: "+49 170 5550123"

    property ListModel threads: ListModel {
        ListElement { thread: "1"; name: "Maya"; address: "+49 170 5550123"; initial: "M"; snippet: "Are you still coming tonight? I can save you a seat near the front"; timeText: "2 min"; unread: true; section: "Unread" }
        ListElement { thread: "2"; name: "Jonas"; address: "+49 160 5550188"; initial: "J"; snippet: "Do you have the keys for the van?"; timeText: "6 min"; unread: true; section: "Unread" }
        ListElement { thread: "3"; name: "Mum"; address: "+49 89 5550111"; initial: "M"; snippet: "Call me when you land"; timeText: "1 h"; unread: false; section: "Earlier" }
        ListElement { thread: "4"; name: "DHL"; address: "DHL"; initial: "D"; snippet: "Your parcel arrives today between 14:00 and 16:00"; timeText: "3 h"; unread: false; section: "Earlier" }
        ListElement { thread: "5"; name: "Lena Fischer"; address: "+49 171 5550190"; initial: "L"; snippet: "Thanks, see you Thursday"; timeText: "Yesterday"; unread: false; section: "Earlier" }
        ListElement { thread: "6"; name: "+49 151 5550142"; address: "+49 151 5550142"; initial: "#"; snippet: "Your code is 482913"; timeText: "Yesterday"; unread: false; section: "Earlier" }
        ListElement { thread: "7"; name: "Tom Becker"; address: "+49 172 5550133"; initial: "T"; snippet: "Ok"; timeText: "Mon"; unread: false; section: "Earlier" }
    }
    property ListModel conversation: ListModel {
        ListElement { body: "Dinner on Thursday still on?"; sent: false; timeText: "19:12"; dayText: "Yesterday" }
        ListElement { body: "Yes, count me in."; sent: true; timeText: "19:20"; dayText: "Yesterday" }
        ListElement { body: "Are you still coming tonight?"; sent: false; timeText: "17:38"; dayText: "Today" }
        ListElement { body: "I can save you a seat near the front if you text me before eight."; sent: false; timeText: "17:38"; dayText: "Today" }
        ListElement { body: "Yes! Leaving at seven."; sent: true; timeText: "17:41"; dayText: "Today" }
        ListElement { body: "Do you need anything from the shop?"; sent: true; timeText: "17:41"; dayText: "Today" }
        ListElement { body: "Bread, if you pass the bakery. Thank you!"; sent: false; timeText: "17:43"; dayText: "Today" }
    }

    function send(body) { conversation.append({ body: body, sent: true, timeText: "17:45", dayText: "Today" }) }
    function sendTo(address, body) { console.log("sendTo", address, body) }
}
