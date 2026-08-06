# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile

# android.rs's find_app_class()/JNI calls reach androidx.core.content.
# FileProvider by raw class/method name strings, not through any
# Kotlin/Java call site R8 can see -- with no keep rule, release
# minification (isMinifyEnabled=true, this app's only difference from the
# debug build every earlier test used) strips/renames its members as
# "unused", and the JNI call then fails with a real
# NoSuchMethodError -- confirmed live, this exact crash is why the "Share"
# button worked in every debug-build test all session but broke the
# moment it shipped in a real release APK.
-keep class androidx.core.content.FileProvider { *; }