"""Tests for RenderEngine — Markdown to HTML conversion."""

from datamirai_engine.render.engine import RenderEngine
from datamirai_engine.render.themes import get_theme, THEME_DEFAULT, THEME_DARK


class TestRenderEngine:
    def setup_method(self):
        self.engine = RenderEngine()

    def test_basic_render(self):
        md = "# Hello\n\nWorld"
        html = self.engine.render(md, title="Test")
        assert "<!DOCTYPE html>" in html
        assert "<h1" in html
        assert "Hello" in html
        assert "World" in html

    def test_self_contained(self):
        """REGLA-367: HTML must be self-contained."""
        html = self.engine.render("# Test", title="Test")
        assert "<style>" in html
        assert "<!DOCTYPE html>" in html
        assert "</html>" in html
        # No external references
        assert 'href="http' not in html.split("<main>")[0]  # no external CSS in head
        assert "<script" not in html

    def test_headings_with_ids(self):
        md = "# First\n\n## Second\n\n### Third"
        html = self.engine.render(md)
        assert 'id="first"' in html
        assert 'id="second"' in html
        assert 'id="third"' in html

    def test_table_rendering(self):
        md = "| Name | Age |\n| --- | --- |\n| Alice | 30 |\n| Bob | 25 |"
        html = self.engine.render(md)
        assert "<table>" in html
        assert "<th>" in html
        assert "Alice" in html

    def test_code_block(self):
        md = "```python\ndef hello():\n    pass\n```"
        html = self.engine.render(md)
        assert "<pre>" in html
        assert "<code" in html
        assert "def hello():" in html

    def test_bold_and_italic(self):
        md = "**bold** and *italic*"
        html = self.engine.render(md)
        assert "<strong>bold</strong>" in html
        assert "<em>italic</em>" in html

    def test_links(self):
        md = "[Example](https://example.com)"
        html = self.engine.render(md)
        assert 'href="https://example.com"' in html
        assert "Example" in html

    def test_images(self):
        md = "![Alt text](image.png)"
        html = self.engine.render(md)
        assert 'src="image.png"' in html
        assert 'alt="Alt text"' in html

    def test_blockquote(self):
        md = "> This is a quote"
        html = self.engine.render(md)
        assert "<blockquote>" in html
        assert "This is a quote" in html

    def test_unordered_list(self):
        md = "- Apple\n- Banana\n- Cherry"
        html = self.engine.render(md)
        assert "<ul>" in html
        assert "<li>" in html
        assert "Apple" in html

    def test_ordered_list(self):
        md = "1. First\n2. Second\n3. Third"
        html = self.engine.render(md)
        assert "<ol>" in html
        assert "First" in html

    def test_horizontal_rule(self):
        md = "Before\n\n---\n\nAfter"
        html = self.engine.render(md)
        assert "<hr>" in html

    def test_toc_generated(self):
        md = "# Section 1\n\nContent\n\n## Section 2\n\nMore content"
        html = self.engine.render(md)
        assert 'class="toc"' in html
        assert "Section 1" in html

    def test_toc_not_generated_for_single_heading(self):
        md = "# Only One\n\nContent"
        html = self.engine.render(md)
        assert 'class="toc"' not in html

    def test_wiki_links_rendered_as_broken(self):
        """REGLA-368: Without vault, wiki-links are visible as broken."""
        md = "See [[some-note]] for details."
        html = self.engine.render(md, resolve_links=True)
        assert "vault-link-broken" in html
        assert "some-note" in html

    def test_theme_applied(self):
        md = "# Hello"
        html_default = self.engine.render(md, theme="default")
        html_dark = self.engine.render(md, theme="dark")
        # Both should have CSS but different content
        assert "background: #ffffff" in html_default
        assert "background: #0f172a" in html_dark

    def test_no_js_execution(self):
        """REGLA-370: Parser must not execute JS from code blocks."""
        md = "```javascript\nalert('xss')\n```"
        html = self.engine.render(md)
        # JS should be escaped in code block, not executable
        assert "<script>" not in html
        assert "alert" in html  # present as text, not executed

    def test_title_in_metadata(self):
        html = self.engine.render("# Content", title="My Report")
        assert "My Report" in html
        assert 'class="metadata"' in html

    def test_inline_code(self):
        md = "Use `pip install` to install."
        html = self.engine.render(md)
        assert "<code>" in html
        assert "pip install" in html


class TestThemes:
    def test_all_themes_exist(self):
        for name in ("default", "dark", "minimal", "report"):
            css = get_theme(name)
            assert len(css) > 100  # non-trivial CSS
            assert "body" in css

    def test_no_js_in_themes(self):
        """REGLA-371: Themes are CSS-only."""
        for name in ("default", "dark", "minimal", "report"):
            css = get_theme(name)
            assert "<script>" not in css
            assert "javascript:" not in css
            assert "onclick" not in css

    def test_unknown_theme_falls_back(self):
        css = get_theme("nonexistent")
        assert css == THEME_DEFAULT
