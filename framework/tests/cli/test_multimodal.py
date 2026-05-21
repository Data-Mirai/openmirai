"""Tests for multimodal — asset detection, content building, model detection."""

from __future__ import annotations

import base64
import os

import pytest

from datamirai_engine.cli.multimodal import (
    DetectedAsset,
    build_multimodal_content,
    classify_asset,
    detect_assets_in_message,
    extract_image_paths,
    file_to_data_uri,
    is_image_file,
    is_vision_model,
    read_text_file,
)


# ---------------------------------------------------------------------------
# Asset classification
# ---------------------------------------------------------------------------

class TestClassifyAsset:
    def test_image_types(self):
        assert classify_asset("photo.png") == "image"
        assert classify_asset("shot.jpg") == "image"
        assert classify_asset("anim.gif") == "image"
        assert classify_asset("pic.webp") == "image"

    def test_audio_types(self):
        assert classify_asset("song.mp3") == "audio"
        assert classify_asset("voice.wav") == "audio"
        assert classify_asset("track.flac") == "audio"
        assert classify_asset("clip.m4a") == "audio"

    def test_video_types(self):
        assert classify_asset("video.mp4") == "video"
        assert classify_asset("clip.mov") == "video"
        assert classify_asset("stream.webm") == "video"

    def test_document_types(self):
        assert classify_asset("report.pdf") == "document"
        assert classify_asset("notes.txt") == "document"
        assert classify_asset("readme.md") == "document"
        assert classify_asset("data.csv") == "document"
        assert classify_asset("slides.pptx") == "document"

    def test_unknown_type(self):
        assert classify_asset("app.py") == "unknown"
        assert classify_asset("binary.bin") == "unknown"


class TestIsImageFile:
    def test_png(self):
        assert is_image_file("screenshot.png") is True

    def test_not_image(self):
        assert is_image_file("code.py") is False
        assert is_image_file("song.mp3") is False


class TestIsVisionModel:
    def test_vision_models(self):
        assert is_vision_model("llava:13b") is True
        assert is_vision_model("gpt-4o") is True
        assert is_vision_model("gemini-2.5-pro") is True

    def test_text_only_models(self):
        assert is_vision_model("qwen3:8b") is False
        assert is_vision_model("llama-3.3-70b-versatile") is False


# ---------------------------------------------------------------------------
# File operations
# ---------------------------------------------------------------------------

class TestFileToDataUri:
    def test_png_file(self, tmp_path):
        png_data = b'\x89PNG\r\n\x1a\n' + b'\x00' * 30
        img = tmp_path / "test.png"
        img.write_bytes(png_data)

        uri = file_to_data_uri(str(img))
        assert uri.startswith("data:image/png;base64,")
        decoded = base64.b64decode(uri.split(",", 1)[1])
        assert decoded == png_data

    def test_mp3_file(self, tmp_path):
        audio = tmp_path / "test.mp3"
        audio.write_bytes(b'\xff\xfb\x90\x00' + b'\x00' * 20)
        uri = file_to_data_uri(str(audio))
        assert uri.startswith("data:audio/mpeg;base64,")

    def test_file_not_found(self):
        with pytest.raises(FileNotFoundError):
            file_to_data_uri("/nonexistent/path.png")


class TestReadTextFile:
    def test_read_normal(self, tmp_path):
        f = tmp_path / "readme.md"
        f.write_text("# Hello\nWorld")
        content = read_text_file(str(f))
        assert "# Hello" in content
        assert "World" in content

    def test_truncation(self, tmp_path):
        f = tmp_path / "big.txt"
        f.write_text("x" * 20_000)
        content = read_text_file(str(f), max_chars=100)
        assert len(content) < 200
        assert "truncated" in content

    def test_file_not_found(self):
        with pytest.raises(FileNotFoundError):
            read_text_file("/fake/path.txt")


# ---------------------------------------------------------------------------
# Asset detection in user messages
# ---------------------------------------------------------------------------

class TestDetectAssetsInMessage:
    def test_no_assets(self):
        text, assets = detect_assets_in_message("just a normal message")
        assert text == "just a normal message"
        assert assets == []

    def test_detect_image_absolute_path(self, tmp_path):
        img = tmp_path / "screenshot.png"
        img.write_bytes(b'\x89PNG' + b'\x00' * 20)

        text, assets = detect_assets_in_message(
            f"mira este screenshot {img} y dime qué ves",
            cwd=str(tmp_path),
        )
        assert len(assets) == 1
        assert assets[0].asset_type == "image"
        assert assets[0].path == str(img)
        assert "dime qué ves" in text

    def test_detect_relative_filename(self, tmp_path):
        img = tmp_path / "photo.jpg"
        img.write_bytes(b'\xff\xd8\xff' + b'\x00' * 20)

        text, assets = detect_assets_in_message(
            "analiza photo.jpg",
            cwd=str(tmp_path),
        )
        assert len(assets) == 1
        assert assets[0].asset_type == "image"

    def test_detect_audio(self, tmp_path):
        audio = tmp_path / "recording.mp3"
        audio.write_bytes(b'\xff\xfb' + b'\x00' * 20)

        text, assets = detect_assets_in_message(
            "transcribe recording.mp3",
            cwd=str(tmp_path),
        )
        assert len(assets) == 1
        assert assets[0].asset_type == "audio"

    def test_detect_document(self, tmp_path):
        doc = tmp_path / "report.pdf"
        doc.write_bytes(b'%PDF-1.4' + b'\x00' * 20)

        text, assets = detect_assets_in_message(
            "revisa report.pdf por favor",
            cwd=str(tmp_path),
        )
        assert len(assets) == 1
        assert assets[0].asset_type == "document"

    def test_detect_multiple_assets(self, tmp_path):
        (tmp_path / "a.png").write_bytes(b'\x89PNG' + b'\x00' * 20)
        (tmp_path / "data.csv").write_text("col1,col2\n1,2")

        text, assets = detect_assets_in_message(
            "compara a.png con data.csv",
            cwd=str(tmp_path),
        )
        assert len(assets) == 2
        types = {a.asset_type for a in assets}
        assert "image" in types
        assert "document" in types

    def test_nonexistent_file_ignored(self):
        text, assets = detect_assets_in_message("open fake_file.png")
        assert assets == []

    def test_explicit_image_command(self, tmp_path):
        img = tmp_path / "shot.png"
        img.write_bytes(b'\x89PNG' + b'\x00' * 20)

        text, assets = detect_assets_in_message(
            f"/image {img} what is this?",
            cwd=str(tmp_path),
        )
        assert len(assets) == 1
        assert assets[0].asset_type == "image"

    def test_explicit_file_command(self, tmp_path):
        doc = tmp_path / "notes.txt"
        doc.write_text("important notes")

        text, assets = detect_assets_in_message(
            f"/file {doc} summarize this",
            cwd=str(tmp_path),
        )
        assert len(assets) == 1
        assert assets[0].asset_type == "document"

    def test_assets_only_generates_default_text(self, tmp_path):
        img = tmp_path / "x.png"
        img.write_bytes(b'\x89PNG' + b'\x00' * 20)

        text, assets = detect_assets_in_message(f"{img}", cwd=str(tmp_path))
        assert len(assets) == 1
        assert "Analyze" in text  # default prompt


# ---------------------------------------------------------------------------
# Build multimodal content
# ---------------------------------------------------------------------------

class TestBuildMultimodalContent:
    def test_text_only(self):
        result = build_multimodal_content("hello world")
        assert result == "hello world"
        assert isinstance(result, str)

    def test_no_assets_returns_string(self):
        result = build_multimodal_content("hello", assets=None)
        assert isinstance(result, str)

    def test_with_image_asset(self, tmp_path):
        img = tmp_path / "test.png"
        img.write_bytes(b'\x89PNG' + b'\x00' * 30)

        asset = DetectedAsset(
            path=str(img), original_ref="test.png",
            asset_type="image", mime_type="image/png", size_bytes=34,
        )
        result = build_multimodal_content("describe", assets=[asset])
        assert isinstance(result, list)
        assert result[0]["type"] == "text"
        assert result[1]["type"] == "image_url"

    def test_with_audio_asset(self, tmp_path):
        audio = tmp_path / "test.mp3"
        audio.write_bytes(b'\xff\xfb' + b'\x00' * 20)

        asset = DetectedAsset(
            path=str(audio), original_ref="test.mp3",
            asset_type="audio", mime_type="audio/mpeg", size_bytes=22,
        )
        result = build_multimodal_content("transcribe", assets=[asset])
        assert isinstance(result, list)
        assert result[1]["type"] == "input_audio"

    def test_with_text_document(self, tmp_path):
        doc = tmp_path / "readme.md"
        doc.write_text("# Project\nDescription here")

        asset = DetectedAsset(
            path=str(doc), original_ref="readme.md",
            asset_type="document", mime_type="text/markdown", size_bytes=30,
        )
        result = build_multimodal_content("summarize", assets=[asset])
        assert isinstance(result, list)
        assert "Content of readme.md" in result[1]["text"]
        assert "# Project" in result[1]["text"]

    def test_with_pdf_document(self, tmp_path):
        doc = tmp_path / "report.pdf"
        doc.write_bytes(b'%PDF-1.4' + b'\x00' * 50)

        asset = DetectedAsset(
            path=str(doc), original_ref="report.pdf",
            asset_type="document", mime_type="application/pdf", size_bytes=58,
        )
        result = build_multimodal_content("analyze", assets=[asset])
        assert isinstance(result, list)
        assert result[1]["type"] == "file"
        assert result[1]["file"]["name"] == "report.pdf"

    def test_with_video_asset(self, tmp_path):
        vid = tmp_path / "clip.mp4"
        vid.write_bytes(b'\x00' * 100)

        asset = DetectedAsset(
            path=str(vid), original_ref="clip.mp4",
            asset_type="video", mime_type="video/mp4", size_bytes=100,
        )
        result = build_multimodal_content("what's in this", assets=[asset])
        assert isinstance(result, list)
        assert "video" in result[1]["text"].lower()

    def test_mixed_assets(self, tmp_path):
        img = tmp_path / "photo.png"
        img.write_bytes(b'\x89PNG' + b'\x00' * 20)
        doc = tmp_path / "notes.txt"
        doc.write_text("some notes")

        assets = [
            DetectedAsset(path=str(img), original_ref="photo.png",
                         asset_type="image", mime_type="image/png", size_bytes=24),
            DetectedAsset(path=str(doc), original_ref="notes.txt",
                         asset_type="document", mime_type="text/plain", size_bytes=10),
        ]
        result = build_multimodal_content("compare these", assets=assets)
        assert isinstance(result, list)
        assert len(result) == 3  # text + image + document

    def test_backward_compat_image_paths(self, tmp_path):
        img = tmp_path / "x.png"
        img.write_bytes(b'\x89PNG' + b'\x00' * 20)

        result = build_multimodal_content("test", image_paths=[str(img)])
        assert isinstance(result, list)
        assert result[1]["type"] == "image_url"


# ---------------------------------------------------------------------------
# Legacy extract_image_paths (backward compat)
# ---------------------------------------------------------------------------

class TestExtractImagePathsLegacy:
    def test_no_images(self):
        text, paths = extract_image_paths("just text")
        assert paths == []

    def test_single_image(self):
        text, paths = extract_image_paths("/image screenshot.png what is this?")
        assert "screenshot.png" in paths
