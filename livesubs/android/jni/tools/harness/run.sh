#!/bin/bash
# Exercises the native pipeline (VAD -> whisper -> JSON) on the *host*, over
# JNI, with a real audio file -- the same code the phone runs, minus the ABI
# and minus Android.
#
# Worth having as a script rather than a one-off: it is the only way to check
# the JNI boundary (symbol names, argument marshalling, the JSON contract)
# without an Android device in the loop, and it catches the class of mistake
# that otherwise shows up as a silent `UnsatisfiedLinkError` on a phone.
#
#   ./run.sh [model-name] [wav-file]
set -e
cd "$(dirname "$0")"
MODEL_NAME="${1:-base}"
MODEL_DIR="${MODEL_DIR:-$HOME/.cache/livesubs/whisper}"
MODEL="$MODEL_DIR/ggml-$MODEL_NAME.bin"
WAV="${2:-/tmp/livesubs-jfk.wav}"

if [ ! -f "$MODEL" ]; then
  echo "Model not found: $MODEL"
  echo "Download it with the desktop app, or:"
  echo "  curl -L -o $MODEL https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$MODEL_NAME.bin"
  exit 1
fi
if [ ! -f "$WAV" ]; then
  echo "Fetching whisper.cpp's sample clip into $WAV"
  curl -sL -o "$WAV" https://raw.githubusercontent.com/ggerganov/whisper.cpp/master/samples/jfk.wav
fi

echo "== building the JNI library for this machine"
(cd ../.. && cargo build --release --quiet)
LIB="$(cd ../../target/release && pwd)"

echo "== compiling the harness"
rm -rf classes && mkdir -p classes
javac -d classes dev/lucasmoy/livesubs/NativeEngine.java Harness.java

echo "== running"
java -Djava.library.path="$LIB" -cp classes Harness "$MODEL" "$MODEL_NAME" "$WAV" "${3:-auto}" \
  2>&1 | grep -vE '^(whisper_|ggml_|main:)'
