# JNA finds its native entry points and structures by reflection, and the UniFFI bindings are called through it.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class org.andromeda.link.ffi.** { *; }
-dontwarn java.awt.**
# LogAccess calls Shizuku.newProcess by reflection.
-keepclassmembers class rikka.shizuku.Shizuku { *** newProcess(...); }
# The desktop starts LogHelper by name through app_process.
-keep class org.andromeda.link.clipboard.LogHelper { public static void main(java.lang.String[]); }
