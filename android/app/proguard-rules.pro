# App-specific R8 rules.

# The engine is Rust, reached through JNA. JNA finds classes, fields and methods by name
# from native code, and the generated bindings describe the native library with
# interfaces and structures that R8 cannot see being used.
-keep class com.sun.jna.** { *; }
-keep class * extends com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class com.decentralizeddeaddrop.engine.** { *; }
-dontwarn java.awt.**

# CameraX and ZXing ship their own rules.

# SQLCipher (message history) is called from its own native code.
-keep class net.zetetic.** { *; }
