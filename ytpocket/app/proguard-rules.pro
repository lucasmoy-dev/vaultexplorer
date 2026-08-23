# The JNI entry points are resolved by name from native code, so R8 must not
# rename or remove them.
-keepclasseswithmembernames class dev.lucasmoy.ytpocket.Native {
    native <methods>;
}
-keep class dev.lucasmoy.ytpocket.Native { *; }
