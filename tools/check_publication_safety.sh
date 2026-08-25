#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"

if [[ "$(git -C "$repo_root" rev-parse --show-toplevel)" != "$repo_root" ]]; then
  echo "Could not resolve the OpenMirai repository root." >&2
  exit 2
fi

cd "$repo_root"

# Scan the working tree, including untracked files. On pull requests, also scan
# every commit introduced by the PR so a secret added and removed in a later
# commit cannot hide in public Git history.
refs=()
base_ref="${PUBLICATION_BASE_REF:-}"
if [[ -n "$base_ref" ]] && git cat-file -e "${base_ref}^{commit}" 2>/dev/null; then
  while IFS= read -r commit; do
    refs+=("$commit")
  done < <(git rev-list --reverse "${base_ref}..HEAD")
fi

pathspecs=(
  .
  ':(exclude)Cargo.lock'
  ':(exclude)**/package-lock.json'
  ':(exclude)tools/check_publication_safety.sh'
)

failed=0

scan_pattern() {
  local label="$1"
  local pattern="$2"
  local ref hits

  hits="$(
    rg -l -I --hidden --pcre2 \
      --glob '!.git/**' \
      --glob '!target/**' \
      --glob '!Cargo.lock' \
      --glob '!**/package-lock.json' \
      --glob '!tools/check_publication_safety.sh' \
      -- "$pattern" . 2>/dev/null || true
  )"
  if [[ -n "$hits" ]]; then
    echo "ERROR: $label detected in the working tree. Matching file(s):" >&2
    printf '%s\n' "$hits" | sort -u >&2
    failed=1
  fi

  for ref in "${refs[@]}"; do
    hits="$(git grep -Il -P "$pattern" "$ref" -- "${pathspecs[@]}" 2>/dev/null || true)"
    if [[ -n "$hits" ]]; then
      echo "ERROR: $label detected in $ref. Matching file(s):" >&2
      printf '%s\n' "$hits" | sed -E 's/^[^:]+://' | sort -u >&2
      failed=1
    fi
  done
}

# High-confidence credential formats. Only filenames are printed; a discovered
# credential must never be echoed into CI logs.
scan_pattern "private key" '-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----'
scan_pattern "GitHub token" '(?:ghp|github_pat)_[A-Za-z0-9_]{20,}'
scan_pattern "AWS access key" '(?:AKIA|ASIA)[A-Z0-9]{16}'
scan_pattern "Google API key" 'AIza[0-9A-Za-z_-]{35}'
scan_pattern "OpenAI API key" 'sk-(?:proj-)?[A-Za-z0-9_-]{20,}'
scan_pattern "Anthropic API key" 'sk-ant-[A-Za-z0-9_-]{20,}'
scan_pattern "Slack token" 'xox[baprs]-[A-Za-z0-9-]{10,}'
scan_pattern "credential-bearing URL" 'https?://[^/@[:space:]]+:[^/@[:space:]]+@'

# Privacy-boundary indicators. Synthetic paths use /home/user or /Users/alice.
scan_pattern "personal home-directory path" '/Users/(?!alice(?:/|\b)|user(?:/|\b)|example(?:/|\b))[A-Za-z0-9._-]+/'
scan_pattern "personal home-directory path" '/home/(?!user(?:/|\b)|runner(?:/|\b)|example(?:/|\b))[A-Za-z0-9._-]+/'
scan_pattern "local file URL" 'file:///'
scan_pattern "non-public organization repository link" 'github\.com/Data-Mirai/(?!openmirai(?:\.git)?(?:[/?#[:space:])>"]|$))[^[:space:])>]+'
scan_pattern "private-network tailnet address" '100\.(?:6[4-9]|[789][0-9]|1[01][0-9]|12[0-7])\.[0-9]{1,3}\.[0-9]{1,3}'

if (( failed != 0 )); then
  echo "Publication safety check failed. See PUBLICATION_POLICY.md." >&2
  exit 1
fi

echo "Publication safety check passed. Manual source-classification review is still required."
