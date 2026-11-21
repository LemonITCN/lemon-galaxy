# uniffi 生成的绑定通过 JNA 反射调用本地库：保留 JNA 与生成代码
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
-dontwarn java.awt.**
-dontwarn com.sun.jna.**
-keep class uniffi.galaxy_ffi.** { *; }

# CoreProvider 通过反射加载真内核适配层
-keep class cn.lemonit.galaxy.core.rust.** { *; }

# kotlinx.serialization：终端配色 JSON
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.**
-keepclassmembers class kotlinx.serialization.json.** { *** Companion; }
-keepclasseswithmembers class kotlinx.serialization.json.** { kotlinx.serialization.KSerializer serializer(...); }
-keep,includedescriptorclasses class cn.lemonit.galaxy.**$$serializer { *; }
-keepclassmembers class cn.lemonit.galaxy.** { *** Companion; }
-keepclasseswithmembers class cn.lemonit.galaxy.** { kotlinx.serialization.KSerializer serializer(...); }
