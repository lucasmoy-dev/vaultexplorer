# The JNI entry points are found by name from native code, so R8 must not
# rename or remove them.
-keepclasseswithmembernames class dev.lucasmoy.livesubs.NativeEngine {
    native <methods>;
}
-keep class dev.lucasmoy.livesubs.NativeEngine { *; }
