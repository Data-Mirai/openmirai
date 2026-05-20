"""Tests for vault parser — frontmatter, wiki-links, slugify."""

from datamirai_engine.vault.parser import (
    build_note_markdown,
    extract_wiki_links,
    parse_frontmatter,
    slugify,
)


class TestParseFrontmatter:
    def test_valid_frontmatter(self):
        md = "---\ntitle: Test Note\ntags: [a, b]\n---\n\n# Content here"
        fm, body = parse_frontmatter(md)
        assert fm["title"] == "Test Note"
        assert fm["tags"] == ["a", "b"]
        assert body.strip() == "# Content here"

    def test_no_frontmatter(self):
        md = "# Just content\n\nNo frontmatter here."
        fm, body = parse_frontmatter(md)
        assert fm == {}
        assert body == md

    def test_invalid_yaml(self):
        md = "---\n: invalid: yaml: [[\n---\n\nBody"
        fm, body = parse_frontmatter(md)
        assert fm == {}
        assert body == md

    def test_empty_string(self):
        fm, body = parse_frontmatter("")
        assert fm == {}
        assert body == ""

    def test_frontmatter_not_dict(self):
        md = "---\n- just a list\n- not a dict\n---\n\nBody"
        fm, body = parse_frontmatter(md)
        assert fm == {}

    def test_no_closing_delimiter(self):
        md = "---\ntitle: No close\nBody continues"
        fm, body = parse_frontmatter(md)
        assert fm == {}
        assert body == md


class TestExtractWikiLinks:
    def test_simple_links(self):
        content = "See [[fed-news]] and [[sp500-analysis]]."
        links = extract_wiki_links(content)
        assert len(links) == 2
        assert links[0] == ("fed-news", None)
        assert links[1] == ("sp500-analysis", None)

    def test_aliased_link(self):
        content = "See [[fed-news|Federal Reserve Decision]]."
        links = extract_wiki_links(content)
        assert len(links) == 1
        assert links[0] == ("fed-news", "Federal Reserve Decision")

    def test_no_links(self):
        content = "No wiki links here. Just [regular](http://example.com) links."
        links = extract_wiki_links(content)
        assert len(links) == 0

    def test_deduplicate(self):
        content = "See [[note-a]] and then [[note-a]] again."
        links = extract_wiki_links(content)
        assert len(links) == 1

    def test_path_links(self):
        content = "See [[scrapes/fed-news]] for details."
        links = extract_wiki_links(content)
        assert links[0] == ("scrapes/fed-news", None)

    def test_empty_brackets(self):
        content = "This [[]] should not match."
        links = extract_wiki_links(content)
        assert len(links) == 0


class TestBuildNoteMarkdown:
    def test_with_frontmatter(self):
        fm = {"title": "Test", "tags": ["a", "b"]}
        md = build_note_markdown(fm, "# Content")
        assert md.startswith("---\n")
        assert "title: Test" in md
        assert "# Content" in md

    def test_without_frontmatter(self):
        md = build_note_markdown({}, "# Content")
        assert md == "# Content"


class TestSlugify:
    def test_basic(self):
        assert slugify("Hello World") == "hello-world"

    def test_special_chars(self):
        assert slugify("Hello! @World #2026") == "hello-world-2026"

    def test_accents(self):
        assert slugify("Café résumé") == "cafe-resume"

    def test_max_length(self):
        result = slugify("a" * 200, max_len=80)
        assert len(result) <= 80

    def test_empty(self):
        assert slugify("") == "untitled"
        assert slugify("!!!") == "untitled"

    def test_leading_trailing_hyphens(self):
        assert slugify("--hello--") == "hello"
