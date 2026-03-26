#!/usr/bin/env python3
"""Embedding sidecar for tendrl-engine.

Runs a small HTTP server that generates text embeddings using
sentence-transformers. The Rust engine calls this to embed event
content and search queries.

Usage:
    python embed.py [--model MODEL] [--port PORT]
    MODEL=all-MiniLM-L6-v2 PORT=3031 python embed.py
"""

import argparse
import os
import sys

from flask import Flask, jsonify, request
from sentence_transformers import SentenceTransformer

app = Flask(__name__)

model = None
model_name = None


def get_model():
    global model, model_name
    if model is None:
        name = os.environ.get("MODEL", "all-MiniLM-L6-v2")
        print(f"Loading model: {name}", file=sys.stderr)
        model = SentenceTransformer(name)
        model_name = name
        print(
            f"Model loaded: {name} ({model.get_sentence_embedding_dimension()}d)",
            file=sys.stderr,
        )
    return model


@app.route("/health", methods=["GET"])
def health():
    m = get_model()
    return jsonify(
        {
            "status": "ok",
            "model": model_name,
            "dimensions": m.get_sentence_embedding_dimension(),
        }
    )


@app.route("/embed", methods=["POST"])
def embed():
    data = request.get_json(force=True)
    texts = data.get("texts", [])
    if not texts:
        return jsonify({"vectors": []})

    m = get_model()

    # Sub-batch to avoid OOM on large requests
    batch_size = 512
    all_vectors = []
    for i in range(0, len(texts), batch_size):
        batch = texts[i : i + batch_size]
        embeddings = m.encode(batch, normalize_embeddings=True)
        all_vectors.extend(embeddings.tolist())

    return jsonify({"vectors": all_vectors})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Embedding sidecar")
    parser.add_argument(
        "--model", default=os.environ.get("MODEL", "all-MiniLM-L6-v2")
    )
    parser.add_argument(
        "--port", type=int, default=int(os.environ.get("PORT", "3031"))
    )
    parser.add_argument("--host", default="127.0.0.1")
    args = parser.parse_args()

    os.environ["MODEL"] = args.model

    # Preload model
    get_model()

    print(f"Embedding sidecar listening on http://{args.host}:{args.port}")
    app.run(host=args.host, port=args.port, debug=False)
