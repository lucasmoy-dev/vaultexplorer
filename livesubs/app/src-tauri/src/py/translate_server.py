"""Long-lived Argos Translate worker.

Spoken by JSON lines over stdin/stdout, one request per line:

    {"id": 7, "text": "hello", "from": "en", "to": "es"}
    -> {"id": 7, "text": "hola"}

A worker process rather than a `python -c` per caption, because importing
argostranslate and loading a language model takes a second or two -- paid
once here instead of on every line of subtitles. Argos pivots through
English on its own when there's no direct package (es->fr becomes es->en->fr),
which is why only the four English pairs are installed.
"""

import json
import sys


def main() -> None:
    try:
        from argostranslate import translate as argos
    except Exception as exc:  # pragma: no cover - reported to the Rust side
        sys.stdout.write(json.dumps({"id": 0, "error": f"argostranslate no disponible: {exc}"}) + "\n")
        sys.stdout.flush()
        return

    # Announce readiness: the Rust side waits for this before sending, so a
    # broken install surfaces as an error at startup rather than as a
    # subtitle that silently never arrives.
    installed = sorted({lang.code for lang in argos.get_installed_languages()})
    sys.stdout.write(json.dumps({"id": 0, "ready": True, "languages": installed}) + "\n")
    sys.stdout.flush()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        request_id = 0
        try:
            request = json.loads(line)
            request_id = request.get("id", 0)
            text = request["text"]
            source = request["from"]
            target = request["to"]
            if source == target:
                translated = text
            else:
                translated = argos.translate(text, source, target)
            response = {"id": request_id, "text": translated}
        except Exception as exc:
            response = {"id": request_id, "error": str(exc)}
        sys.stdout.write(json.dumps(response, ensure_ascii=False) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
