"""multimodal — asset detection and content building for the agentic terminal.

User messages can naturally contain file references inline with text:
    "mira este screenshot /Users/gabo/img.png y dime qué ves"
    "analiza este audio recording.mp3 y transcribe"
    "revisa este PDF report.pdf"

This module:
1. Scans user text for file paths that exist on disk
2. Classifies each asset (image, audio, video, document)
3. Builds the appropriate multi-part content for the LLM
"""

from __future__ import annotations

import base64
import mimetypes
import os
import re
from dataclasses import dataclass
from typing import Any


# ---------------------------------------------------------------------------
# Asset type classification
# ---------------------------------------------------------------------------

_IMAGE_EXT = frozenset({".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".tiff", ".svg"})
_AUDIO_EXT = frozenset({".mp3", ".wav", ".ogg", ".flac", ".m4a", ".aac", ".wma"})
_VIDEO_EXT = frozenset({".mp4", ".mov", ".avi", ".mkv", ".webm", ".flv"})
_DOC_EXT = frozenset({".pdf", ".doc", ".docx", ".txt", ".md", ".csv", ".xls", ".xlsx", ".pptx"})

ALL_ASSET_EXT = _IMAGE_EXT | _AUDIO_EXT | _VIDEO_EXT | _DOC_EXT


@dataclass(frozen=True)
class DetectedAsset:
    """A file asset detected in the user's message."""
    path: str           # absolute path
    original_ref: str   # as referenced in the message
    asset_type: str     # image | audio | video | document
    mime_type: str
    size_bytes: int


def classify_asset(path: str) -> str:
    """Classify a file path into an asset type."""
    ext = os.path.splitext(path)[1].lower()
    if ext in _IMAGE_EXT:
        return "image"
    if ext in _AUDIO_EXT:
        return "audio"
    if ext in _VIDEO_EXT:
        return "video"
    if ext in _DOC_EXT:
        return "document"
    return "unknown"


def is_image_file(path: str) -> bool:
    ext = os.path.splitext(path)[1].lower()
    return ext in _IMAGE_EXT


# ---------------------------------------------------------------------------
# Vision model detection
# ---------------------------------------------------------------------------

_VISION_PATTERNS = (
    "llava", "vision", "bakllava", "moondream", "minicpm-v",
    "cogvlm", "gpt-4o", "gpt-4-turbo", "gemini", "claude-3",
    "pixtral", "llama-3.2-vision",
)


def is_vision_model(model_id: str) -> bool:
    lower = model_id.lower()
    return any(p in lower for p in _VISION_PATTERNS)


# ---------------------------------------------------------------------------
# File → data URI / content extraction
# ---------------------------------------------------------------------------

def file_to_data_uri(file_path: str) -> str:
    """Convert any file to a base64 data URI."""
    path = os.path.abspath(file_path)
    if not os.path.isfile(path):
        raise FileNotFoundError(f"File not found: {path}")

    mime, _ = mimetypes.guess_type(path)
    if not mime:
        mime = "application/octet-stream"

    with open(path, "rb") as f:
        data = base64.b64encode(f.read()).decode("ascii")

    return f"data:{mime};base64,{data}"


def read_text_file(file_path: str, max_chars: int = 10_000) -> str:
    """Read a text-based file and return its content (truncated if large)."""
    path = os.path.abspath(file_path)
    if not os.path.isfile(path):
        raise FileNotFoundError(f"File not found: {path}")
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            content = f.read(max_chars + 1)
        if len(content) > max_chars:
            content = content[:max_chars] + f"\n... (truncated at {max_chars} chars)"
        return content
    except Exception as e:
        return f"(could not read file: {e})"


# backward compat alias
image_to_data_uri = file_to_data_uri


# ---------------------------------------------------------------------------
# Asset detection in user messages
# ---------------------------------------------------------------------------

# Regex: match file paths (absolute, relative, or just filename with known extension)
_PATH_PATTERN = re.compile(
    r"""(?:^|\s)"""                          # start or whitespace
    r"""("""
    r"""(?:[~/.][\w./\-\s]*?)"""             # paths starting with ~ / . /
    r"""|"""
    r"""(?:[\w\-]+)"""                       # or just a filename
    r""")"""
    r"""(\.(?:"""
    + "|".join(ext.lstrip(".") for ext in sorted(ALL_ASSET_EXT))
    + r"""))"""
    r"""(?=\s|$|[,;:!?)]|")""",             # followed by space/end/punctuation
    re.IGNORECASE,
)


def detect_assets_in_message(text: str, cwd: str = ".") -> tuple[str, list[DetectedAsset]]:
    """Scan user text for file references. Returns (cleaned_text, assets).

    Works with:
      - Absolute paths: /Users/gabo/screenshot.png
      - Relative paths: ./img/photo.jpg
      - Home paths: ~/Desktop/report.pdf
      - Bare filenames: screenshot.png (resolved against cwd)
    """
    assets: list[DetectedAsset] = []
    spans_to_remove: list[tuple[int, int]] = []
    cwd = os.path.abspath(cwd)

    for match in _PATH_PATTERN.finditer(text):
        raw_ref = (match.group(1) + match.group(2)).strip()
        full_start = match.start(1)
        full_end = match.end(2)

        # Resolve path
        if raw_ref.startswith("~"):
            resolved = os.path.expanduser(raw_ref)
        elif os.path.isabs(raw_ref):
            resolved = raw_ref
        else:
            resolved = os.path.join(cwd, raw_ref)

        resolved = os.path.abspath(resolved)

        if not os.path.isfile(resolved):
            continue

        asset_type = classify_asset(resolved)
        mime, _ = mimetypes.guess_type(resolved)
        size = os.path.getsize(resolved)

        assets.append(DetectedAsset(
            path=resolved,
            original_ref=raw_ref,
            asset_type=asset_type,
            mime_type=mime or "application/octet-stream",
            size_bytes=size,
        ))
        spans_to_remove.append((full_start, full_end))

    # Also support explicit /image, /audio, /file commands
    explicit_pattern = re.compile(r"/(?:image|audio|file|attach)\s+(\S+)", re.IGNORECASE)
    for match in explicit_pattern.finditer(text):
        raw_ref = match.group(1)
        if raw_ref.startswith("~"):
            resolved = os.path.expanduser(raw_ref)
        elif os.path.isabs(raw_ref):
            resolved = raw_ref
        else:
            resolved = os.path.join(cwd, raw_ref)
        resolved = os.path.abspath(resolved)

        if not os.path.isfile(resolved):
            continue

        # Avoid duplicates
        if any(a.path == resolved for a in assets):
            continue

        asset_type = classify_asset(resolved)
        mime, _ = mimetypes.guess_type(resolved)
        size = os.path.getsize(resolved)

        assets.append(DetectedAsset(
            path=resolved,
            original_ref=raw_ref,
            asset_type=asset_type,
            mime_type=mime or "application/octet-stream",
            size_bytes=size,
        ))
        spans_to_remove.append((match.start(), match.end()))

    # Remove asset references from text (clean up)
    if spans_to_remove:
        # Sort spans in reverse to not mess up indices
        spans_to_remove.sort(reverse=True)
        cleaned = text
        for start, end in spans_to_remove:
            cleaned = cleaned[:start] + cleaned[end:]
        cleaned = re.sub(r"\s+", " ", cleaned).strip()
    else:
        cleaned = text

    if not cleaned and assets:
        type_names = ", ".join(a.asset_type for a in assets)
        cleaned = f"Analyze this {type_names}."

    return cleaned, assets


# ---------------------------------------------------------------------------
# Build multimodal content for the LLM
# ---------------------------------------------------------------------------

def build_multimodal_content(
    text: str,
    assets: list[DetectedAsset] | None = None,
    image_paths: list[str] | None = None,
) -> str | list[dict[str, Any]]:
    """Build message content with inline assets.

    Supports:
    - Images → base64 data URI (OpenAI image_url format)
    - Audio → base64 data URI (for models that support audio)
    - Documents (txt, md, csv) → inline text content
    - Other docs (pdf, docx) → base64 data URI

    If no assets, returns plain string.
    """
    # Backward compat: convert image_paths to assets
    if image_paths and not assets:
        assets = []
        for p in image_paths:
            resolved = os.path.abspath(p)
            if os.path.isfile(resolved):
                assets.append(DetectedAsset(
                    path=resolved,
                    original_ref=p,
                    asset_type="image",
                    mime_type=mimetypes.guess_type(resolved)[0] or "image/png",
                    size_bytes=os.path.getsize(resolved),
                ))

    if not assets:
        return text

    parts: list[dict[str, Any]] = []

    if text:
        parts.append({"type": "text", "text": text})

    for asset in assets:
        if asset.asset_type == "image":
            try:
                data_uri = file_to_data_uri(asset.path)
                parts.append({
                    "type": "image_url",
                    "image_url": {"url": data_uri},
                })
            except FileNotFoundError:
                parts.append({"type": "text", "text": f"(image not found: {asset.original_ref})"})

        elif asset.asset_type == "audio":
            try:
                data_uri = file_to_data_uri(asset.path)
                parts.append({
                    "type": "input_audio",
                    "input_audio": {"data": data_uri.split(",", 1)[1], "format": asset.path.rsplit(".", 1)[-1]},
                })
            except FileNotFoundError:
                parts.append({"type": "text", "text": f"(audio not found: {asset.original_ref})"})

        elif asset.asset_type == "document":
            ext = os.path.splitext(asset.path)[1].lower()
            if ext in (".txt", ".md", ".csv"):
                content = read_text_file(asset.path)
                parts.append({
                    "type": "text",
                    "text": f"--- Content of {asset.original_ref} ---\n{content}\n--- End ---",
                })
            else:
                # PDF, docx, etc. → base64 for models that support it
                try:
                    data_uri = file_to_data_uri(asset.path)
                    parts.append({
                        "type": "file",
                        "file": {"url": data_uri, "name": os.path.basename(asset.path)},
                    })
                except FileNotFoundError:
                    parts.append({"type": "text", "text": f"(file not found: {asset.original_ref})"})

        elif asset.asset_type == "video":
            parts.append({
                "type": "text",
                "text": f"(video file referenced: {asset.original_ref} — {asset.size_bytes} bytes. Video content not directly supported, use system_bash to extract frames or metadata.)",
            })

    return parts


# Backward compat
def extract_image_paths(text: str) -> tuple[str, list[str]]:
    """Legacy: extract /image commands. Use detect_assets_in_message instead."""
    pattern = re.compile(r"/image\s+(\S+\.(?:png|jpg|jpeg|gif|webp|bmp|svg))", re.IGNORECASE)
    paths: list[str] = []
    for match in pattern.finditer(text):
        paths.append(match.group(1))
    cleaned = pattern.sub("", text).strip()
    if not cleaned and paths:
        cleaned = "Describe this image."
    return cleaned, paths
