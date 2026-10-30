#!/usr/bin/env bash
# English-only guard (convention C49): scan tracked artifacts for CJK characters.
# Scanned: crates/, bindings/, www/, scripts/, docs/, config templates, pixi.toml,
#          Dockerfile, mediaservo.sh, CHANGELOG.md, AGENTS.md, README.md, SKILL.md,
#          .github/workflows, .agents/rules, .agents/skills.
# Exempt:  .agents/memorys (established Chinese corpus), git history, vendored dirs
#          (never tracked under above globs).
# Usage: bash scripts/check-english-only.sh [--summary]
# Exit:  0 = clean, 1 = CJK found (prints offenders), 2 = setup error
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

SUMMARY=0
[ "${1:-}" = "--summary" ] && SUMMARY=1

# File-level allowlist (path|reason); used only for historical/translation artifacts.
ALLOW_FILE=scripts/english-only-allow.txt
declare -A ALLOW=()
if [ -f "$ALLOW_FILE" ]; then
  while IFS='|' read -r allow_path allow_reason; do
    case "$allow_path" in ''|'#'*) continue ;; esac
    ALLOW["$allow_path"]=1
  done < "$ALLOW_FILE"
fi

# Collect tracked files matching artifact globs (respects gitignore automatically)
mapfile -t FILES < <(git ls-files \
  'crates/**/*.rs' \
  'bindings/**/*.rs' 'bindings/**/*.c' 'bindings/**/*.cpp' 'bindings/**/*.hpp' 'bindings/**/*.h' \
  'bindings/**/*.py' 'bindings/**/*.ts' 'bindings/**/*.js' 'bindings/**/*.md' \
  'bindings/**/*.toml' 'bindings/**/*.json' \
  'www/**/*.ts' 'www/**/*.tsx' 'www/**/*.md' \
  'scripts/*.py' 'scripts/*.sh' \
  'docs/**/*.md' 'docs/**/*.yml' 'docs/**/*.yaml' \
  'config/*.yaml' 'config/*.conf' 'data/etc/*.yaml' \
  'crates/**/*.yaml' 'crates/**/*.md' 'crates/**/*.template' \
  '.github/workflows/*.yml' \
  '.agents/rules/**/*.md' \
  '.agents/skills/**/SKILL.md' \
  'pixi.toml' 'Dockerfile' 'mediaservo.sh' 'CHANGELOG.md' 'AGENTS.md' 'README.md' 'SKILL.md' \
  | grep -v '^\.agents/memorys/' | grep -v '^vendor/')

if [ "${#FILES[@]}" -eq 0 ]; then
  echo "check-english-only: no files matched (git ls-files empty?)"
  exit 2
fi

# Functional-CJK exemption: lines carrying the pragma `c49:trigger` (bilingual skill
# trigger keywords that MUST match user Chinese input) are not counted as offenders.
# grep -P CJK range, minus pragma lines
count_cjk() { # $1 = file; prints count of CJK lines lacking the c49:trigger pragma
  [ -n "${ALLOW[$1]:-}" ] && { echo 0; return 0; }
  grep -P '[\x{4e00}-\x{9fff}]' -- "$1" 2>/dev/null | grep -cv 'c49:trigger' || true
}
if [ "${1:-}" = "--self-test" ]; then
  mkdir -p .cache
  tmp=$(mktemp -d .cache/english-self-test.XXXXXX) || exit 2
  printf 'hello \u4e2d\u6587\n' > "$tmp/plain.md"
  printf '\u4e2d\u6587 <!-- c49:trigger -->\n' > "$tmp/trigger.md"
  printf '\u4e2d\u6587\n' > "$tmp/allow.md"
  ALLOW["$tmp/allow.md"]=1
  n_plain=$(count_cjk "$tmp/plain.md")
  n_trigger=$(count_cjk "$tmp/trigger.md")
  n_allow=$(count_cjk "$tmp/allow.md")
  rm -rf "$tmp"
  if [ "$n_plain" -eq 1 ] && [ "$n_trigger" -eq 0 ] && [ "$n_allow" -eq 0 ]; then
    echo "CHECK-ENGLISH self-test ✓ (plain red, trigger exempt, allowlist exempt)"
    exit 0
  fi
  echo "CHECK-ENGLISH self-test ✗ plain=$n_plain trigger=$n_trigger allow=$n_allow"
  exit 1
fi

if [ "$SUMMARY" -eq 1 ]; then
  HITS=0
  for f in "${FILES[@]}"; do
    n=$(count_cjk "$f")
    [ "$n" -gt 0 ] && HITS=$((HITS+1))
  done
  echo "files_with_cjk: $HITS / ${#FILES[@]} scanned"
  if [ "$HITS" -gt 0 ]; then exit 1; else exit 0; fi
fi

OFFENDERS=""
for f in "${FILES[@]}"; do
  n=$(count_cjk "$f")
  [ "$n" -gt 0 ] && OFFENDERS+="$f:$n"$'\n'
done
OFFENDERS=$(printf '%s' "$OFFENDERS" | sort -t: -k2 -rn)
if [ -z "$OFFENDERS" ]; then
  echo "check-english-only: PASS (0 CJK lines in ${#FILES[@]} tracked artifacts)"
  exit 0
fi
echo "check-english-only: FAIL — CJK found in tracked artifacts:"
echo "$OFFENDERS"
exit 1
