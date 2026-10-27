# JNA and the UniFFI bindings are reached reflectively from native code.
-keep class com.sun.jna.** { *; }
-keep class org.andromeda.link.ffi.** { *; }
