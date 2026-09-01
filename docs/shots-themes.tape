# The README's theme screenshots, taken by vhs (https://github.com/charmbracelet/vhs):
#
#   cargo build --release && vhs docs/shots-themes.tape
#
# One picture per theme, all of the same window showing the same file, because a set of theme
# shots is a comparison and a comparison needs everything except the colours to hold still.
#
# CleeCode is restarted once per theme, with a config directory that names it, rather than the
# theme being chosen from the drop-down with arrow keys. Driving a menu from a tape means counting
# rows, and a tape that counts rows photographs the wrong thing the day a row is inserted above
# the one it meant — quietly, because the shot still succeeds. The directories are made by
# scripts/theme_shot_configs.sh, which is where the quoting a TOML string needs can live: a Type
# line here cannot hold nested quotes.
#
# The real config is never touched, and the author's own theme cannot leak into the pictures.

Output docs/.shots-themes-scratch.gif

Set Shell "bash"
Set FontFamily "JetBrainsMono NFM"
Set FontSize 16
Set Width 1400
Set Height 800
Set Padding 12

Hide
Type "bash scripts/theme_shot_configs.sh /tmp/clee-theme-shots && clear"
Enter
Sleep 2s
Show

# ---- CleeCode: the default, and what every other picture is a variation on.
Hide
Type "export XDG_CONFIG_HOME=/tmp/clee-theme-shots/cleecode && clear"
Enter
Sleep 2s
Show
Type "./target/release/clee examples/hello.py"
Enter
Sleep 9s
Screenshot docs/screenshots/theme-cleecode.png
Hide
Ctrl+q
Sleep 3s
Show

# ---- Turbo: the blue screen. A light bar over a dark field, and the initials in red.
Hide
Type "export XDG_CONFIG_HOME=/tmp/clee-theme-shots/turbo && clear"
Enter
Sleep 2s
Show
Type "./target/release/clee examples/hello.py"
Enter
Sleep 9s
Screenshot docs/screenshots/theme-turbo.png
Hide
Ctrl+q
Sleep 3s
Show

# ---- Solarized Dark.
Hide
Type "export XDG_CONFIG_HOME=/tmp/clee-theme-shots/solarized-dark && clear"
Enter
Sleep 2s
Show
Type "./target/release/clee examples/hello.py"
Enter
Sleep 9s
Screenshot docs/screenshots/theme-solarized-dark.png
Hide
Ctrl+q
Sleep 3s
Show

# ---- Eighties.
Hide
Type "export XDG_CONFIG_HOME=/tmp/clee-theme-shots/eighties && clear"
Enter
Sleep 2s
Show
Type "./target/release/clee examples/hello.py"
Enter
Sleep 9s
Screenshot docs/screenshots/theme-eighties.png
Hide
Ctrl+q
Sleep 3s
Show

# ---- Mocha: the warm one, browns where the others have greys.
Hide
Type "export XDG_CONFIG_HOME=/tmp/clee-theme-shots/mocha && clear"
Enter
Sleep 2s
Show
Type "./target/release/clee examples/hello.py"
Enter
Sleep 9s
Screenshot docs/screenshots/theme-mocha.png
Hide
Ctrl+q
Sleep 3s
Show

# ---- CleeCode Light: the first of the four that paint their own surface.
Hide
Type "export XDG_CONFIG_HOME=/tmp/clee-theme-shots/cleecode-light && clear"
Enter
Sleep 2s
Show
Type "./target/release/clee examples/hello.py"
Enter
Sleep 9s
Screenshot docs/screenshots/theme-cleecode-light.png
Hide
Ctrl+q
Sleep 3s
Show

# ---- Solarized Light.
Hide
Type "export XDG_CONFIG_HOME=/tmp/clee-theme-shots/solarized-light && clear"
Enter
Sleep 2s
Show
Type "./target/release/clee examples/hello.py"
Enter
Sleep 9s
Screenshot docs/screenshots/theme-solarized-light.png
Hide
Ctrl+q
Sleep 3s
Show

# ---- Ocean Light.
Hide
Type "export XDG_CONFIG_HOME=/tmp/clee-theme-shots/ocean-light && clear"
Enter
Sleep 2s
Show
Type "./target/release/clee examples/hello.py"
Enter
Sleep 9s
Screenshot docs/screenshots/theme-ocean-light.png
Hide
Ctrl+q
Sleep 3s
Show

# ---- GitHub, the lightest of them.
Hide
Type "export XDG_CONFIG_HOME=/tmp/clee-theme-shots/github && clear"
Enter
Sleep 2s
Show
Type "./target/release/clee examples/hello.py"
Enter
Sleep 9s
Screenshot docs/screenshots/theme-github.png
Hide
Ctrl+q
Sleep 3s
Show
