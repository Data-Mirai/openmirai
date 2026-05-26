"""Tests for HTML to Markdown converter."""

from datamirai_engine.tools.builtin.data.html_to_markdown import (
    extract_main_content,
    html_to_markdown,
)


class TestHtmlToMarkdown:
    def test_headings(self):
        html = "<h1>Title</h1><h2>Subtitle</h2><h3>Section</h3>"
        md = html_to_markdown(html, extract_main=False)
        assert "# Title" in md
        assert "## Subtitle" in md
        assert "### Section" in md

    def test_paragraphs(self):
        html = "<p>First paragraph.</p><p>Second paragraph.</p>"
        md = html_to_markdown(html, extract_main=False)
        assert "First paragraph." in md
        assert "Second paragraph." in md

    def test_bold_and_italic(self):
        html = "<p><strong>bold</strong> and <em>italic</em></p>"
        md = html_to_markdown(html, extract_main=False)
        assert "**bold**" in md
        assert "*italic*" in md

    def test_links(self):
        html = '<p>Visit <a href="https://example.com">Example</a></p>'
        md = html_to_markdown(html, extract_main=False)
        assert "[Example](https://example.com)" in md

    def test_images(self):
        html = '<img src="photo.jpg" alt="A photo">'
        md = html_to_markdown(html, extract_main=False)
        assert "![A photo](photo.jpg)" in md

    def test_unordered_list(self):
        html = "<ul><li>Apple</li><li>Banana</li></ul>"
        md = html_to_markdown(html, extract_main=False)
        assert "- Apple" in md
        assert "- Banana" in md

    def test_ordered_list(self):
        html = "<ol><li>First</li><li>Second</li></ol>"
        md = html_to_markdown(html, extract_main=False)
        assert "1. First" in md
        assert "2. Second" in md

    def test_table(self):
        html = """
        <table>
            <tr><th>Name</th><th>Age</th></tr>
            <tr><td>Alice</td><td>30</td></tr>
            <tr><td>Bob</td><td>25</td></tr>
        </table>
        """
        md = html_to_markdown(html, extract_main=False)
        assert "Name" in md
        assert "Age" in md
        assert "Alice" in md
        assert "|" in md

    def test_code_inline(self):
        html = "<p>Use <code>pip install</code> to install.</p>"
        md = html_to_markdown(html, extract_main=False)
        assert "`pip install`" in md

    def test_code_block(self):
        html = "<pre><code>def hello():\n    print('hi')</code></pre>"
        md = html_to_markdown(html, extract_main=False)
        assert "```" in md
        assert "def hello():" in md

    def test_blockquote(self):
        html = "<blockquote>A wise quote.</blockquote>"
        md = html_to_markdown(html, extract_main=False)
        assert "> A wise quote." in md

    def test_horizontal_rule(self):
        html = "<p>Before</p><hr><p>After</p>"
        md = html_to_markdown(html, extract_main=False)
        assert "---" in md

    def test_skips_scripts(self):
        html = "<p>Content</p><script>alert('xss')</script><p>More</p>"
        md = html_to_markdown(html, extract_main=False)
        assert "Content" in md
        assert "alert" not in md

    def test_skips_styles(self):
        html = "<style>body{color:red}</style><p>Content</p>"
        md = html_to_markdown(html, extract_main=False)
        assert "Content" in md
        assert "color:red" not in md

    def test_no_css_js_in_output(self):
        """REGLA-354: Never include CSS, JS, or HTML attributes."""
        html = '<div class="fancy" style="color:red"><p>Text</p></div>'
        md = html_to_markdown(html, extract_main=False)
        assert "class=" not in md
        assert "style=" not in md
        assert "Text" in md


class TestExtractMainContent:
    def test_extracts_article(self):
        html = "<nav>Menu</nav><article><p>Main content</p></article><footer>Foot</footer>"
        main = extract_main_content(html)
        assert "Main content" in main
        assert "Menu" not in main

    def test_extracts_main_tag(self):
        html = "<header>Top</header><main><p>Body here</p></main>"
        main = extract_main_content(html)
        assert "Body here" in main

    def test_fallback_strips_chrome(self):
        html = "<nav>Nav</nav><script>js()</script><p>Content</p><footer>Foot</footer>"
        main = extract_main_content(html)
        assert "Content" in main
        assert "Nav" not in main
        assert "js()" not in main
