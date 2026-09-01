#!/usr/bin/env bash
# One throwaway config directory per theme, for docs/shots-themes.tape.
#
# The tape restarts CleeCode once per theme rather than walking the drop-down with arrow keys: a
# tape that counts rows photographs the wrong thing the day a row is inserted above the one it
# meant, and does it quietly, because the shot still succeeds. Here the theme is named, and the
# name is the same key the drop-down writes — `the_settings_keys_are_what_they_have_always_been`
# fails loudly if one of them ever stops being real.
#
# It lives in a script rather than in the tape because the tape's Type lines cannot hold nested
# quotes, and a TOML string needs them.
#
#   bash scripts/theme_shot_configs.sh [directory]
#
# Defaults to a directory under $TMPDIR, prints it, and never touches the real config.
set -euo pipefail

root="${1:-${TMPDIR:-/tmp}/clee-theme-shots}"
rm -rf "$root"

for theme in cleecode turbo solarized-dark eighties mocha \
             cleecode-light solarized-light ocean-light github; do
    mkdir -p "$root/$theme/cleecode"
    # The theme, and the solid background. A theme with light text is free to be left
    # translucent, which is right for using it and wrong for photographing it: through it the
    # picture would show the recorder's black rather than Turbo's blue or Solarized's slate.
    {
        printf 'theme = "%s"\n' "$theme"
        printf 'opaque_background = true\n'
    } > "$root/$theme/cleecode/settings.toml"
done

echo "$root"
