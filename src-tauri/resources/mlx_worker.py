"""Sedjem MLX Whisper worker.

A long-lived process that keeps an MLX Whisper model warm on the Apple GPU
and answers JSON-line requests on stdin with JSON-line responses on stdout.

Protocol (one JSON object per line):
  -> {"id": 1, "op": "load", "model": "/abs/model/dir"}
  <- {"id": 1, "ok": true}
  -> {"id": 2, "op": "transcribe", "audio": "<base64 float32 LE>",
      "language": "tr" | null, "prompt": "..." | null}
  <- {"id": 2, "ok": true, "text": "...", "language": "tr"}
  <- {"id": n, "ok": false, "error": "..."}

stdout is reserved for the protocol: library output is redirected to stderr
so a stray print can never corrupt a response. Transcripts are never logged.
"""

import base64
import json
import os
import sys

PROTOCOL_OUT = sys.stdout
sys.stdout = sys.stderr

# Whisper's own fallback ladder: greedy first, sample only when the output
# fails the compression-ratio / log-prob checks.
TEMPERATURES = (0.0, 0.2, 0.4, 0.6, 0.8, 1.0)
WARMUP_SAMPLES = 1600  # 0.1 s of silence at 16 kHz

model_dir = None


def respond(payload):
    PROTOCOL_OUT.write(json.dumps(payload, ensure_ascii=False) + "\n")
    PROTOCOL_OUT.flush()


def transcribe(np, mlx_whisper, audio, language, prompt):
    result = mlx_whisper.transcribe(
        audio,
        path_or_hf_repo=model_dir,
        language=language,
        initial_prompt=prompt,
        temperature=TEMPERATURES,
        condition_on_previous_text=False,
        word_timestamps=False,
        verbose=None,
    )
    return result.get("text", "").strip(), result.get("language")


def handle(request, np, mlx_whisper):
    global model_dir
    op = request.get("op")
    if op == "load":
        candidate = request.get("model")
        if not isinstance(candidate, str) or not os.path.isdir(candidate):
            raise ValueError("model folder not found")
        model_dir = candidate
        # mlx_whisper has no separate load call: the first transcribe loads
        # and caches the model, so warm it with a short silent clip.
        transcribe(np, mlx_whisper, np.zeros(WARMUP_SAMPLES, dtype=np.float32), "en", None)
        return {"ok": True}
    if op == "transcribe":
        if model_dir is None:
            raise RuntimeError("model not loaded")
        raw = base64.b64decode(request["audio"], validate=True)
        audio = np.frombuffer(raw, dtype="<f4").astype(np.float32)
        language = request.get("language") or None
        prompt = request.get("prompt") or None
        text, detected = transcribe(np, mlx_whisper, audio, language, prompt)
        return {"ok": True, "text": text, "language": detected}
    if op == "ping":
        return {"ok": True}
    raise ValueError(f"unknown op: {op!r}")


def main():
    try:
        import numpy as np
        import mlx_whisper
    except ImportError as exc:
        respond({"id": 0, "ok": False, "error": f"import failed: {exc}"})
        return 2

    respond({"id": 0, "ok": True, "ready": True})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        request_id = None
        try:
            request = json.loads(line)
            request_id = request.get("id")
            reply = handle(request, np, mlx_whisper)
        except Exception as exc:  # noqa: BLE001 - every failure must reach the host
            reply = {"ok": False, "error": f"{type(exc).__name__}: {exc}"}
        reply["id"] = request_id
        respond(reply)
    return 0


if __name__ == "__main__":
    sys.exit(main())
