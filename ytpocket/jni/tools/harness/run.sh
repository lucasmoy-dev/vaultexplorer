#!/bin/bash
# Exercises the native surface (YouTube search, stream resolution, filename
# rules) on the *host*, over JNI, against real YouTube -- the same code the
# phone runs, minus the ABI and minus Android.
#
# Worth keeping as a script: it is the only check that the JNI symbol names
# and the JSON contract line up, and its failure mode on a phone would
# otherwise be a silent UnsatisfiedLinkError. It also doubles as the canary
# for "YouTube changed something", which is this app's normal way to break.
#
#   ./run.sh ["search terms"]
set -e
cd "$(dirname "$0")"
echo "== building the JNI library for this machine"
(cd ../.. && cargo build --release --quiet)
LIB="$(cd ../../target/release && pwd)"

echo "== compiling the harness"
rm -rf classes && mkdir -p classes
javac -d classes dev/lucasmoy/ytpocket/Native.java Harness.java

echo "== running"
java -Djava.library.path="$LIB" -cp classes Harness "${1:-rick astley never gonna give you up}"
